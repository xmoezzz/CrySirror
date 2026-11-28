// SPDX-License-Identifier: MPL-2.0
//! Numbered v1 DLT clips on native channel zero.
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

#[derive(Debug, PartialEq, Clone, Copy)]
pub(super) struct Placement {
    pub id: i32,
    pub slot: u8,
    pub origin: [i32; 2],
}

struct Fade {
    start: u64,
    duration: u32,
    from: u8,
    to: u8,
    retiring: bool,
}

impl Fade {
    fn level(&self, now: u64) -> u8 {
        let duration = u64::from(self.duration);
        if duration == 0 {
            return self.to;
        }
        let age = now.saturating_sub(self.start).min(duration);
        ((u64::from(self.from) * (duration - age) + u64::from(self.to) * age) / duration) as u8
    }
}

type Decoded = (DltDecoder, Option<Arc<RgbaImage>>, bool);

pub(super) struct OpeningDlt {
    placement: Placement,
    size: [u32; 2],
    opening: Option<Task<DltDecoder>>,
    work: Option<Task<Decoded>>,
    worker: TaskPool,
    decoder: Option<DltDecoder>,
    path: Option<String>,
    frame: Option<u64>,
    fade: Option<Fade>,
    level: u8,
    playing: bool,
    finished: bool,
    retired: bool,
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

impl OpeningDlt {
    pub fn preparing_cg(&self) -> Option<i32> {
        self.opening.as_ref().map(|_| self.placement.id)
    }

    pub fn slot(&self) -> u8 {
        self.placement.slot
    }

    fn layer_id(&self) -> u32 {
        256 + u32::from(self.slot())
    }

    fn layer(&self, paint: Value, level: u8) -> Value {
        let [x, y] = self.placement.origin;
        let [w, h] = self.size;
        json!({"op":"put","layer":{"id":self.layer_id(),"order":[2,self.slot()],"rect":[x,y,w,h],
            "paint":paint,"opacity":level.saturating_mul(16)}})
    }

    pub fn prepare(h: &mut dyn HostServices, c: ClientId, placement: Placement) -> Result<Self> {
        let storage = h.storage(c)?;
        // The loop runs during dialogue, so it must never occupy the only
        // foreground slot used for voice and script resources.
        let worker = TaskPool::new("exhibit-opening-dlt", 1, 1)?;
        let opening = worker.spawn(move |cancel| {
            cancel.check()?;
            let id = placement.id;
            let asset = VfsPath::new(
                Mount::Asset,
                &format!("res/g/ev/{:02}/{id:04}.dlt", id / 100),
            )?;
            let reader = storage.open_read(&asset)?;
            let bytes = crysirror_resource::read_all(reader.as_ref(), 32 << 20)?;
            cancel.check()?;
            ensure!(
                bytes.starts_with(&0x0101u32.to_le_bytes()),
                "unsupported v1 opening DLT version"
            );
            DltDecoder::open(bytes.into())
        })?;
        Ok(Self {
            placement,
            size: [0; 2],
            opening: Some(opening),
            work: None,
            worker,
            decoder: None,
            path: None,
            frame: None,
            fade: None,
            level: 0,
            playing: false,
            finished: false,
            retired: false,
        })
    }

    pub fn start(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        level: u8,
        duration: u32,
    ) -> Result<()> {
        ensure!(
            self.opening.is_none() && self.decoder.is_some() && !self.playing,
            "invalid ExHibit opening DLT start"
        );
        self.playing = true;
        self.fade = Some(Fade {
            start: h.now_ms(),
            from: self.level,
            to: level,
            duration,
            retiring: false,
        });
        self.request_frame(h.now_ms(), c)
    }

    pub fn stop(&mut self, now: u64, duration: u32) -> Result<()> {
        ensure!(self.fade.is_none(), "invalid ExHibit opening DLT stop");
        self.fade = Some(Fade {
            start: now,
            from: self.level,
            to: 0,
            duration,
            retiring: true,
        });
        Ok(())
    }

    pub fn retired(&self) -> bool {
        self.retired
    }

    fn request_frame(&mut self, now: u64, c: ClientId) -> Result<()> {
        if self.work.is_some() || self.finished {
            return Ok(());
        }
        let Some(decoder) = &self.decoder else {
            return Ok(());
        };
        let count = native_steps(self.frame, now, decoder.info().interval_ms);
        if count == 0 {
            return Ok(());
        }
        let permit = self.worker.reserve()?;
        let mut decoder = self.decoder.take().context("missing ExHibit DLT decoder")?;
        self.work = Some(permit.spawn(move |cancel| {
            let mut latest = None;
            let mut finished = false;
            for _ in 0..count {
                cancel.check()?;
                let Some(frame) = decoder.next_looped_frame()? else {
                    finished = true;
                    break;
                };
                latest = Some(frame);
            }
            let image = latest
                .map(|frame| {
                    RgbaImage::new(c, decoder.info().size, frame.rgba.into()).map(Arc::new)
                })
                .transpose()?;
            Ok((decoder, image, finished))
        })?);
        self.frame = Some(now);
        Ok(())
    }

    pub fn present(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        if let Some(task) = &mut self.opening {
            let Some(decoder) = task.try_take()? else {
                return Ok(false);
            };
            self.size = decoder.info().size;
            send(
                h,
                c,
                self.layer(json!({"kind":"solid","color":[0,0,0,0]}), 0),
            )?;
            self.decoder = Some(decoder);
            self.opening = None;
        }
        let now = h.now_ms();
        if let Some(fade) = &self.fade {
            let level = fade.level(now);
            if level == 0 && fade.retiring {
                send(h, c, json!({"op":"remove","id":self.layer_id()}))?;
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
                    json!({"op":"set_opacity","id":self.layer_id(),"opacity":level.saturating_mul(16)}),
                )?;
                self.level = level;
            }
            if !fade.retiring && level == fade.to && self.path.is_some() {
                self.fade = None;
            }
        }
        if let Some(work) = &mut self.work {
            if let Some((decoder, image, finished)) = work.try_take()? {
                self.finished = finished;
                if let Some(image) = image {
                    let uploaded = h.publish_image_resource(c, self.layer_id(), image)?;
                    let path = uploaded.path.relative().to_owned();
                    if let Err(error) = send(
                        h,
                        c,
                        self.layer(json!({"kind":"image","asset":path}), self.level),
                    ) {
                        let _ = h.release_image_resource(c, &path);
                        return Err(error);
                    }
                    if let Some(old) = self.path.replace(path) {
                        h.release_image_resource(c, &old)?;
                    }
                }
                self.decoder = Some(decoder);
                self.work = None;
            }
        }
        if self.playing {
            self.request_frame(now, c)?;
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
        let mut clip = OpeningDlt {
            placement: Placement {
                id: 1180,
                slot: 40,
                origin: [200, 50],
            },
            size: [1, 1],
            opening: None,
            work: None,
            worker: TaskPool::new("opening-dlt-fixture", 1, 1)?,
            decoder: Some(minimal_stream()?),
            path: None,
            frame: None,
            fade: None,
            level: 0,
            playing: false,
            finished: false,
            retired: false,
        };
        let mut host = Host::default();
        let client = ClientId(1);
        clip.start(&mut host, client, 16, 800)?;
        let until = Instant::now() + Duration::from_secs(10);
        while host.frames == 0 {
            assert!(!clip.present(&mut host, client)?);
            ensure!(Instant::now() < until, "fixture frame was not produced");
            std::thread::yield_now();
        }
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0]["layer"]["opacity"],
            0
        );
        let layer = &host.submissions.last().unwrap()["layers"][0]["layer"];
        assert_eq!(layer["id"], 296);
        assert_eq!(layer["rect"], json!([200, 50, 1, 1]));
        host.clock = 400;
        assert!(!clip.present(&mut host, client)?);
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0]["opacity"],
            128
        );
        while host.frames < 2 {
            assert!(!clip.present(&mut host, client)?);
            ensure!(
                Instant::now() < until,
                "fixture catchup frame was not produced"
            );
            std::thread::yield_now();
        }
        assert_eq!(host.released, ["fixture/dlt-1"]);
        host.clock = 800;
        assert!(clip.present(&mut host, client)?);
        clip.stop(800, 800)?;
        host.clock = 1551;
        assert!(clip.present(&mut host, client)?);
        assert!(clip.retired());
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0],
            json!({"op":"remove","id":296})
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
        let fade = Fade {
            start: 0,
            duration: 800,
            from: 16,
            to: 0,
            retiring: true,
        };
        assert_eq!(fade.level(400), 8);
        assert_eq!(fade.level(750), 1);
        assert_eq!(fade.level(751), 0);
    }
}
