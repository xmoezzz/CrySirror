//! Native window bitmap policy over the shared integer composite worker.
use super::*;
use crysirror_core::image_processing::{
    ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageJob, ImageProcessState, ImageResource,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowBlendOwner {
    Native(u64),
    Selection(u64),
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowBlit {
    pub bitmap: i32,
    pub position: [i32; 2],
    pub mode: i32,
    pub parameter: i32,
}
enum Input {
    Image(Arc<dyn ImageResource>),
    Capture(ImageJob),
}
pub(super) struct WindowRasterJob {
    owner: WindowBlendOwner,
    reset: bool,
    inputs: Vec<Input>,
    formats: Vec<i32>,
    operations: Vec<WindowBlit>,
    job: Option<ImageJob>,
}
impl BgiVm {
    fn window_raster_input(
        &self,
        host: &mut dyn HostServices,
        bitmap: i32,
    ) -> Result<(Input, i32)> {
        if let Some(w) = self.windows.records.get(&bitmap) {
            Ok((
                if let Some(image) = &w.backing_raw {
                    Input::Image(image.clone())
                } else {
                    Input::Capture(
                        host.begin_canvas_image(self.client, NativeWindow::canvas(bitmap, 0))?,
                    )
                },
                2,
            ))
        } else {
            let (id, b) = self.bitmap(bitmap)?;
            Ok((
                if let Some(image) = &b.image {
                    Input::Image(image.clone())
                } else {
                    Input::Capture(host.begin_canvas_image(self.client, id)?)
                },
                b.format,
            ))
        }
    }
    pub(crate) fn cancel_window_blend(&mut self, window: i32, owner: WindowBlendOwner) {
        if self
            .windows
            .blends
            .get(&window)
            .is_some_and(|job| job.owner == owner)
        {
            self.windows.blends.remove(&window);
        }
    }
    pub(crate) fn window_blend(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
        owner: WindowBlendOwner,
        reset: bool,
        operations: Vec<WindowBlit>,
    ) -> Result<bool> {
        self.windows.get(window)?;
        if reset && operations.is_empty() && !self.windows.blends.contains_key(&window) {
            self.submit(host,json!({"canvases":[{"op":"clear","id":NativeWindow::canvas(window,0),"color":[0,0,0,0]}]}))?;
            if let Some(path) = self.windows.get(window)?.backing_path.clone() {
                host.release_image_resource(self.client, &path)?;
            }
            let w = self.windows.get_mut(window)?;
            w.backing_raw = None;
            w.backing_path = None;
            w.dirty = true;
            self.publish_windows(host)?;
            return Ok(true);
        }
        if self
            .windows
            .blends
            .get(&window)
            .is_some_and(|job| job.owner != owner)
        {
            return Ok(false);
        }
        let mut work = if let Some(work) = self.windows.blends.remove(&window) {
            work
        } else {
            ensure!(
                operations.len() <= 128,
                "BGI window composite operation budget"
            );
            let mut inputs = Vec::new();
            let mut formats = Vec::new();
            if !reset {
                let (input, _) = self.window_raster_input(host, window)?;
                inputs.push(input);
            }
            for operation in &operations {
                target_checks::effect_mode(operation.mode)?;
                target_checks::effect_level(operation.parameter)?;
                let (input, format) = self.window_raster_input(host, operation.bitmap)?;
                inputs.push(input);
                formats.push(format);
            }
            WindowRasterJob {
                owner,
                reset,
                inputs,
                formats,
                operations,
                job: None,
            }
        };
        if work.job.is_none() {
            for input in &mut work.inputs {
                if let Input::Capture(job) = input {
                    match job.poll()? {
                        ImageProcessState::Pending => {
                            self.windows.blends.insert(window, work);
                            return Ok(false);
                        }
                        ImageProcessState::Failed(error) => bail!("BGI window capture: {error}"),
                        ImageProcessState::Ready(image) => *input = Input::Image(image),
                    }
                }
            }
            let images = work
                .inputs
                .iter()
                .map(|input| match input {
                    Input::Image(image) => image.clone(),
                    Input::Capture(_) => unreachable!(),
                })
                .collect::<Vec<_>>();
            let size = self.windows.get(window)?.size;
            let (base, offset) = if work.reset {
                (
                    ImageInput::Solid {
                        size,
                        color: [0, 0, 0, 0],
                    },
                    0,
                )
            } else {
                (ImageInput::Resource(images[0].clone()), 1)
            };
            let layers = work
                .operations
                .iter()
                .enumerate()
                .map(|(index, operation)| {
                    let image = images[index + offset].clone();
                    let size = image.size();
                    ImageCompositeLayer {
                        input: Box::new(ImageInput::Resource(image)),
                        source: [0, 0, size[0] as i32, size[1] as i32],
                        destination: operation.position,
                        clip: None,
                        opacity: 255,
                        math: ImageCompositeMath::IntegerStored {
                            destination_format: 2,
                            source_format: work.formats[index] as u8,
                            mode: operation.mode,
                            parameter: operation.parameter,
                        },
                    }
                })
                .collect();
            work.job = Some(host.image_metadata(self.client)?.begin_process(
                ImageInput::Composite {
                    base: Box::new(base),
                    layers,
                },
                Arc::from(b"[]".as_slice()),
            )?);
        }
        match work.job.as_mut().unwrap().poll()? {
            ImageProcessState::Pending => {
                self.windows.blends.insert(window, work);
                Ok(false)
            }
            ImageProcessState::Failed(error) => bail!("BGI window composite: {error}"),
            ImageProcessState::Ready(image) => {
                let uploaded = host.publish_image_resource(
                    self.client,
                    0xb9000000 | (window as u32 & 15),
                    image.clone(),
                )?;
                let asset = uploaded.path.relative();
                let size = self.windows.get(window)?.size;
                self.submit(host,json!({"canvases":[{"op":"set","id":NativeWindow::canvas(window,0),"layer":{"id":0,"rect":[0,0,size[0],size[1]],"paint":{"kind":"image","asset":asset}}}]}))?;
                let w = self.windows.get_mut(window)?;
                w.backing_raw = Some(image);
                w.backing_path = Some(asset.to_owned());
                w.dirty = true;
                self.publish_windows(host)?;
                Ok(true)
            }
        }
    }
}
