//! Prototype: 4cam's rotary stock simulation on the GPU (wgpu: Vulkan, DX12 or Metal), with
//! the CPU simulation as the fallback. See docs/gpu.md.
//!
//! The CPU still decides where the tool is: it samples every move exactly as
//! `fourcam::sim::apply_segment` does and works out each position's window of cells in f64.
//! The GPU computes where each cell's ray enters the tool (the CPU's iteration, in f32) and
//! keeps the lowest with `atomicMin` on a key: height in micrometres, operation tag in the
//! low 8 bits.
//!
//! Compared with the CPU, heights agree to within the 1 µm quantization plus f32 rounding,
//! and on equal heights the tag is the lowest one, not the last. Toolpaths and G-code never
//! come from here.

use fourcam::dropcutter::Cutter;
use fourcam::sim::{self, RadialMap};
use fourcam::Toolpath;
use std::f64::consts::{PI, TAU};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GridU {
    x0: f32,
    dx: f32,
    nx: u32,
    na: u32,
    count: u32,
    win_i: u32,
    win_j: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[doc(hidden)]
pub struct Pos {
    pub x: f32,
    pub r: f32,
    pub phi_t: f32,
    pub radius: f32,
    pub kind: u32,
    pub inv_tan: f32,
    pub tag: u32,
    pub i0: u32,
    pub i1: u32,
    pub j0: i32,
    pub j1: i32,
    pub _pad: u32,
}

/// Tool positions per dispatch (the dispatch's Y size).
const BATCH: usize = 65_535;

/// A GPU ready to simulate.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// The adapter's name, e.g. "NVIDIA GeForce RTX 5080".
    pub name: String,
}

impl Gpu {
    /// The first high-performance hardware adapter, or None (no GPU, or only a software one,
    /// as on headless CI).
    pub fn new() -> Option<Gpu> {
        pollster::block_on(Self::init())
    }

    async fn init() -> Option<Gpu> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .ok()?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return None;
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("fourcam"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .ok()?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stamp"),
            source: wgpu::ShaderSource::Wgsl(include_str!("stamp.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("stamp"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        Some(Gpu {
            device,
            queue,
            pipeline,
            layout,
            name: info.name,
        })
    }

    /// Cut `tps` into `stock` on the GPU. Errors (the caller falls back to the CPU) on indexed
    /// 3+1 moves and on more than 255 operations.
    pub fn simulate(
        &self,
        stock: &mut RadialMap,
        tps: &[Toolpath],
        cutter_for: &dyn Fn(u32) -> Option<Cutter>,
        step: f64,
    ) -> Result<usize, String> {
        let positions = positions(stock, tps, cutter_for, step)?;
        self.stamp_positions(stock, &positions)?;
        Ok(positions.len())
    }

    /// Stamp prepared tool positions into `stock`.
    #[doc(hidden)]
    pub fn stamp_positions(&self, stock: &mut RadialMap, positions: &[Pos]) -> Result<(), String> {
        let g = stock.grid;
        let cells = g.len();
        // keys: height in µm << 8 | tag
        let keys: Vec<u32> = stock
            .r
            .iter()
            .zip(&stock.op)
            .map(|(&r, &op)| (((r.max(0.0) * 1000.0) as u32) << 8) | (op as u32 & 255))
            .collect();
        use wgpu::util::DeviceExt;
        let key_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("keys"),
                contents: bytemuck::cast_slice(&keys),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let pos_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("positions"),
            size: (BATCH * std::mem::size_of::<Pos>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uni_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid"),
            size: std::mem::size_of::<GridU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stamp"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uni_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pos_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: key_buf.as_entire_binding(),
                },
            ],
        });
        for batch in positions.chunks(BATCH) {
            let win_i = batch.iter().map(|p| p.i1 - p.i0 + 1).max().unwrap_or(1);
            let win_j = batch
                .iter()
                .map(|p| (p.j1 - p.j0 + 1).max(1) as u32)
                .max()
                .unwrap_or(1);
            let u = GridU {
                x0: g.x0 as f32,
                dx: g.dx as f32,
                nx: g.nx as u32,
                na: g.na as u32,
                count: batch.len() as u32,
                win_i,
                win_j,
                _pad: 0,
            };
            self.queue.write_buffer(&uni_buf, 0, bytemuck::bytes_of(&u));
            self.queue
                .write_buffer(&pos_buf, 0, bytemuck::cast_slice(batch));
            let mut enc = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups((win_i * win_j).div_ceil(64), batch.len() as u32, 1);
            }
            self.queue.submit(Some(enc.finish()));
            // the next batch rewrites the position and grid buffers
            self.device
                .poll(wgpu::PollType::Wait)
                .map_err(|e| format!("GPU: {e}"))?;
        }
        // read the keys back
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read"),
            size: (cells * 4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&key_buf, 0, &read, 0, (cells * 4) as u64);
        self.queue.submit(Some(enc.finish()));
        let slice = read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::Wait)
            .map_err(|e| format!("GPU: {e}"))?;
        let data = slice.get_mapped_range();
        let out: &[u32] = bytemuck::cast_slice(&data);
        for (k, &key) in out.iter().enumerate() {
            let r = (key >> 8) as f32 / 1000.0;
            // cells the GPU did not lower keep the CPU's exact radius and tag
            if (key >> 8) < keys[k] >> 8 {
                stock.r[k] = r;
                stock.op[k] = (key & 255) as u16;
            }
        }
        Ok(())
    }
}

/// Every tool position of `tps`, sampled as `sim::apply_segment` samples, with its cell
/// window worked out in f64 as `sim::stamp` does.
#[doc(hidden)]
pub fn positions(
    stock: &RadialMap,
    tps: &[Toolpath],
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    step: f64,
) -> Result<Vec<Pos>, String> {
    let g = stock.grid;
    let dphi = TAU / g.na as f64;
    let mut out = Vec::new();
    for seg in sim::segments(tps) {
        if seg.index_a.is_some() {
            return Err("indexed 3+1 moves are simulated on the CPU".into());
        }
        if seg.op > 255 {
            return Err("more than 255 operations".into());
        }
        let (Some(c), Some([x0, r0, a0])) = (seg.tool.and_then(cutter_for), seg.from) else {
            continue;
        };
        let (kind, inv_tan) = match c {
            Cutter::Flat { .. } => (0, 0.0),
            Cutter::Ball { .. } => (1, 0.0),
            Cutter::Cone { half_angle, .. } => (2, 1.0 / half_angle.tan()),
        };
        let rad = c.radius();
        let [x1, r1, a1] = seg.to;
        let arc = 0.5 * (r0 + r1) * (a1 - a0).to_radians().abs();
        let len = (x1 - x0).abs().max((r1 - r0).abs()).max(arc);
        let n = ((len / step).ceil() as usize).max(1);
        for k in 1..=n {
            let t = k as f64 / n as f64;
            let (x, r, a) = (x0 + (x1 - x0) * t, r0 + (r1 - r0) * t, a0 + (a1 - a0) * t);
            let phi_t = (PI / 2.0 - a.to_radians()).rem_euclid(TAU);
            let i0 = (((x - rad - g.x0) / g.dx - 0.5).ceil()).max(0.0) as usize;
            let i1 = ((x + rad - g.x0) / g.dx - 0.5).floor();
            if i1 < 0.0 || i0 >= g.nx {
                continue;
            }
            let i1 = (i1 as usize).min(g.nx - 1);
            if i0 > i1 {
                continue;
            }
            let half = if r <= rad { PI / 2.0 } else { (rad / r).asin() };
            let j0 = ((phi_t - half) / dphi - 0.5).ceil() as i64;
            let j1 = ((phi_t + half) / dphi - 0.5).floor() as i64;
            if j1 < j0 {
                continue;
            }
            out.push(Pos {
                x: x as f32,
                r: r as f32,
                phi_t: phi_t as f32,
                radius: rad as f32,
                kind,
                inv_tan: inv_tan as f32,
                tag: seg.op as u32,
                i0: i0 as u32,
                i1: i1 as u32,
                j0: j0 as i32,
                j1: j1 as i32,
                _pad: 0,
            });
        }
    }
    Ok(out)
}

/// Which simulation ran.
#[derive(Debug, Clone, PartialEq)]
pub enum Backend {
    Gpu(String),
    Cpu(String),
}

/// Simulate on the GPU when one is given and the job suits it, else on the CPU (threads).
pub fn simulate_or_cpu(
    gpu: Option<&Gpu>,
    stock: &mut RadialMap,
    tps: &[Toolpath],
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    step: f64,
) -> Backend {
    if let Some(g) = gpu {
        let mut trial = stock.clone();
        match g.simulate(&mut trial, tps, cutter_for, step) {
            Ok(_) => {
                *stock = trial;
                return Backend::Gpu(g.name.clone());
            }
            Err(e) => {
                for tp in tps {
                    sim::simulate(stock, tp, cutter_for, step);
                }
                return Backend::Cpu(e);
            }
        }
    }
    for tp in tps {
        sim::simulate(stock, tp, cutter_for, step);
    }
    Backend::Cpu("no GPU".into())
}
