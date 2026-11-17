//! Inputs for the GPU stock simulation (`web/gpu/stamp.wgsl`, used natively by the
//! `marcam-gpu` crate and in the browser through WebGPU). Kept here, without any GPU
//! dependency, so the WebAssembly build can hand the same data to the browser.
//!
//! The CPU decides where the tool is: every move is sampled exactly as the CPU simulation
//! samples it, and each tool position gets its window of cells in f64. The shader evaluates
//! the cut per (position, cell) in f32 and keeps the lowest height with `atomicMin` on a
//! key, `height_um << 8 | tag`. For blocks, heights are measured from `zbase`.
//!
//! Indexed 3+1 moves are not offered. Their stamp cuts a cell only when its surface point is
//! inside the tool, which depends on the order cuts arrive in, and concurrent GPU threads
//! cannot reproduce that order.

use crate::dropcutter::Cutter;
use crate::sim::{segments, Grid, RadialMap};
use crate::sim3::{segments3, Heightmap3};
use crate::Toolpath;
use std::f64::consts::{PI, TAU};

/// Words per position (16 x 4 bytes); the layout of `Pos` in the shader.
pub const POS_WORDS: usize = 16;

/// Mode word: rotary radial stamp, or block (3-axis) stamp.
pub const MODE_ROTARY: u32 = 0;
pub const MODE_BLOCK: u32 = 1;

/// Tool positions for the shader, with where each segment's positions start.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuInput {
    /// `POS_WORDS` words per position (f32 and u32/i32 bit patterns).
    pub words: Vec<u32>,
    /// For segment `s` of the job, its positions are `seg_start[s]..seg_start[s + 1]`.
    pub seg_start: Vec<u32>,
    /// For each program, its first segment (and the total at the end).
    pub prog_seg: Vec<u32>,
    /// Whether each segment is a rapid (its positions report cutting into material).
    pub seg_rapid: Vec<bool>,
    pub mode: u32,
    /// Block heights are keyed from here (mm).
    pub zbase: f64,
}

impl GpuInput {
    pub fn len(&self) -> usize {
        self.words.len() / POS_WORDS
    }

    /// Dispatches for segments `seg_from..seg_to`: runs of feed segments, and each rapid
    /// segment alone so it sees the stock the cuts before it left. Each entry is (first
    /// position, count), at most `max` positions.
    pub fn runs(&self, seg_from: usize, seg_to: usize, max: usize) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        let mut s = seg_from;
        while s < seg_to {
            let mut e = s + 1;
            if !self.seg_rapid[s] {
                while e < seg_to && !self.seg_rapid[e] {
                    e += 1;
                }
            }
            let (mut a, b) = (self.seg_start[s], self.seg_start[e]);
            while a < b {
                let n = (b - a).min(max as u32);
                out.push((a, n));
                a += n;
            }
            s = e;
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

fn kind_of(c: &Cutter) -> (u32, f64) {
    match *c {
        Cutter::Flat { .. } => (0, 0.0),
        Cutter::Ball { .. } => (1, 0.0),
        Cutter::Cone { half_angle, .. } => (2, 1.0 / half_angle.tan()),
        // refused before any position is written (see `cpu_only`)
        Cutter::TaperedBall { .. } => (u32::MAX, 0.0),
    }
}

/// Tools the GPU shader has no shape for: their programs are simulated on the CPU.
fn cpu_only(tps: &[Toolpath], cutter_for: &dyn Fn(u32) -> Option<Cutter>) -> Result<(), String> {
    let tapered = tps.iter().flat_map(|tp| &tp.moves).any(|m| {
        matches!(m, crate::Move::ToolChange { tool, .. }
            if matches!(cutter_for(*tool), Some(Cutter::TaperedBall { .. })))
    });
    if tapered {
        return Err("tapered ball tools are simulated on the CPU".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push(
    w: &mut Vec<u32>,
    p: [f64; 4],
    c: &Cutter,
    tag: u16,
    win: [i64; 4],
    mode: u32,
    seg: usize,
    rapid: bool,
) {
    let (kind, inv_tan) = kind_of(c);
    w.extend_from_slice(&[
        (p[0] as f32).to_bits(),
        (p[1] as f32).to_bits(),
        (p[2] as f32).to_bits(),
        (p[3] as f32).to_bits(),
        (c.radius() as f32).to_bits(),
        kind,
        (inv_tan as f32).to_bits(),
        tag as u32,
        win[0] as u32,
        win[1] as u32,
        win[2] as i32 as u32,
        win[3] as i32 as u32,
        mode,
        seg as u32,
        u32::from(rapid),
        0,
    ]);
}

/// Positions of a rotary job on `grid` (radial stamp), sampled as `sim::apply_segment` does.
pub fn rotary(
    grid: Grid,
    tps: &[Toolpath],
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    step: f64,
) -> Result<GpuInput, String> {
    cpu_only(tps, cutter_for)?;
    let g = grid;
    let dphi = TAU / g.na as f64;
    let mut words = Vec::new();
    let mut seg_start = Vec::new();
    let mut prog_seg = Vec::new();
    let mut seg_rapid = Vec::new();
    for (s, seg) in segments(tps).iter().enumerate() {
        while prog_seg.len() <= seg.program as usize {
            prog_seg.push(s as u32);
        }
        seg_start.push((words.len() / POS_WORDS) as u32);
        seg_rapid.push(seg.rapid);
        if seg.index_a.is_some() {
            return Err("indexed 3+1 moves are simulated on the CPU".into());
        }
        if seg.op > 255 {
            return Err("more than 255 operations".into());
        }
        let (Some(c), Some([x0, r0, a0])) = (seg.tool.and_then(cutter_for), seg.from) else {
            continue;
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
            push(
                &mut words,
                [x, r, phi_t, 0.0],
                &c,
                seg.op,
                [i0 as i64, i1 as i64, j0, j1],
                MODE_ROTARY,
                s,
                seg.rapid,
            );
        }
    }
    let nseg = seg_start.len() as u32;
    seg_start.push((words.len() / POS_WORDS) as u32);
    while prog_seg.len() < tps.len() {
        prog_seg.push(nseg);
    }
    prog_seg.push(nseg);
    Ok(GpuInput {
        words,
        seg_start,
        prog_seg,
        seg_rapid,
        mode: MODE_ROTARY,
        zbase: 0.0,
    })
}

/// Positions of a 3-axis job on a block heightmap, sampled as `sim3::apply_segment3` does.
pub fn block(
    hm: &Heightmap3,
    tps: &[Toolpath],
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    step: f64,
) -> Result<GpuInput, String> {
    cpu_only(tps, cutter_for)?;
    let mut words = Vec::new();
    let mut seg_start = Vec::new();
    let mut prog_seg = Vec::new();
    let mut seg_rapid = Vec::new();
    for (s, seg) in segments3(tps).iter().enumerate() {
        while prog_seg.len() <= seg.program as usize {
            prog_seg.push(s as u32);
        }
        seg_start.push((words.len() / POS_WORDS) as u32);
        seg_rapid.push(seg.rapid);
        if seg.op > 255 {
            return Err("more than 255 operations".into());
        }
        let (Some(c), Some(a)) = (seg.tool.and_then(cutter_for), seg.from) else {
            continue;
        };
        let r = c.radius();
        let b = seg.to;
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        let n = ((len / step).ceil() as usize).max(1);
        for k in 0..=n {
            let t = k as f64 / n as f64;
            let p = [0, 1, 2].map(|q| a[q] + (b[q] - a[q]) * t);
            // the cells sim3::Heightmap3::stamp visits
            let lo = |v: f64, o: f64| (((v - r - o) / hm.cell).floor()).max(0.0) as i64;
            let hi = |v: f64, o: f64, n: usize| {
                ((((v + r - o) / hm.cell).floor()) as i64).min(n as i64 - 1)
            };
            let (i0, j0) = (lo(p[0], hm.x0), lo(p[1], hm.y0));
            let (i1, j1) = (hi(p[0], hm.x0, hm.nx), hi(p[1], hm.y0, hm.ny));
            if i1 < i0 || j1 < j0 {
                continue;
            }
            push(
                &mut words,
                [p[0], p[1], p[2], 0.0],
                &c,
                seg.op,
                [i0, i1, j0, j1],
                MODE_BLOCK,
                s,
                seg.rapid,
            );
        }
    }
    let nseg = seg_start.len() as u32;
    seg_start.push((words.len() / POS_WORDS) as u32);
    while prog_seg.len() < tps.len() {
        prog_seg.push(nseg);
    }
    prog_seg.push(nseg);
    Ok(GpuInput {
        words,
        seg_start,
        prog_seg,
        seg_rapid,
        mode: MODE_BLOCK,
        zbase: hm.bottom - 1.0,
    })
}

/// Initial keys for a rotary stock: radius in µm << 8 | tag.
pub fn rotary_keys(stock: &RadialMap) -> Vec<u32> {
    stock
        .r
        .iter()
        .zip(&stock.op)
        .map(|(&r, &op)| (((r.max(0.0) * 1000.0) as u32) << 8) | (op as u32 & 255))
        .collect()
}

/// Initial keys for a block: height above `zbase` in µm << 8 | tag.
pub fn block_keys(hm: &Heightmap3, zbase: f64) -> Vec<u32> {
    hm.z.iter()
        .zip(&hm.op)
        .map(|(&z, &op)| ((((z - zbase).max(0.0) * 1000.0) as u32) << 8) | (op as u32 & 255))
        .collect()
}

/// Write keys back into a rotary stock (cells the GPU lowered).
pub fn rotary_from_keys(stock: &mut RadialMap, before: &[u32], keys: &[u32]) {
    for (k, (&key, &old)) in keys.iter().zip(before).enumerate() {
        if key >> 8 < old >> 8 {
            stock.r[k] = (key >> 8) as f32 / 1000.0;
            stock.op[k] = (key & 255) as u16;
        }
    }
}

/// Write keys back into a block.
pub fn block_from_keys(hm: &mut Heightmap3, zbase: f64, before: &[u32], keys: &[u32]) {
    for (k, (&key, &old)) in keys.iter().zip(before).enumerate() {
        if key >> 8 < old >> 8 {
            hm.z[k] = zbase + (key >> 8) as f64 / 1000.0;
            hm.op[k] = (key & 255) as u16;
        }
    }
}
