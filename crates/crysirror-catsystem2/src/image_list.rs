// SPDX-License-Identifier: MPL-2.0
//! Native image-list initialization policy over the shared image service.
//! 643D10→279→742660 allocates a zero32bpp work bitmap and a hidden1280/720
//! (configured virtual size) base plane. No private pixel/composition code lives
//! here: ImageInput::Solid is allocated by engine workers and published by owner.
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource},
    service::{ClientStorage, HostServices, ImageMetadata},
    state::ClientId,
};
use crysirror_resource::xml::XmlDocument;
use crysirror_tasks::{Task, TaskPool};
use std::sync::Arc;
fn page_size(storage: &dyn ClientStorage, check: &dyn Fn() -> Result<()>) -> Result<[u32; 2]> {
    check()?;
    let bytes = crate::native_settings::optional_asset(storage, "config/startup.xml")?
        .context("CatSystem2 texture page startup missing")?;
    let d = XmlDocument::parse(&bytes)?;
    let mut size = [0; 2];
    for (i, path) in ["TEXTURE/size/x", "TEXTURE/size/y"].iter().enumerate() {
        size[i] = crate::native_values::integer(
            &d.unique_child_path(path)?
                .context("CatSystem2 unsupported absent texture page definition")?
                .text,
        );
    }
    ensure!(
        size.iter().all(|v| (1..=8192).contains(v)),
        "CatSystem2 texture page extent"
    );
    check()?;
    Ok(size)
}
pub(crate) struct PreparedImages {
    pub page_size: [u32; 2],
    pub atlas: Arc<dyn ImageResource>,
    pub base: Arc<dyn ImageResource>,
}
pub(crate) struct PendingImages {
    pub task_id: u32,
    pub native_pc: u32,
    config: Task<[u32; 2]>,
    page: Option<[u32; 2]>,
    images: Arc<dyn ImageMetadata>,
    job: Option<ImageJob>,
    size: [u32; 2],
    viewport: [u32; 2],
    atlas: Option<Arc<dyn ImageResource>>,
    base: Option<Arc<dyn ImageResource>>,
}
impl PendingImages {
    pub fn begin(
        pool: &TaskPool,
        host: &mut dyn HostServices,
        client: ClientId,
        task_id: u32,
        native_pc: u32,
        size: [u32; 2],
        viewport: [u32; 2],
    ) -> Result<Option<Self>> {
        ensure!(
            size.iter().all(|v| (1..=8192).contains(v))
                && u64::from(size[0]) * u64::from(size[1]) <= 16 * 1024 * 1024,
            "CatSystem2 image-list bitmap extent budget"
        );
        let permit = match pool.reserve() {
            Ok(p) => p,
            Err(e) if e.is::<crysirror_tasks::CapacityExceeded>() => return Ok(None),
            Err(e) => return Err(e),
        };
        let storage = host.storage(client)?;
        let images = host.image_metadata(client)?;
        let config = permit.spawn(move |cancel| page_size(storage.as_ref(), &|| cancel.check()))?;
        Ok(Some(Self {
            task_id,
            native_pc,
            config,
            page: None,
            images,
            job: None,
            size,
            viewport,
            atlas: None,
            base: None,
        }))
    }
    pub fn poll(&mut self) -> Result<Option<PreparedImages>> {
        if self.page.is_none() {
            let Some(page) = self.config.try_take()? else {
                return Ok(None);
            };
            let pages = self.size[0]
                .div_ceil(page[0])
                .checked_mul(self.size[1].div_ceil(page[1]))
                .context("CatSystem2 page count overflow")?;
            ensure!(pages <= 4096, "CatSystem2 image-list dirty page budget");
            self.page = Some(page);
        }
        if self.atlas.is_none() || self.base.is_none() {
            if self.job.is_none() {
                let size = if self.atlas.is_none() {
                    self.size
                } else {
                    self.viewport
                };
                self.job = Some(self.images.begin_process(
                    ImageInput::Solid {
                        size,
                        color: [0; 4],
                    },
                    Arc::from(b"[]".as_slice()),
                )?);
                return Ok(None);
            }
            match self.job.as_mut().unwrap().poll()? {
                ImageProcessState::Pending => return Ok(None),
                ImageProcessState::Failed(e) => {
                    anyhow::bail!("CatSystem2 shared image-list preparation: {e}")
                }
                ImageProcessState::Ready(image) => {
                    if self.atlas.is_none() {
                        ensure!(image.size() == self.size, "CatSystem2 atlas prepared size");
                        self.atlas = Some(image);
                    } else {
                        ensure!(
                            image.size() == self.viewport,
                            "CatSystem2 base image prepared size"
                        );
                        self.base = Some(image);
                    }
                    self.job = None;
                }
            }
        }
        if let (Some(atlas), Some(base)) = (&self.atlas, &self.base) {
            Ok(Some(PreparedImages {
                page_size: self.page.unwrap(),
                atlas: atlas.clone(),
                base: base.clone(),
            }))
        } else {
            Ok(None)
        }
    }
}
