//! 4cam's stock simulation on the GPU (wgpu: Vulkan, DX12 or Metal), with the CPU
//! simulation as the fallback. See docs/gpu.md.
//!
//! The CPU still decides where the tool is (`fourcam::gpu_input`): it samples every move
//! exactly as the CPU simulation does and gives each position its window of cells in f64.
//! The shader (`web/gpu/stamp.wgsl`, shared with the web app's WebGPU path) evaluates the
//! cut per cell in f32 and keeps the lowest with `atomicMin` on a key: height in
//! micrometres, operation tag in the low 8 bits.
//!
//! Compared with the CPU, heights agree to within the 1 µm quantization plus f32 rounding,
//! and on equal heights the tag is the lowest one, not the last. Toolpaths and G-code never
//! come from here.

use fourcam::dropcutter::Cutter;
use fourcam::gpu_input::{self, GpuInput, POS_WORDS};
use fourcam::sim::{self, RadialMap};
use fourcam::sim3::{self, Heightmap3};
use fourcam::Toolpath;

/// The grid uniform (8 words).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GridParams {
    pub x0: f32,
    pub dx: f32,
    pub y0: f32,
    pub zbase: f32,
    pub nx: u32,
    pub ny: u32,
    pub _pad: [u32; 2],
}

/// Per-dispatch parameters live 256 bytes apart (the dynamic offset alignment).
const DISP_STRIDE: u64 = 256;

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
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../web/gpu/stamp.wgsl").into()),
        });
        let entry =
            |binding: u32, ty: wgpu::BufferBindingType, dynamic: bool| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty,
                    has_dynamic_offset: dynamic,
                    min_binding_size: None,
                },
                count: None,
            };
        let storage = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stamp"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform, false),
                entry(1, storage(true), false),
                entry(2, storage(false), false),
                entry(3, wgpu::BufferBindingType::Uniform, true),
                entry(4, storage(false), false),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stamp"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("stamp"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Some(Gpu {
            device,
            queue,
            pipeline,
            layout,
            name: info.name,
        })
    }

    /// Run segments `segs` of `input` on `keys`: the keys after, and for each segment of the
    /// job whether it was a rapid that cut material.
    pub fn stamp(
        &self,
        keys: &[u32],
        input: &GpuInput,
        segs: std::ops::Range<usize>,
        grid: GridParams,
    ) -> Result<(Vec<u32>, Vec<bool>), String> {
        use wgpu::util::DeviceExt;
        let cells = keys.len();
        let nseg = input.seg_rapid.len().max(1);
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let init = |label, contents: &[u8], usage| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage,
                })
        };
        let key_buf = init("keys", bytemuck::cast_slice(keys), usage);
        let hit_buf = init("hits", bytemuck::cast_slice(&vec![0u32; nseg]), usage);
        let empty = [0u32; POS_WORDS];
        let words: &[u32] = if input.words.is_empty() {
            &empty
        } else {
            &input.words
        };
        let pos_buf = init(
            "positions",
            bytemuck::cast_slice(words),
            wgpu::BufferUsages::STORAGE,
        );
        let grid_buf = init(
            "grid",
            bytemuck::bytes_of(&grid),
            wgpu::BufferUsages::UNIFORM,
        );
        // one entry per dispatch, each with the window its positions need
        let runs = input.runs(segs.start, segs.end, BATCH);
        let mut disp = vec![0u32; runs.len().max(1) * (DISP_STRIDE as usize / 4)];
        let mut sizes = Vec::new();
        for (k, &(first, count)) in runs.iter().enumerate() {
            let (mut wi, mut wj) = (1u32, 1u32);
            let range = first as usize * POS_WORDS..(first + count) as usize * POS_WORDS;
            for p in input.words[range].as_chunks::<POS_WORDS>().0 {
                wi = wi.max(p[9] - p[8] + 1);
                wj = wj.max((p[11] as i32 - p[10] as i32 + 1).max(1) as u32);
            }
            let o = k * DISP_STRIDE as usize / 4;
            disp[o..o + 4].copy_from_slice(&[first, count, wi, wj]);
            sizes.push(((wi * wj).div_ceil(64), count));
        }
        let disp_buf = init(
            "dispatches",
            bytemuck::cast_slice(&disp),
            wgpu::BufferUsages::UNIFORM,
        );
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stamp"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pos_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: key_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &disp_buf,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: hit_buf.as_entire_binding(),
                },
            ],
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            // dispatches run in order: each sees the stock the ones before it left
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            for (k, &(gx, gy)) in sizes.iter().enumerate() {
                pass.set_bind_group(0, &bind, &[(k as u64 * DISP_STRIDE) as u32]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
        }
        let read = |buf: &wgpu::Buffer, n: usize, enc: &mut wgpu::CommandEncoder| {
            let r = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("read"),
                size: (n * 4) as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            enc.copy_buffer_to_buffer(buf, 0, &r, 0, (n * 4) as u64);
            r
        };
        let rk = read(&key_buf, cells, &mut enc);
        let rh = read(&hit_buf, nseg, &mut enc);
        self.queue.submit(Some(enc.finish()));
        for r in [&rk, &rh] {
            r.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }
        self.device
            .poll(wgpu::PollType::Wait)
            .map_err(|e| format!("GPU: {e}"))?;
        let out = bytemuck::cast_slice::<u8, u32>(&rk.slice(..).get_mapped_range()).to_vec();
        let hits = bytemuck::cast_slice::<u8, u32>(&rh.slice(..).get_mapped_range())
            .iter()
            .map(|&h| h != 0)
            .collect();
        Ok((out, hits))
    }

    /// Cut `tps` into a rotary `stock`. Errors (the caller falls back to the CPU) on indexed
    /// 3+1 moves and on more than 255 operations. Returns the rapids that cut material, as
    /// `SimStats::rapid_hits` lists them (move indices, in order).
    pub fn simulate(
        &self,
        stock: &mut RadialMap,
        tps: &[Toolpath],
        cutter_for: &dyn Fn(u32) -> Option<Cutter>,
        step: f64,
    ) -> Result<Vec<usize>, String> {
        let input = gpu_input::rotary(stock.grid, tps, cutter_for, step)?;
        let g = stock.grid;
        let before = gpu_input::rotary_keys(stock);
        let (keys, hits) = self.stamp(
            &before,
            &input,
            0..input.seg_rapid.len(),
            GridParams {
                x0: g.x0 as f32,
                dx: g.dx as f32,
                nx: g.nx as u32,
                ny: g.na as u32,
                ..Default::default()
            },
        )?;
        gpu_input::rotary_from_keys(stock, &before, &keys);
        Ok(rapid_hits(tps, &hits, fourcam::sim::segments))
    }

    /// Cut `tps` into a block heightmap (3-axis).
    pub fn simulate_block(
        &self,
        hm: &mut Heightmap3,
        tps: &[Toolpath],
        cutter_for: &dyn Fn(u32) -> Option<Cutter>,
        step: f64,
    ) -> Result<Vec<usize>, String> {
        let input: GpuInput = gpu_input::block(hm, tps, cutter_for, step)?;
        let before = gpu_input::block_keys(hm, input.zbase);
        let (keys, hits) = self.stamp(
            &before,
            &input,
            0..input.seg_rapid.len(),
            GridParams {
                x0: hm.x0 as f32,
                dx: hm.cell as f32,
                y0: hm.y0 as f32,
                zbase: input.zbase as f32,
                nx: hm.nx as u32,
                ny: hm.ny as u32,
                ..Default::default()
            },
        )?;
        gpu_input::block_from_keys(hm, input.zbase, &before, &keys);
        Ok(rapid_hits(tps, &hits, fourcam::sim3::segments3))
    }
}

/// Move indices of the rapids that cut, in segment order.
fn rapid_hits(
    tps: &[Toolpath],
    hits: &[bool],
    segs: fn(&[Toolpath]) -> Vec<fourcam::sim::Segment>,
) -> Vec<usize> {
    segs(tps)
        .iter()
        .zip(hits)
        .filter(|(_, &h)| h)
        .map(|(s, _)| s.move_index)
        .collect()
}

/// Which simulation ran.
#[derive(Debug, Clone, PartialEq)]
pub enum Backend {
    Gpu(String),
    Cpu(String),
}

/// Simulate a rotary job on the GPU when one is given and the job suits it, else on the
/// CPU (threads).
pub fn simulate_or_cpu(
    gpu: Option<&Gpu>,
    stock: &mut RadialMap,
    tps: &[Toolpath],
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    step: f64,
) -> Backend {
    let why = match gpu {
        Some(g) => {
            let mut trial = stock.clone();
            match g.simulate(&mut trial, tps, cutter_for, step) {
                Ok(_) => {
                    *stock = trial;
                    return Backend::Gpu(g.name.clone());
                }
                Err(e) => e,
            }
        }
        None => "no GPU".into(),
    };
    for tp in tps {
        sim::simulate(stock, tp, cutter_for, step);
    }
    Backend::Cpu(why)
}

/// The same for a block (3-axis).
pub fn simulate_block_or_cpu(
    gpu: Option<&Gpu>,
    hm: &mut Heightmap3,
    tps: &[Toolpath],
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    step: f64,
) -> Backend {
    let why = match gpu {
        Some(g) => {
            let mut trial = hm.clone();
            match g.simulate_block(&mut trial, tps, cutter_for, step) {
                Ok(_) => {
                    *hm = trial;
                    return Backend::Gpu(g.name.clone());
                }
                Err(e) => e,
            }
        }
        None => "no GPU".into(),
    };
    for tp in tps {
        sim3::simulate3(hm, tp, cutter_for, step);
    }
    Backend::Cpu(why)
}
