//! Presenter-owned, client-scoped mesh-layer feedback. Ordinary frames never
//! read pixels to the CPU; explicit readback is reserved for save/capture jobs.
use super::*;
use crysirror_core::service::{frame_history_path, MeshFrameHistory, MeshFrameHistoryProvider};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

struct Slot {
    size: [u32; 2],
    output: Option<wgpu::Texture>,
    history: Option<wgpu::Texture>,
    declaration: Option<Arc<crysirror_puppet::PuppetFrame>>,
    dependencies: Vec<String>,
}
#[derive(Default)]
struct State {
    clients: std::collections::BTreeMap<u32, usize>,
    slots: std::collections::BTreeMap<(u32, u32), Slot>,
}
pub(super) struct Provider {
    gpu: Arc<Gpu>,
    state: Mutex<State>,
    copies: AtomicU64,
    renders: AtomicU64,
    readbacks: AtomicU64,
}
struct Client {
    owner: Arc<Provider>,
    client: u32,
}
impl Drop for Client {
    fn drop(&mut self) {
        if let Ok(mut state) = self.owner.state.lock() {
            if let Some(leases) = state.clients.get_mut(&self.client) {
                *leases -= 1;
                if *leases == 0 {
                    state.clients.remove(&self.client);
                    state.slots.retain(|(client, _), _| *client != self.client);
                }
            }
        }
    }
}
fn error(e: impl std::fmt::Display) -> crysirror_core::Error {
    crysirror_core::Error::Io(e.to_string())
}
impl MeshFrameHistoryProvider for Provider {
    fn client(
        self: Arc<Self>,
        client: ClientId,
    ) -> crysirror_core::Result<Arc<dyn MeshFrameHistory>> {
        {
            let mut state = self.state.lock().map_err(error)?;
            if !state.clients.contains_key(&client.0) && state.clients.len() >= 64 {
                return Err(crysirror_core::Error::Limit("frame-history clients"));
            }
            *state.clients.entry(client.0).or_default() += 1;
        }
        Ok(Arc::new(Client {
            owner: self,
            client: client.0,
        }))
    }
    fn registered(&self, client: ClientId, layer: u32) -> bool {
        self.state
            .lock()
            .is_ok_and(|s| s.slots.contains_key(&(client.0, layer)))
    }
}
impl MeshFrameHistory for Client {
    fn texture(&self, layer: u32, size: [u32; 2]) -> crysirror_core::Result<String> {
        let bytes = crysirror_image::rgba_len(size[0], size[1]).map_err(error)? as u64;
        if size.iter().any(|n| *n > puppet::MAX_TARGET) {
            return Err(crysirror_core::Error::Limit("frame-history extent"));
        }
        let mut state = self.owner.state.lock().map_err(error)?;
        if state
            .slots
            .get(&(self.client, layer))
            .is_some_and(|s| s.size == size)
        {
            return Ok(frame_history_path(layer));
        }
        if !state.slots.contains_key(&(self.client, layer))
            && state
                .slots
                .keys()
                .filter(|(c, _)| *c == self.client)
                .count()
                >= 8
        {
            return Err(crysirror_core::Error::Limit(
                "frame-history slots per client",
            ));
        }
        let occupied: u64 = state
            .slots
            .iter()
            .filter(|(key, _)| **key != (self.client, layer))
            .map(|(_, s)| u64::from(s.size[0]) * u64::from(s.size[1]) * 8)
            .sum();
        if occupied + bytes * 2 > 128 * 1024 * 1024 {
            return Err(crysirror_core::Error::Limit("frame-history working set"));
        }
        state.slots.insert(
            (self.client, layer),
            Slot {
                size,
                output: None,
                history: None,
                declaration: None,
                dependencies: Vec::new(),
            },
        );
        Ok(frame_history_path(layer))
    }
    fn readback(&self, layer: u32, size: [u32; 2]) -> crysirror_core::Result<Vec<u8>> {
        crysirror_image::rgba_len(size[0], size[1]).map_err(error)?;
        let (source, extent) = {
            let state = self.owner.state.lock().map_err(error)?;
            let slot = state
                .slots
                .get(&(self.client, layer))
                .ok_or(crysirror_core::Error::NotFound)?;
            (
                slot.output.clone().ok_or(crysirror_core::Error::NotFound)?,
                slot.size,
            )
        };
        self.owner.readbacks.fetch_add(1, Ordering::Relaxed);
        let image = read_texture(&self.owner.gpu, &source, extent[0], extent[1]).map_err(error)?;
        Ok(if extent == size {
            image.into_raw()
        } else {
            image::imageops::resize(
                &image,
                size[0],
                size[1],
                image::imageops::FilterType::Triangle,
            )
            .into_raw()
        })
    }
}
impl Provider {
    pub fn new(gpu: Arc<Gpu>) -> Arc<Self> {
        Arc::new(Self {
            gpu,
            state: Mutex::new(State::default()),
            copies: AtomicU64::new(0),
            renders: AtomicU64::new(0),
            readbacks: AtomicU64::new(0),
        })
    }
    pub fn active(&self) -> bool {
        self.state.lock().is_ok_and(|s| !s.slots.is_empty())
    }
    pub fn stats(&self) -> serde_json::Value {
        serde_json::json!({"copies":self.copies.load(Ordering::Relaxed),"renders":self.renders.load(Ordering::Relaxed),"readbacks":self.readbacks.load(Ordering::Relaxed)})
    }
    /// Admit a logical declaration without mutating its previous picture.
    pub fn begin(
        &self,
        client: u32,
        layer: u32,
        size: [u32; 2],
        frame: &Arc<crysirror_puppet::PuppetFrame>,
        output: &wgpu::Texture,
        dependencies: &[String],
    ) -> Result<bool> {
        let mut state = self.state.lock().map_err(error)?;
        let Some(slot) = state.slots.get_mut(&(client, layer)) else {
            return Ok(true);
        };
        ensure!(
            slot.size == size,
            "frame-history logical layer size changed without reservation"
        );
        if slot
            .declaration
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, frame))
            && slot.output.as_ref() == Some(output)
            && slot.dependencies == dependencies
        {
            return Ok(false);
        }
        Ok(true)
    }
    /// Freeze every referenced source before encoding any current layer.
    /// This also handles an unchanged or hidden source read by a new consumer.
    pub fn freeze(&self, client: u32, layer: u32) -> Result<()> {
        let mut state = self.state.lock().map_err(error)?;
        let slot = state
            .slots
            .get_mut(&(client, layer))
            .context("unreserved frame-history source")?;
        let size = slot.size;
        if slot.history.is_none() {
            slot.history = Some(self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("mesh layer history"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }
        if let Some(previous) = &slot.output {
            let mut enc = self
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("mesh history copy"),
                });
            enc.copy_texture_to_texture(
                previous.as_image_copy(),
                slot.history.as_ref().unwrap().as_image_copy(),
                wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
            );
            self.gpu.queue.submit([enc.finish()]);
            self.copies.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
    pub fn history(&self, client: u32, layer: u32) -> Result<wgpu::Texture> {
        self.state
            .lock()
            .map_err(error)?
            .slots
            .get(&(client, layer))
            .and_then(|s| s.history.clone())
            .context("frame-history texture has not been admitted")
    }
    pub fn complete(
        &self,
        client: u32,
        layer: u32,
        frame: Arc<crysirror_puppet::PuppetFrame>,
        output: wgpu::Texture,
        dependencies: Vec<String>,
    ) -> Result<()> {
        let mut state = self.state.lock().map_err(error)?;
        if let Some(slot) = state.slots.get_mut(&(client, layer)) {
            slot.output = Some(output);
            slot.declaration = Some(frame);
            slot.dependencies = dependencies;
            self.renders.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

/// Common readback fence used by screenshots and explicit history reads.
pub(super) fn read_texture(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage> {
    let format = texture.format();
    ensure!(
        matches!(
            format,
            wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8UnormSrgb
                | wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Bgra8Unorm
        ),
        "unsupported RGBA8 readback source format: {format:?}"
    );
    crysirror_image::rgba_len(width, height)?;
    let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture readback"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let submission = gpu.queue.submit([enc.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(20)),
        })
        .context("GPU readback completion timeout")?;
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .context("GPU readback callback missing")??;
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in data.chunks_exact(stride as usize).take(height as usize) {
        pixels.extend_from_slice(&row[..width as usize * 4]);
    }
    drop(data);
    buffer.unmap();
    if matches!(
        format,
        wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
    ) {
        for p in pixels.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
    }
    image::RgbaImage::from_raw(width, height, pixels).context("invalid GPU readback dimensions")
}
