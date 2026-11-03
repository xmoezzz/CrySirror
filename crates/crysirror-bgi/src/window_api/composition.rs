//! Destination-reading window display policy over shared prefix/composite jobs.
use super::*;
use crysirror_core::image_processing::{
    ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageJob, ImageProcessState, ImageResource,
};
#[derive(Default)]
pub(super) struct WindowComposition {
    requested: Value,
    committed: Option<Value>,
    asset: Option<(String, [u32; 2])>,
    work: Option<CompositionJob>,
}
struct CompositionJob {
    signature: Value,
    screen: [u32; 2],
    position: [i32; 2],
    mode: i32,
    parameter: i32,
    prefix: Option<ImageJob>,
    source: Option<ImageJob>,
    images: [Option<Arc<dyn ImageResource>>; 2],
    process: Option<ImageJob>,
}
impl BgiVm {
    pub(super) fn release_window_composition(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
    ) -> Result<()> {
        if let Some(state) = self.windows.compositions.remove(&window) {
            if let Some((path, _)) = state.asset {
                host.release_image_resource(self.client, &path)?;
            }
        }
        Ok(())
    }
    pub(crate) fn window_prefix_ready(&self, object: i32) -> bool {
        let cut = self.graph.tree.chain_position(object).unwrap_or(0);
        self.windows.records.iter().all(|(&window, w)| {
            if self
                .graph
                .tree
                .chain_position(window)
                .is_none_or(|position| position >= cut)
            {
                return true;
            }
            if w.dirty {
                return false;
            }
            let Some(p) = self.graph.objects.get(&window) else {
                return true;
            };
            let visible = self.graph.global_enabled != 0
                && p.native.draw_enabled != 0
                && p.native.suppress_draw == 0
                && p.mask_alpha < 256
                && p.alpha_multiplier != 0
                && self.graph.tree.chain_drawable(window);
            if p.blend_mode == 128 || !visible {
                return true;
            }
            self.windows.compositions.get(&window).is_some_and(|state| {
                state.work.is_none() && state.committed.as_ref() == Some(&state.requested)
            })
        })
    }
    pub(crate) fn tick_window_compositions(&mut self, host: &mut dyn HostServices) -> Result<()> {
        if !self.windows.compositions.is_empty() {
            self.publish_windows(host)?;
        }
        Ok(())
    }
    pub(super) fn window_composition_paint(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
        position: [i32; 2],
        visible: bool,
    ) -> Result<(Value, [u32; 2])> {
        let w = self.windows.get(window)?;
        let p = self
            .graph
            .objects
            .get(&window)
            .context("BGI composited window properties")?;
        let order = [
            p.native.priority as i32,
            self.graph
                .tree
                .chain_position(window)
                .context("BGI composited window order")? as i32,
        ];
        let mode = p.blend_mode;
        let parameter = p.transparency_parameter();
        target_checks::effect_mode(mode)?;
        target_checks::effect_level(parameter)?;
        let signature = json!({"source":w.content_version,"size":w.size,"screen":self.graph.screen,"order":order,
            "position":position,"mode":mode,"parameter":parameter,"visible":visible,"prefix":self.graph.visual_prefix(window)});
        let ready = self.graph_prefix_ready(window) && self.window_prefix_ready(window);
        let mut state = self
            .windows
            .compositions
            .remove(&window)
            .unwrap_or_default();
        state.requested = signature.clone();
        if state.work.is_none() && state.committed.as_ref() != Some(&signature) && ready {
            if visible {
                let size = self.windows.get(window)?.size;
                let left = (-i64::from(position[0])).max(0).min(i64::from(size[0]));
                let top = (-i64::from(position[1])).max(0).min(i64::from(size[1]));
                let right = (i64::from(self.graph.screen[0]) - i64::from(position[0]))
                    .max(left)
                    .min(i64::from(size[0]));
                let bottom = (i64::from(self.graph.screen[1]) - i64::from(position[1]))
                    .max(top)
                    .min(i64::from(size[1]));
                let region = [
                    left as u32,
                    top as u32,
                    (right - left) as u32,
                    (bottom - top) as u32,
                ];
                let source = if region[2] == 0 || region[3] == 0 {
                    None
                } else {
                    Some(host.begin_canvas_region_image(
                        self.client,
                        NativeWindow::canvas(window, 3),
                        region,
                    )?)
                };
                state.work = Some(CompositionJob {
                    signature,
                    screen: self.graph.screen,
                    position: [
                        (i64::from(position[0]) + left) as i32,
                        (i64::from(position[1]) + top) as i32,
                    ],
                    mode,
                    parameter,
                    prefix: Some(host.begin_layer_prefix_image_over(
                        self.client,
                        self.graph.screen,
                        order,
                        [0, 0, 0, 255],
                    )?),
                    source,
                    images: [None, None],
                    process: None,
                });
            } else {
                state.committed = Some(signature);
            }
        }
        let mut complete = false;
        if let Some(work) = &mut state.work {
            for (index, job) in [(0, &mut work.prefix), (1, &mut work.source)] {
                if let Some(task) = job {
                    match task.poll()? {
                        ImageProcessState::Pending => (),
                        ImageProcessState::Failed(error) => {
                            bail!("BGI window display capture: {error}")
                        }
                        ImageProcessState::Ready(image) => {
                            work.images[index] = Some(image);
                            *job = None;
                        }
                    }
                }
            }
            if work.images[0].is_some() && work.source.is_none() && work.process.is_none() {
                let layers = if let Some(source) = &work.images[1] {
                    let size = source.size();
                    vec![ImageCompositeLayer {
                        input: Box::new(ImageInput::Resource(source.clone())),
                        source: [0, 0, size[0] as i32, size[1] as i32],
                        destination: work.position,
                        clip: None,
                        opacity: 255,
                        math: ImageCompositeMath::IntegerStored {
                            destination_format: 1,
                            source_format: 2,
                            mode: work.mode,
                            parameter: work.parameter,
                        },
                    }]
                } else {
                    Vec::new()
                };
                let input = ImageInput::Composite {
                    base: Box::new(ImageInput::Resource(
                        work.images[0].as_ref().unwrap().clone(),
                    )),
                    layers,
                };
                work.process = Some(host.image_metadata(self.client)?.begin_process(
                    input,
                    Arc::from(serde_json::to_vec(&vec![
                        crysirror_image::effects::ImageEffect::OverrideAlpha { alpha: 255 },
                    ])?),
                )?);
            }
            if let Some(job) = &mut work.process {
                match job.poll()? {
                    ImageProcessState::Pending => (),
                    ImageProcessState::Failed(error) => {
                        bail!("BGI window display composite: {error}")
                    }
                    ImageProcessState::Ready(image) => {
                        let image = host.publish_image_resource(
                            self.client,
                            0xbb000000 | (window as u32 & 15),
                            image,
                        )?;
                        state.asset = Some((image.path.relative().to_owned(), work.screen));
                        state.committed = Some(work.signature.clone());
                        complete = true;
                    }
                }
            }
        }
        if complete {
            state.work = None;
            self.graph.mark_visual(window);
        }
        let paint = state.asset.as_ref().map_or_else(
            || (json!({"kind":"solid","color":[0,0,0,0]}), self.graph.screen),
            |(path, size)| (json!({"kind":"image","asset":path}), *size),
        );
        self.windows.compositions.insert(window, state);
        Ok(paint)
    }
}
