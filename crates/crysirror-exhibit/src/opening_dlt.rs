// SPDX-License-Identifier: MPL-2.0
//! The witnessed v1 opening DLT on native channel 0, layer 4.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    image_processing::RgbaImage,
    path::{Mount, VfsPath},
    service::HostServices,
    state::ClientId,
};
use crysirror_resource::exhibit_dlt::DltDecoder;
use crysirror_tasks::{Task, TaskPool};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) const SLOT: u8 = 4;
pub(super) const ID: i32 = 1239;
const LAYER: u32 = 256 + SLOT as u32;
const FADE_MS: u64 = 800;

struct Fade {
    start: u64,
    showing: bool,
}

type Decoded = (DltDecoder, Arc<RgbaImage>);

pub(super) struct OpeningDlt {
    opening: Option<Task<DltDecoder>>,
    work: Option<Task<Decoded>>,
    decoder: Option<DltDecoder>,
    path: Option<String>,
    frame: Option<u64>,
    fade: Option<Fade>,
    level: u8,
    playing: bool,
    retired: bool,
}

fn layer(paint: Value, level: u8) -> Value {
    json!({"op":"put","layer":{"id":LAYER,"order":[2,SLOT],"rect":[0,0,800,600],
        "paint":paint,"opacity":level.saturating_mul(16)}})
}

fn send(h: &mut dyn HostServices, c: ClientId, op: Value) -> Result<()> {
    h.submit_engine(c, 1, &serde_json::to_vec(&json!({"layers":[op]}))?)?;
    Ok(())
}

fn native_steps(last: Option<u64>, now: u64, interval: u32) -> usize {
    match last.map(|t| now.saturating_sub(t)) {
        None | Some(2001..) => 1,
        Some(elapsed) => (elapsed / u64::from(interval)) as usize,
    }
}

fn fade_level(age: u64, showing: bool) -> u8 {
    let age = age.min(FADE_MS);
    (16 * if showing { age } else { FADE_MS - age } / FADE_MS) as u8
}

impl OpeningDlt {
    pub fn prepare(h: &mut dyn HostServices, c: ClientId, pool: &TaskPool) -> Result<Self> {
        let storage = h.storage(c)?;
        let opening = pool.spawn(move |cancel| {
            cancel.check()?;
            let asset = VfsPath::new(Mount::Asset, "res/g/ev/12/1239.dlt")?;
            let reader = storage.open_read(&asset)?;
            let bytes = crysirror_resource::read_all(reader.as_ref(), 32 << 20)?;
            cancel.check()?;
            ensure!(
                bytes.starts_with(&0x0101u32.to_le_bytes()),
                "unsupported v1 opening DLT version"
            );
            let decoder = DltDecoder::open(bytes.into())?;
            let info = decoder.info();
            ensure!(
                info.size == [800, 600]
                    && info.interval_ms == 80
                    && info.frames > 1
                    && info.loop_hint,
                "unsupported v1 opening DLT layout"
            );
            Ok(decoder)
        })?;
        Ok(Self {
            opening: Some(opening),
            work: None,
            decoder: None,
            path: None,
            frame: None,
            fade: None,
            level: 0,
            playing: false,
            retired: false,
        })
    }

    pub fn start(&mut self, h: &mut dyn HostServices, c: ClientId, pool: &TaskPool) -> Result<()> {
        ensure!(
            self.opening.is_none() && self.decoder.is_some() && !self.playing,
            "invalid ExHibit opening DLT start"
        );
        self.playing = true;
        self.fade = Some(Fade {
            start: h.now_ms(),
            showing: true,
        });
        self.request_frame(h.now_ms(), c, pool)
    }

    pub fn stop(&mut self, now: u64) -> Result<()> {
        ensure!(
            self.playing && self.fade.is_none() && self.level == 16,
            "invalid ExHibit opening DLT stop"
        );
        self.fade = Some(Fade {
            start: now,
            showing: false,
        });
        Ok(())
    }

    pub fn retired(&self) -> bool {
        self.retired
    }

    fn request_frame(&mut self, now: u64, c: ClientId, pool: &TaskPool) -> Result<()> {
        if self.work.is_some() {
            return Ok(());
        }
        let Some(decoder) = &self.decoder else {
            return Ok(());
        };
        let count = native_steps(self.frame, now, decoder.info().interval_ms);
        if count == 0 {
            return Ok(());
        }
        let mut decoder = self.decoder.take().context("missing ExHibit DLT decoder")?;
        self.work = Some(pool.spawn(move |cancel| {
            let mut latest = None;
            for _ in 0..count {
                cancel.check()?;
                latest = Some(
                    decoder
                        .next_looped_frame()?
                        .context("unexpected end of opening DLT")?,
                );
            }
            let latest = latest.context("empty ExHibit DLT frame request")?;
            let image = Arc::new(RgbaImage::new(c, decoder.info().size, latest.rgba.into())?);
            Ok((decoder, image))
        })?);
        self.frame = Some(now);
        Ok(())
    }

    pub fn present(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        pool: &TaskPool,
    ) -> Result<bool> {
        if let Some(task) = &mut self.opening {
            let Some(decoder) = task.try_take()? else {
                return Ok(false);
            };
            send(h, c, layer(json!({"kind":"solid","color":[0,0,0,0]}), 0))?;
            self.decoder = Some(decoder);
            self.opening = None;
        }
        let now = h.now_ms();
        if let Some(fade) = &self.fade {
            let level = fade_level(now.saturating_sub(fade.start), fade.showing);
            if level == 0 && !fade.showing {
                send(h, c, json!({"op":"remove","id":LAYER}))?;
                if let Some(path) = self.path.take() {
                    h.release_image_resource(c, &path)?;
                }
                self.work = None;
                self.decoder = None;
                self.fade = None;
                self.playing = false;
                self.retired = true;
                return Ok(true);
            }
            if self.level != level {
                send(
                    h,
                    c,
                    json!({"op":"set_opacity","id":LAYER,"opacity":level.saturating_mul(16)}),
                )?;
                self.level = level;
            }
            if fade.showing && level == 16 && self.path.is_some() {
                self.fade = None;
            }
        }
        if let Some(work) = &mut self.work {
            if let Some((decoder, image)) = work.try_take()? {
                let uploaded = h.publish_image_resource(c, LAYER, image)?;
                let path = uploaded.path.relative().to_owned();
                if let Err(error) = send(
                    h,
                    c,
                    layer(json!({"kind":"image","asset":path}), self.level),
                ) {
                    let _ = h.release_image_resource(c, &path);
                    return Err(error);
                }
                if let Some(old) = self.path.replace(path) {
                    h.release_image_resource(c, &old)?;
                }
                self.decoder = Some(decoder);
                self.work = None;
            }
        }
        if self.playing {
            self.request_frame(now, c, pool)?;
        }
        Ok(self.fade.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crysirror_core::{
        Error, Result as CoreResult,
        image_processing::ImageResource,
        service::{FileSystem, UploadedImage},
        state::Command,
    };
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Host {
        clock: u64,
        submissions: Vec<Value>,
        frames: usize,
        released: Vec<String>,
    }

    impl FileSystem for Host {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> CoreResult<Vec<u8>> {
            Err(Error::NotFound)
        }
        fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Host {
        fn submit(&mut self, _: ClientId, _: &[Command]) -> CoreResult<()> {
            Ok(())
        }
        fn now_ms(&self) -> u64 {
            self.clock
        }
        fn submit_engine(&mut self, _: ClientId, _: u32, payload: &[u8]) -> CoreResult<()> {
            self.submissions
                .push(serde_json::from_slice(payload).map_err(|e| Error::Io(e.to_string()))?);
            Ok(())
        }
        fn publish_image_resource(
            &mut self,
            _: ClientId,
            _: u32,
            image: Arc<dyn ImageResource>,
        ) -> CoreResult<UploadedImage> {
            self.frames += 1;
            Ok(UploadedImage {
                path: VfsPath::new(Mount::Asset, &format!("fixture/dlt-{}", self.frames))?,
                size: image.size(),
                opaque: image.opaque(),
            })
        }
        fn release_image_resource(&mut self, _: ClientId, path: &str) -> CoreResult<()> {
            self.released.push(path.to_owned());
            Ok(())
        }
    }

    fn minimal_stream() -> Result<DltDecoder> {
        let mut raw = Vec::new();
        for x in [0x0101u32, 1, 1, 3, 0x0801, 80, 0, 0, 0x0e02, 1024] {
            raw.extend(x.to_le_bytes());
        }
        raw.resize(raw.len() + 1024, 0);
        for x in [0x0f02u32, 4, 1, 0x0200, 2, 0, 0, 1, 1] {
            raw.extend(x.to_le_bytes());
        }
        raw.extend([1, 2]);
        raw.extend(0x8000u32.to_le_bytes());
        DltDecoder::open(raw.into())
    }

    #[test]
    fn async_clip_publishes_replaces_and_retires_owned_images_without_a_game() -> Result<()> {
        let worker = TaskPool::new("opening-dlt-fixture", 1, 1)?;
        let mut clip = OpeningDlt {
            opening: None,
            work: None,
            decoder: Some(minimal_stream()?),
            path: None,
            frame: None,
            fade: None,
            level: 0,
            playing: false,
            retired: false,
        };
        let mut host = Host::default();
        let client = ClientId(1);
        clip.start(&mut host, client, &worker)?;
        let until = Instant::now() + Duration::from_secs(10);
        while host.frames == 0 {
            assert!(!clip.present(&mut host, client, &worker)?);
            ensure!(Instant::now() < until, "fixture frame was not produced");
            std::thread::yield_now();
        }
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0]["layer"]["opacity"],
            0
        );
        host.clock = 400;
        assert!(!clip.present(&mut host, client, &worker)?);
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0]["opacity"],
            128
        );
        while host.frames < 2 {
            assert!(!clip.present(&mut host, client, &worker)?);
            ensure!(
                Instant::now() < until,
                "fixture catchup frame was not produced"
            );
            std::thread::yield_now();
        }
        assert_eq!(host.released, ["fixture/dlt-1"]);
        host.clock = 800;
        assert!(clip.present(&mut host, client, &worker)?);
        clip.stop(800)?;
        host.clock = 1551;
        assert!(clip.present(&mut host, client, &worker)?);
        assert!(clip.retired());
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0],
            json!({"op":"remove","id":260})
        );
        assert_eq!(host.released, ["fixture/dlt-1", "fixture/dlt-2"]);
        Ok(())
    }

    #[test]
    fn clock_updates_with_native_catchup_and_independent_fade() {
        assert_eq!(native_steps(None, 0, 80), 1);
        assert_eq!(native_steps(Some(16), 95, 80), 0);
        assert_eq!(native_steps(Some(16), 96, 80), 1);
        assert_eq!(native_steps(Some(16), 176, 80), 2);
        assert_eq!(native_steps(Some(16), 2016, 80), 25);
        assert_eq!(native_steps(Some(16), 2017, 80), 1);
        assert_eq!(fade_level(400, true), 8);
        assert_eq!(fade_level(750, false), 1);
        assert_eq!(fade_level(751, false), 0);
    }
}
