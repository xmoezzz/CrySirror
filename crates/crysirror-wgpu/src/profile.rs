// SPDX-License-Identifier: MPL-2.0
//! Bounded GPU timestamp/readback ring. Live frames never wait for a mapping.
use crate::Gpu;
use crysirror_tasks::profile::Profiler;
use std::sync::mpsc::{self, Receiver};
pub(crate) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let required = wgpu::Features::TIMESTAMP_QUERY;
    if adapter.features().contains(required)
        && adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        && adapter.limits().max_storage_buffers_per_shader_stage >= 1
    {
        required
    } else {
        wgpu::Features::empty()
    }
}
struct Slot {
    query: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    pending: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    sample: Option<(Profiler, u64)>,
    completed: Option<Receiver<()>>,
}
pub(crate) struct GpuTimings {
    slots: Vec<Slot>,
    marker: Option<(wgpu::ComputePipeline, wgpu::BindGroup)>,
    pub dropped: u64,
    pub failures: u64,
}
impl GpuTimings {
    pub fn new(gpu: &Gpu) -> Self {
        let mut slots = vec![];
        if gpu
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
        {
            for _ in 0..4 {
                slots.push(Slot {
                    query: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("frame timestamps"),
                        ty: wgpu::QueryType::Timestamp,
                        count: 4,
                    }),
                    resolve: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("timestamp resolve"),
                        size: 256,
                        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    }),
                    read: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("timestamp readback"),
                        size: 32,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                    pending: None,
                    completed: None,
                    sample: None,
                });
            }
        }
        let marker = if slots.is_empty() {
            None
        } else {
            let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("profiler timestamp marker"),source:wgpu::ShaderSource::Wgsl("@group(0) @binding(0) var<storage,read_write> marker:u32; @compute @workgroup_size(1) fn main(){marker=marker+1u;}".into())});
            let pipeline = gpu
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("profiler marker"),
                    layout: None,
                    module: &shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
            let scratch = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("profiler marker storage"),
                size: 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("profiler marker bind"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scratch.as_entire_binding(),
                }],
            });
            Some((pipeline, bind))
        };
        Self {
            marker,
            slots,
            dropped: 0,
            failures: 0,
        }
    }
    pub fn pending(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.pending.is_some() || s.completed.is_some())
            .count()
    }
    pub fn enabled(&self) -> bool {
        !self.slots.is_empty()
    }
    /// Controlled capture/shutdown only. Live frames never call this fence.
    pub fn finish_offline(&mut self, gpu: &Gpu) -> anyhow::Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while self.pending() > 0 {
            self.poll(gpu)?;
            if self.pending() == 0 {
                break;
            }
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(|| anyhow::anyhow!("GPU profiler completion timed out"))?;
            gpu.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(remaining),
            })?;
        }
        Ok(())
    }
    pub fn poll(&mut self, gpu: &Gpu) -> anyhow::Result<()> {
        gpu.device.poll(wgpu::PollType::Poll)?;
        for s in &mut self.slots {
            if let Some(receiver) = &s.completed {
                match receiver.try_recv() {
                    Ok(()) => {
                        s.completed = None;
                        let mut encoder =
                            gpu.device
                                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("completed GPU timestamp resolve"),
                                });
                        encoder.resolve_query_set(&s.query, 0..4, &s.resolve, 0);
                        encoder.copy_buffer_to_buffer(&s.resolve, 0, &s.read, 0, 32);
                        gpu.queue.submit(Some(encoder.finish()));
                        let (tx, rx) = mpsc::channel();
                        s.read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                            let _ = tx.send(r);
                        });
                        s.pending = Some(rx);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        s.completed = None;
                        s.sample = None;
                        self.failures += 1;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            let ready = match s.pending.as_ref().map(Receiver::try_recv) {
                Some(Ok(v)) => Some(v.is_ok()),
                Some(Err(mpsc::TryRecvError::Disconnected)) => Some(false),
                _ => None,
            };
            if let Some(ok) = ready {
                s.pending = None;
                if ok {
                    let data = s.read.slice(..).get_mapped_range();
                    let a = u64::from_le_bytes(data[8..16].try_into().unwrap());
                    let b = u64::from_le_bytes(data[16..24].try_into().unwrap());
                    if let Some((p, frame)) = s.sample.take() {
                        if b > a && a != 0 {
                            p.gpu_duration(
                                frame,
                                "frame",
                                (b - a) as f64 * gpu.queue.get_timestamp_period() as f64,
                            );
                        } else {
                            self.failures += 1;
                        }
                    }
                    drop(data);
                } else {
                    self.failures += 1;
                    s.sample = None;
                }
                s.read.unmap();
            }
        }
        Ok(())
    }
    pub fn begin(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        p: Profiler,
        frame: u64,
    ) -> Option<usize> {
        let Some(i) = self
            .slots
            .iter()
            .position(|s| s.pending.is_none() && s.completed.is_none())
        else {
            if self.enabled() {
                self.dropped += 1;
            }
            return None;
        };
        let s = &mut self.slots[i];
        s.sample = Some((p, frame));
        self.write_marker(encoder, i, true);
        Some(i)
    }
    pub fn end(&self, encoder: &mut wgpu::CommandEncoder, i: Option<usize>) {
        if let Some(i) = i {
            self.write_marker(encoder, i, false);
        }
    }
    fn write_marker(&self, encoder: &mut wgpu::CommandEncoder, i: usize, start: bool) {
        // Pass-boundary writes avoid Metal's deferred blit timestamp path.
        // A real storage write prevents an empty marker encoder being elided.
        let (pipeline, bind) = self.marker.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("profile frame boundary"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.slots[i].query,
                beginning_of_pass_write_index: Some(if start { 0 } else { 2 }),
                end_of_pass_write_index: Some(if start { 1 } else { 3 }),
            }),
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    pub fn submitted(&mut self, gpu: &Gpu, i: Option<usize>) {
        if let Some(i) = i {
            let (tx, rx) = mpsc::channel();
            gpu.queue.on_submitted_work_done(move || {
                let _ = tx.send(());
            });
            self.slots[i].completed = Some(rx);
        }
    }
}
