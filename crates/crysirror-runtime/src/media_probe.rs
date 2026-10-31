// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Scoped metadata/decoder preparation. No Media slot, playback thread, Layer,
//! clock, decoded picture publication or PCM output is created.
use crate::image_processing::Charge;
use anyhow::{ensure, Context};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{
        AssetReader, ClientStorage, MediaProbe, MediaProbeInfo, MediaProbeJob, MediaProbeState,
    },
    state::ClientId,
    Error,
};
use crysirror_tasks::{Cancellation, Task, TaskPool};
use crysirror_video::{amv, asf, isobmff, mpeg, theora};
use std::{
    cell::Cell,
    io::{Read, Seek, SeekFrom},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
const MAX_HEADER_BYTES: usize = 64 * 1024 * 1024;
struct Reader {
    source: Arc<dyn AssetReader>,
    len: u64,
    left: AtomicUsize,
    cancel: Cancellation,
}
impl AssetReader for Reader {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.len)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> crysirror_core::Result<usize> {
        self.cancel.check().map_err(core)?;
        if offset >= self.len || output.is_empty() {
            return Ok(0);
        }
        let count = output
            .len()
            .min((self.len - offset).min(usize::MAX as u64) as usize);
        self.left
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(count)
            })
            .map_err(|_| Error::Limit("media probe header reads"))?;
        let n = self.source.read_at(offset, &mut output[..count])?;
        if n > count {
            return Err(Error::Invalid("media probe read length"));
        }
        self.left.fetch_add(count - n, Ordering::AcqRel);
        self.cancel.check().map_err(core)?;
        Ok(n)
    }
}
fn core(error: impl std::fmt::Display) -> Error {
    Error::Io(error.to_string())
}
struct Job {
    owner: ClientId,
    task: Option<Task<MediaProbeInfo>>,
    state: MediaProbeState,
}
impl MediaProbe for Job {
    fn client(&self) -> ClientId {
        self.owner
    }
    fn poll(&mut self) -> crysirror_core::Result<MediaProbeState> {
        if let Some(task) = &mut self.task {
            match task.try_take() {
                Ok(None) => (),
                Ok(Some(info)) => {
                    self.state = MediaProbeState::Ready(info);
                    self.task = None;
                }
                Err(error) => {
                    self.state = MediaProbeState::Failed(format!("{error:#}"));
                    self.task = None;
                }
            }
        }
        Ok(self.state.clone())
    }
}
pub(crate) fn begin(
    owner: ClientId,
    asset: VfsPath,
    storage: Arc<dyn ClientStorage>,
    pool: TaskPool,
    memory: Arc<AtomicUsize>,
) -> crysirror_core::Result<MediaProbeJob> {
    if asset.mount() != Mount::Asset {
        return Err(Error::ReadOnly);
    }
    let permit = pool.reserve().map_err(core)?;
    // Header/index scratch and existing decoder-constructor working storage
    // share the image resource budget. The reservation is released on Ready.
    let scratch = Charge::new(memory, MAX_HEADER_BYTES + 256 * 1024 * 1024)?;
    let task = permit
        .spawn(move |cancel| {
            let _scratch = scratch;
            cancel.check()?;
            let source = storage.open_asset(&asset)?;
            let len = source.len()?;
            ensure!(
                (8..=crysirror_video::MAX_VIDEO_BYTES as u64).contains(&len),
                "media probe input length"
            );
            let reader: Arc<dyn AssetReader> = Arc::new(Reader {
                source,
                len,
                left: AtomicUsize::new(MAX_HEADER_BYTES),
                cancel: cancel.clone(),
            });
            let mut input = super::AssetCursor::new(reader)?;
            let mut head = [0; 8];
            input.read_exact(&mut head)?;
            input.seek(SeekFrom::Start(0))?;
            let mut result = MediaProbeInfo {
                client: owner,
                size: [0, 0],
                container: String::new(),
                video_decoder: String::new(),
                header_validated: false,
                audio_decoder: None,
                has_audio: None,
                duration_ms: None,
                frame_ms: None,
            };
            if isobmff::is_mp4(&head) {
                let decoder = isobmff::Mp4Stream::new(input)?;
                result.size = [decoder.info().width, decoder.info().height];
                result.container = "mp4".into();
                result.video_decoder = decoder.codec().into();
                result.has_audio = Some(decoder.has_audio());
                result.frame_ms = decoder.info().fps.map(|fps| 1000. / fps);
            } else if asf::is_asf(&mut input)? {
                let done = Cell::new(false);
                let mut metadata = None;
                asf::decode_video_with_metadata(
                    input,
                    || cancel.is_cancelled() || done.get(),
                    |info| {
                        metadata = Some(info);
                        done.set(true);
                        Ok(())
                    },
                    |_| anyhow::bail!("media probe decoded an unexpected picture"),
                )?;
                let info = metadata.context("ASF decoder metadata missing")?;
                result.size = info.size;
                result.container = "asf".into();
                result.video_decoder = "wmv/vc1".into();
                result.duration_ms = info.duration_ms;
                result.frame_ms = info.frame_ms;
            } else if theora::is_input(&mut input)? {
                let done = Cell::new(false);
                let mut metadata = None;
                theora::decode_video_with_metadata(
                    input,
                    || cancel.is_cancelled() || done.get(),
                    |info| {
                        metadata = Some(info);
                        done.set(true);
                        Ok(())
                    },
                    |_| anyhow::bail!("media probe decoded an unexpected picture"),
                )?;
                let info = metadata.context("Theora decoder metadata missing")?;
                result.size = info.size;
                result.container = if &head[..4] == b"OggS" { "ogg" } else { "omv" }.into();
                result.video_decoder = "theora".into();
                result.duration_ms = info.duration_ms;
                result.frame_ms = info.frame_ms;
            } else if amv::is_input(&mut input)? {
                // The AMV path has no header-only decoder-constructor hook yet.
                // Do not promote a signature into verified codec readiness.
                anyhow::bail!("AlphaMovie header-only decoder preparation is not implemented");
            } else {
                let info = mpeg::sequence_info(&mut input)?
                    .context("unrecognized video or MPEG sequence header")?;
                // Same validated sequence/size/rate used to prepare the shared MPEG
                // pipeline. This is header readiness, not a complete-file decode.
                result.size = [info.width, info.height];
                result.frame_ms = Some(info.frame_ms);
                result.container = "mpeg".into();
                result.video_decoder = "mpeg1/mpeg2".into();
            }
            ensure!(
                result.size.iter().all(|v| *v > 0 && *v <= 8192)
                    && u64::from(result.size[0]) * u64::from(result.size[1]) <= 16 * 1024 * 1024,
                "media probe dimensions"
            );
            if let Some(ms) = result.frame_ms {
                ensure!(
                    ms.is_finite() && ms > 0. && ms <= 10000.,
                    "media probe frame duration"
                );
            }
            cancel.check()?;
            result.header_validated = true;
            Ok(result)
        })
        .map_err(core)?;
    Ok(Box::new(Job {
        owner,
        task: Some(task),
        state: MediaProbeState::Pending,
    }))
}
