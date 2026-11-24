//! Source bytes stay behind a retained, budgeted engine resource lease. The
//! source-layout path preserves RIFF PCM8/16 mono/stereo. Visualization reads
//! use an independent engine decoder and return signed stereo PCM.
use super::*;
use crysirror_core::{
    service::{
        AudioSampleFormat, AudioSampleQuery, AudioSampleReceipt, AudioSampleRequest,
        AudioSampleState,
    },
    state::ClientId,
    Error,
};
struct Query {
    task: crysirror_tasks::Task<AudioSampleReceipt>,
    done: bool,
    failure: Option<String>,
}
impl Drop for Query {
    fn drop(&mut self) {
        self.task.cancel();
    }
}
impl AudioSampleQuery for Query {
    fn poll(&mut self) -> crysirror_core::Result<AudioSampleState> {
        if let Some(error) = &self.failure {
            return Err(Error::Io(error.clone()));
        }
        if self.done {
            return Err(Error::Invalid("audio sample query consumed"));
        }
        let result = match self.task.try_take() {
            Ok(value) => value,
            Err(error) => {
                let message = alloc_error(error);
                self.failure = Some(message.clone());
                return Err(Error::Io(message));
            }
        };
        match result {
            None => Ok(AudioSampleState::Pending),
            Some(value) => {
                self.done = true;
                Ok(AudioSampleState::Ready(value))
            }
        }
    }
}
fn alloc_error(e: anyhow::Error) -> String {
    format!("audio source sample worker: {e:#}")
}
impl AudioResources {
    pub fn begin_samples(
        &self,
        client: u32,
        resource: AudioResourceHandle,
        request: AudioSampleRequest,
        pool: &TaskPool,
    ) -> Result<Box<dyn AudioSampleQuery>> {
        self.sample_query(client, resource, request, pool, |sound, request, cancel| {
            let bytes = &sound.snapshot.bytes;
            let (format, start, len) = pcm_layout(bytes)?;
            cancel.check()?;
            let align = u64::from(format.channels) * u64::from(format.bits_per_sample / 8);
            let available = (len as u64 / align).min(sound.frame_limit.unwrap_or(u64::MAX));
            ensure!(
                request.start_frame <= available,
                "audio sample position beyond source"
            );
            let frames =
                (u64::from(request.max_bytes) / align).min(available - request.start_frame);
            let offset = start + (request.start_frame * align) as usize;
            let size = (frames * align) as usize;
            Ok((format, frames as u32, bytes[offset..offset + size].to_vec()))
        })
    }

    pub fn begin_visual_samples(
        &self,
        client: u32,
        resource: AudioResourceHandle,
        request: AudioSampleRequest,
        pool: &TaskPool,
    ) -> Result<Box<dyn AudioSampleQuery>> {
        let decoder_budget = self.budget.clone();
        self.sample_query(
            client,
            resource,
            request,
            pool,
            move |sound, request, cancel| {
                let _scratch = Charge::new(decoder_budget, SCRATCH_BYTES)?;
                let mut source = sound.open()?;
                read_visual_window(&mut *source, request, &|| cancel.check())
            },
        )
    }

    fn sample_query(
        &self,
        client: u32,
        resource: AudioResourceHandle,
        request: AudioSampleRequest,
        pool: &TaskPool,
        read: impl FnOnce(
                &Arc<PreparedSound>,
                AudioSampleRequest,
                &Cancellation,
            ) -> Result<(AudioSampleFormat, u32, Vec<u8>)>
            + Send
            + 'static,
    ) -> Result<Box<dyn AudioSampleQuery>> {
        ensure!(
            request.max_bytes <= 1024 * 1024,
            "audio source sample read budget"
        );
        let sound = self.capture(client, resource)?;
        let mut charge = Charge::new(self.budget.clone(), request.max_bytes as usize * 2 + 8192)?;
        let task = pool.spawn(move |cancel| {
            cancel.check()?;
            let (format, frames, bytes) = read(&sound, request, &cancel)?;
            let size = bytes.len();
            let mut result = AudioSampleReceipt::new(
                ClientId(client),
                resource,
                format,
                request.start_frame,
                frames,
                bytes,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            // The copied output and its clones retain the reservation until
            // their last consumer drops; worker completion is not its lifetime.
            charge.shrink(size + 8192);
            result.retain_storage(Arc::new(charge));
            cancel.check()?;
            Ok(result)
        })?;
        Ok(Box::new(Query {
            task,
            done: false,
            failure: None,
        }))
    }
}

fn read_visual_window(
    source: &mut dyn AudioSource,
    request: AudioSampleRequest,
    check: &dyn Fn() -> Result<()>,
) -> Result<(AudioSampleFormat, u32, Vec<u8>)> {
    let format = AudioSampleFormat {
        sample_rate: source.sample_rate(),
        channels: 2,
        bits_per_sample: 16,
    };
    let budget = request.max_bytes as usize / 4;
    let mut bytes = Vec::with_capacity(budget * 4);
    if budget != 0
        && source
            .frame_count()
            .is_none_or(|end| request.start_frame < end)
    {
        if request.start_frame != 0 {
            source.seek(request.start_frame)?;
        }
        let mut block = [[0.0; 2]; 256];
        while bytes.len() / 4 < budget {
            check()?;
            let count = block.len().min(budget - bytes.len() / 4);
            let n = source.read(&mut block[..count])?;
            ensure!(n <= count, "decoder exceeded visualization window");
            if n == 0 {
                break;
            }
            for sample in block[..n].iter().flatten() {
                bytes.extend_from_slice(&((*sample * 32768.0).round() as i16).to_le_bytes());
            }
        }
    }
    Ok((format, (bytes.len() / 4) as u32, bytes))
}
fn pcm_layout(bytes: &[u8]) -> Result<(AudioSampleFormat, usize, usize)> {
    ensure!(
        bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
        "source-layout samples unsupported for this encoded codec"
    );
    let end = u32::from_le_bytes(bytes[4..8].try_into()?) as usize + 8;
    ensure!(end <= bytes.len() && end >= 12, "source PCM RIFF extent");
    let mut at = 12;
    let mut format = None;
    let mut data = None;
    let mut count = 0;
    while at + 8 <= end {
        count += 1;
        ensure!(count <= 4096, "source PCM chunk count");
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into()?) as usize;
        let start = at + 8;
        let finish = start
            .checked_add(size)
            .context("source PCM chunk overflow")?;
        ensure!(finish <= end, "source PCM truncated chunk");
        match &bytes[at..at + 4] {
            b"fmt " => {
                ensure!(format.is_none() && size >= 16, "source PCM format chunk");
                let u16at = |n| u16::from_le_bytes([bytes[start + n], bytes[start + n + 1]]);
                let channels = u16at(2);
                let bits = u16at(14);
                let rate = u32::from_le_bytes(bytes[start + 4..start + 8].try_into()?);
                ensure!(
                    u16at(0) == 1
                        && matches!(channels, 1 | 2)
                        && matches!(bits, 8 | 16)
                        && (8000..=192000).contains(&rate),
                    "source PCM samples require integer8/16 mono/stereo"
                );
                ensure!(u16at(12) == channels * (bits / 8), "source PCM alignment");
                format = Some(AudioSampleFormat {
                    sample_rate: rate,
                    channels,
                    bits_per_sample: bits,
                });
            }
            b"data" => {
                ensure!(data.is_none(), "source PCM multiple data chunks");
                data = Some((start, size));
            }
            _ => {}
        }
        at = finish
            .checked_add(size & 1)
            .context("source PCM chunk pad overflow")?;
    }
    let format = format.context("source PCM format absent")?;
    let (start, len) = data.context("source PCM samples absent")?;
    ensure!(
        len % (usize::from(format.channels) * usize::from(format.bits_per_sample / 8)) == 0,
        "source PCM partial frame"
    );
    Ok((format, start, len))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_prepared_pcm_rewinds_shares_storage_and_keeps_independent_cursors() {
        let budget = Arc::new(AtomicUsize::new(0));
        let snapshot = Snapshot { bytes: vec![], _charge: Charge::new(budget.clone(), 0).unwrap() };
        let frames = vec![[0.25, -0.25], [0.5, -0.5], [0., 0.]];
        let mut measured = decoder::open_pcm(crysirror_core::service::PcmAudio::new(8000, frames.clone()).unwrap());
        measured.seek(3).unwrap();
        let pcm = prepare_short_pcm(&mut *measured, Some(3), &snapshot, &Cancellation::default()).unwrap().unwrap();
        let mut a = decoder::open_pcm(pcm.clone());
        let mut b = decoder::open_pcm(pcm);
        a.seek(1).unwrap();
        let mut sample = [[0.;2]];
        a.read(&mut sample).unwrap();
        assert_eq!(sample[0], frames[1]);
        b.read(&mut sample).unwrap();
        assert_eq!(sample[0], frames[0]);
        assert_eq!(budget.load(Ordering::Acquire), 24);
        drop((a,b));
        assert_eq!(budget.load(Ordering::Acquire), 0);
    }

    #[test]
    fn visualization_reads_real_ogg_samples_at_the_requested_cursor() {
        let encoded: Arc<[u8]> = include_bytes!("../../tests/fixtures/tone.ogg")
            .as_slice()
            .into();
        let mut reference = decoder::Stream::from_bytes(encoded.clone(), Some("ogg")).unwrap();
        let mut decoded = [[0.0; 2]; 96];
        assert_eq!(reference.read(&mut decoded).unwrap(), 96);
        let expected: Vec<_> = decoded[32..]
            .iter()
            .flatten()
            .flat_map(|s| ((*s * 32768.0).round() as i16).to_le_bytes())
            .collect();
        assert!(expected.iter().any(|b| *b != 0));
        let mut source = decoder::Stream::from_bytes(encoded, Some("ogg")).unwrap();
        let (fmt, frames, bytes) = read_visual_window(
            &mut source,
            AudioSampleRequest {
                start_frame: 32,
                max_bytes: 64 * 4 + 3,
            },
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!((fmt.channels, fmt.bits_per_sample, frames), (2, 16, 64));
        assert_eq!(bytes, expected);
        let end = source.frame_count().unwrap();
        assert_eq!(
            read_visual_window(
                &mut source,
                AudioSampleRequest {
                    start_frame: end,
                    max_bytes: 32
                },
                &|| Ok(())
            )
            .unwrap()
            .1,
            0
        );
    }
    #[test]
    fn source_layout_preserves_depth_and_channels() {
        for (channels, bits) in [(1, 8), (2, 16)] {
            let spec = crysirror_resource::WaveSpec {
                channels,
                bits_per_sample: bits,
                ..Default::default()
            };
            let bytes = spec.encode(&[0, 128, 255, 127]).unwrap();
            let (format, offset, length) = pcm_layout(&bytes).unwrap();
            assert_eq!((format.channels, format.bits_per_sample), (channels, bits));
            assert_eq!(&bytes[offset..offset + length], &[0, 128, 255, 127]);
        }
        assert!(pcm_layout(b"OggS").is_err());
    }
}
#[cfg(test)]
mod lifetime_tests {
    use super::*;
    fn ready(query: &mut dyn AudioSampleQuery) -> AudioSampleReceipt {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match query.poll().unwrap() {
                AudioSampleState::Ready(value) => return value,
                AudioSampleState::Pending => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                }
            }
        }
    }
    #[test]
    fn sample_receipt_retains_budget_across_clones_and_failures_are_stable() {
        let mut resources = AudioResources::default();
        let handle = AudioResourceHandle { client: 7, id: 1 };
        let bytes = crysirror_resource::WaveSpec {
            channels: 1,
            ..Default::default()
        }
        .encode(&[0, 128, 255, 127])
        .unwrap();
        let count = bytes.len();
        let snapshot = Arc::new(Snapshot {
            bytes,
            _charge: Charge::new(resources.budget.clone(), count).unwrap(),
        });
        resources.resources.insert(
            handle,
            Arc::new(PreparedSound {
                snapshot,
                short_pcm: None,
                extension: Some("wav".into()),
                frame_limit: None,
                info: AudioResourceInfo {
                    source_rate: 44100,
                    source_frames: Some(2),
                    encoded_bytes: count as u64,
                    has_replay_gain: false,
                },
                stems: None,
            }),
        );
        let pool = TaskPool::new("source-pcm-test", 1, 4).unwrap();
        assert!(resources
            .begin_samples(
                8,
                handle,
                AudioSampleRequest {
                    start_frame: 0,
                    max_bytes: 4
                },
                &pool
            )
            .is_err());
        let mut query = resources
            .begin_samples(
                7,
                handle,
                AudioSampleRequest {
                    start_frame: 0,
                    max_bytes: 4,
                },
                &pool,
            )
            .unwrap();
        let receipt = ready(query.as_mut());
        assert_eq!(receipt.bytes.as_ref(), &[0, 128, 255, 127]);
        assert_eq!(receipt.frames, 2);
        let copy = receipt.clone();
        drop(query);
        drop(receipt);
        assert_eq!(resources.resident_bytes(), count + 8192 + 4);
        drop(copy);
        assert_eq!(resources.resident_bytes(), count);
        let mut query = resources
            .begin_samples(
                7,
                handle,
                AudioSampleRequest {
                    start_frame: 99,
                    max_bytes: 4,
                },
                &pool,
            )
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let first = loop {
            match query.poll() {
                Err(error) => break error.to_string(),
                Ok(AudioSampleState::Pending) => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now()
                }
                Ok(_) => panic!("unexpected PCM success"),
            }
        };
        assert_eq!(query.poll().err().unwrap().to_string(), first);
        assert_eq!(resources.resident_bytes(), count);
    }
}
