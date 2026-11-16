//! Tool holder and shank collisions: whether anything above the flutes (the shank up to the
//! stickout, then the holder) touches material that is still there when the tool passes.
//!
//! The tool stands along machine +Z. Its body above the tip is two cylinders: the shank
//! (`flute_length..stickout`, shank radius) and the holder (`stickout..stickout + holder
//! length`, holder radius). The flutes cut; the rest must not touch the stock. The check
//! runs inside the material-removal simulation: before each move is cut, every evaluated tool
//! position along it is tested against the stock as the earlier moves left it (the rotary
//! [`RadialMap`] or the 3-axis [`Heightmap3`]), and the deepest contact is reported per move.
//!
//! The rotary stock outside the simulated X range is taken as uncut bar (the stock radius
//! over the stock's length, `-length..0`). Chucks, tailstocks and vises are not modelled.

use crate::dropcutter::Cutter;
use crate::job::{Tool, ToolShape};
use crate::sim::{self, RadialMap, Segment, SimStats};
use crate::sim3::{self, Heightmap3};
use crate::Toolpath;
use serde::Serialize;
use std::f64::consts::{PI, TAU};

/// Holder (collet nut) diameter when a tool does not give one (mm).
pub const DEFAULT_HOLDER_DIAMETER: f64 = 20.0;
/// Holder length when a tool does not give one (mm).
pub const DEFAULT_HOLDER_LENGTH: f64 = 30.0;
/// Stickout when a tool does not give one: flute length plus this (mm).
pub const DEFAULT_STICKOUT_MARGIN: f64 = 5.0;
/// Flute length when a tool does not give one: this many cutting diameters.
pub const DEFAULT_FLUTE_DIAMETERS: f64 = 3.0;
/// Contacts shallower than this are not reported (mm): a cell's worth of rounding.
pub const CONTACT_TOL: f64 = 0.01;
/// Tool positions checked along a move are at most this far apart (mm), or the
/// simulation's step if that is coarser.
pub const CHECK_STEP: f64 = 0.5;
/// At most this many collisions are listed per program (all are counted).
pub const MAX_LISTED: usize = 50;

/// What of a tool is above its flutes, with every field filled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ToolBody {
    /// Cutting radius (the flutes').
    pub cut_radius: f64,
    pub flute_length: f64,
    pub shank_radius: f64,
    pub stickout: f64,
    pub holder_radius: f64,
    pub holder_length: f64,
}

fn cutting_diameter(shape: &ToolShape) -> f64 {
    match *shape {
        ToolShape::Flat { diameter } | ToolShape::Ball { diameter } => diameter,
        ToolShape::Vbit { diameter, .. } => diameter.unwrap_or(3.175),
    }
}

impl ToolBody {
    /// The tool's body, defaults filled: flute length 3 diameters, shank the cutting
    /// diameter, stickout flute + 5 mm, holder 20 mm across and 30 mm long.
    pub fn of(tool: &Tool) -> ToolBody {
        let d = cutting_diameter(&tool.shape);
        let flute = tool
            .flute_length
            .unwrap_or(DEFAULT_FLUTE_DIAMETERS * d)
            .max(0.0);
        let stickout = tool
            .stickout
            .unwrap_or(flute + DEFAULT_STICKOUT_MARGIN)
            .max(flute);
        ToolBody {
            cut_radius: d / 2.0,
            flute_length: flute,
            shank_radius: tool.shank_diameter.unwrap_or(d) / 2.0,
            stickout,
            holder_radius: tool.holder_diameter.unwrap_or(DEFAULT_HOLDER_DIAMETER) / 2.0,
            holder_length: tool.holder_length.unwrap_or(DEFAULT_HOLDER_LENGTH),
        }
    }

    /// The parts that must not touch: (which, from, to above the tip, radius).
    fn zones(&self) -> [(Contact, f64, f64, f64); 2] {
        [
            (
                Contact::Shank,
                self.flute_length,
                self.stickout,
                self.shank_radius,
            ),
            (
                Contact::Holder,
                self.stickout,
                self.stickout + self.holder_length,
                self.holder_radius,
            ),
        ]
    }

    /// Height above the tip below which nothing but the flutes is.
    fn lowest(&self) -> f64 {
        self.flute_length
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Contact {
    Shank,
    Holder,
}

impl std::fmt::Display for Contact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Contact::Shank => "shank",
            Contact::Holder => "holder",
        })
    }
}

/// One move where the shank or holder touches material.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Collision {
    /// Index of the move in its toolpath.
    pub move_index: usize,
    /// G-code line of the move (1-based), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub tool: u32,
    pub contact: Contact,
    /// How far into the material the part reaches (mm, along the tool axis).
    pub depth: f64,
    /// Tool tip there: machine [x, y, z], with the rotary axis at `a` (deg).
    pub tip: [f64; 3],
    pub a: f64,
    pub rapid: bool,
}

/// Collisions in one program.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CollisionReport {
    pub program: String,
    /// Moves with a collision (all of them, even past the listed ones).
    pub count: usize,
    /// Deepest contact (mm).
    pub max_depth: f64,
    /// The first [`MAX_LISTED`] collisions, in program order.
    pub collisions: Vec<Collision>,
}

impl CollisionReport {
    fn add(&mut self, c: Collision) {
        self.count += 1;
        self.max_depth = self.max_depth.max(c.depth);
        if self.collisions.len() < MAX_LISTED {
            self.collisions.push(c);
        }
    }
}

/// Deepest contact of the tool body with tip at machine `tip`, rotary axis at `a_deg`,
/// against a radial stock map. `bar` is the starting bar, also taken as uncut outside the
/// map: (radius, x from, x to).
pub fn contact_radial(
    stock: &RadialMap,
    bar: (f64, f64, f64),
    tip: [f64; 3],
    a_deg: f64,
    body: &ToolBody,
) -> Option<(Contact, f64)> {
    let g = stock.grid;
    let a = a_deg.to_radians();
    let mut best: Option<(Contact, f64)> = None;
    for (which, lo, hi, rad) in body.zones() {
        if rad <= 0.0 || hi <= lo {
            continue;
        }
        let z_lo = tip[2] + lo;
        let z_hi = tip[2] + hi;
        // the stock never grows: nothing is above the starting bar radius
        if z_lo >= bar.0 {
            continue;
        }
        let mut deepest = 0.0f64;
        // the highest material point inside the zone's cylinder, over the columns it spans
        let mut take = |zmax: f64| {
            if zmax > z_lo {
                deepest = deepest.max(zmax.min(z_hi) - z_lo);
            }
        };
        // columns inside the map
        let i0 = (((tip[0] - rad - g.x0) / g.dx - 0.5).ceil()).max(0.0) as usize;
        let i1 = ((tip[0] + rad - g.x0) / g.dx - 0.5).floor();
        // only rays pointing high enough can reach z_lo: sin(th) > z_lo / bar radius
        let span = TAU / g.na as f64;
        let th0 = if z_lo > 0.0 {
            (z_lo / bar.0).min(1.0).asin()
        } else {
            0.0
        };
        // part angles φ = th - a over th in (th0, π - th0), widened by a cell each side
        let j_lo = ((th0 - a) / span - 1.5).floor() as i64;
        let j_hi = ((PI - th0 - a) / span + 0.5).ceil() as i64;
        let na = g.na as i64;
        if i1 >= 0.0 {
            for i in i0..=(i1 as usize).min(g.nx.saturating_sub(1)) {
                let dx = g.x(i) - tip[0];
                let w = (rad * rad - dx * dx).max(0.0).sqrt();
                // The map holds only each ray's outer radius, so a hole the tool cut along
                // its own axis reads as solid. Above the tip, everything within the cutting
                // radius was cut by the flutes on the way down: the shank is checked only
                // outside it.
                let wi = if which == Contact::Shank {
                    (body.cut_radius.powi(2) - dx * dx).max(0.0).sqrt()
                } else {
                    0.0
                };
                let bands = if wi > 0.0 {
                    [(tip[1] - w, tip[1] - wi), (tip[1] + wi, tip[1] + w)]
                } else {
                    [(tip[1] - w, tip[1] + w), (1.0, 0.0)]
                };
                for jj in j_lo..=j_hi.min(j_lo + na - 1) {
                    let j = jj.rem_euclid(na) as usize;
                    let th = g.phi(j) + a;
                    let rho = stock.r[i * g.na + j] as f64;
                    for &(ylo, yhi) in &bands {
                        if let Some(z) = ray_top(rho, th, ylo, yhi) {
                            take(z);
                        }
                    }
                }
            }
        }
        // uncut bar beyond the map, sampled at the map's X spacing
        let (r0, bx0, bx1) = bar;
        let (map_lo, map_hi) = (g.x0, g.x0 + g.dx * g.nx as f64);
        let mut x = (tip[0] - rad).max(bx0);
        while x <= (tip[0] + rad).min(bx1) {
            if x < map_lo || x > map_hi {
                let dx = x - tip[0];
                let w = (rad * rad - dx * dx).max(0.0).sqrt();
                // the disc of radius r0 inside |y - ty| <= w: highest where |y| is least
                let y = if (tip[1] - w..=tip[1] + w).contains(&0.0) {
                    0.0
                } else {
                    (tip[1] - w).abs().min((tip[1] + w).abs())
                };
                if y < r0 {
                    take((r0 * r0 - y * y).sqrt());
                }
            }
            x += g.dx;
        }
        if deepest > CONTACT_TOL && best.is_none_or(|(_, d)| deepest > d) {
            best = Some((which, deepest));
        }
    }
    best
}

/// Highest Z of the material along a cell's ray (angle `th` in the machine Y/Z plane, from
/// the axis out to `rho`) with machine Y in `ylo..=yhi`.
fn ray_top(rho: f64, th: f64, ylo: f64, yhi: f64) -> Option<f64> {
    let (sn, c) = th.sin_cos();
    if rho <= 0.0 || sn <= 0.0 || yhi - ylo < 1e-9 {
        return None;
    }
    let (mut s0, mut s1) = (0.0f64, rho);
    if c.abs() < 1e-12 {
        if ylo > 0.0 || yhi < 0.0 {
            return None;
        }
    } else {
        let (u, v) = (ylo / c, yhi / c);
        s0 = s0.max(u.min(v));
        s1 = s1.min(u.max(v));
        if s0 > s1 {
            return None;
        }
    }
    Some(sn * s1)
}

/// Deepest contact of the tool body with tip at work `tip` against a 3-axis heightmap.
pub fn contact_block(
    hm: &Heightmap3,
    top: f64,
    tip: [f64; 3],
    body: &ToolBody,
) -> Option<(Contact, f64)> {
    let mut best: Option<(Contact, f64)> = None;
    for (which, lo, hi, rad) in body.zones() {
        if rad <= 0.0 || hi <= lo {
            continue;
        }
        let z_lo = tip[2] + lo;
        if z_lo >= top {
            continue;
        }
        let z_hi = tip[2] + hi;
        let c = hm.cell;
        let i0 = (((tip[0] - rad - hm.x0) / c - 0.5).ceil()).max(0.0) as usize;
        let i1 = ((tip[0] + rad - hm.x0) / c - 0.5).floor();
        let j0 = (((tip[1] - rad - hm.y0) / c - 0.5).ceil()).max(0.0) as usize;
        let j1 = ((tip[1] + rad - hm.y0) / c - 0.5).floor();
        if i1 < 0.0 || j1 < 0.0 {
            continue;
        }
        let mut deepest = 0.0f64;
        for j in j0..=(j1 as usize).min(hm.ny - 1) {
            for i in i0..=(i1 as usize).min(hm.nx - 1) {
                let (x, y) = hm.centre(i, j);
                if (x - tip[0]).powi(2) + (y - tip[1]).powi(2) > rad * rad {
                    continue;
                }
                let z = hm.z[j * hm.nx + i];
                if z > z_lo {
                    deepest = deepest.max(z.min(z_hi) - z_lo);
                }
            }
        }
        if deepest > CONTACT_TOL && best.is_none_or(|(_, d)| deepest > d) {
            best = Some((which, deepest));
        }
    }
    best
}

/// Tool tip positions evaluated along a segment (machine [x, y, z], A), at most `step` apart.
/// Rotary segments ([x, r, a]) put the tip at (x, 0, r).
fn samples(seg: &Segment, step: f64) -> Vec<([f64; 3], f64)> {
    let to = |p: [f64; 3]| match seg.index_a {
        Some(a) => (p, a),
        None => ([p[0], 0.0, p[1]], p[2]),
    };
    let (q, aq) = to(seg.to);
    let Some(from) = seg.from else {
        return vec![(q, aq)];
    };
    let (p, ap) = to(from);
    let arc = 0.5 * (p[2] + q[2]) * (aq - ap).to_radians().abs();
    let len = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2))
        .sqrt()
        .max(arc);
    let n = ((len / step).ceil() as usize).max(1);
    (0..=n)
        .map(|k| {
            let t = k as f64 / n as f64;
            (
                [0, 1, 2].map(|i| p[i] + (q[i] - p[i]) * t),
                ap + (aq - ap) * t,
            )
        })
        .collect()
}

/// Whether any move of `segs` brings a tool body low enough to reach stock that tops out at
/// `top` (the bar radius, or the block top): if none does, there is nothing to simulate.
pub fn can_touch(segs: &[Segment], body_for: &dyn Fn(u32) -> Option<ToolBody>, top: f64) -> bool {
    segs.iter().any(|s| {
        s.tool
            .and_then(body_for)
            .is_some_and(|b| lowest_tip(s) + b.lowest() < top)
    })
}

/// The lowest tip Z along a segment (machine), for skipping moves that cannot touch.
fn lowest_tip(seg: &Segment) -> f64 {
    let z = |p: [f64; 3]| if seg.index_a.is_some() { p[2] } else { p[1] };
    seg.from.map_or(z(seg.to), |f| z(f).min(z(seg.to)))
}

fn worst(
    seg: &Segment,
    step: f64,
    tool: u32,
    contact: &dyn Fn([f64; 3], f64) -> Option<(Contact, f64)>,
) -> Option<Collision> {
    let mut out: Option<Collision> = None;
    for (tip, a) in samples(seg, step.max(CHECK_STEP)) {
        if let Some((which, depth)) = contact(tip, a) {
            if out.as_ref().is_none_or(|c| depth > c.depth) {
                out = Some(Collision {
                    move_index: seg.move_index,
                    line: None,
                    tool,
                    contact: which,
                    depth,
                    tip,
                    a,
                    rapid: seg.rapid,
                });
            }
        }
    }
    out
}

/// [`sim::simulate`] that also checks the tool bodies: moves whose tip stays high enough that
/// nothing above the flutes can reach the stock (`bar.0`, the starting radius, bounds it) are
/// cut in bulk; the others are checked against the stock before they cut it. With
/// `check_only`, cutting stops after the last move that can touch (the stock is then not the
/// program's result: use it for the last program only).
#[allow(clippy::too_many_arguments)]
pub fn simulate_rotary(
    stock: &mut RadialMap,
    tp: &Toolpath,
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    body_for: &dyn Fn(u32) -> Option<ToolBody>,
    step: f64,
    bar: (f64, f64, f64),
    check_only: bool,
) -> (SimStats, CollisionReport) {
    let segs = sim::segments(std::slice::from_ref(tp));
    let cutters: Vec<Option<Cutter>> = segs.iter().map(|s| s.tool.and_then(cutter_for)).collect();
    let mut st = SimStats::default();
    let mut rep = CollisionReport {
        program: tp.name.clone(),
        ..Default::default()
    };
    let risky = |s: &Segment| -> Option<(u32, ToolBody)> {
        let t = s.tool?;
        let b = body_for(t)?;
        (lowest_tip(s) + b.lowest() < bar.0).then_some((t, b))
    };
    let threads = sim::sim_threads(stock.grid.nx);
    // nothing after the last move that can touch matters
    let end = if check_only {
        segs.iter()
            .rposition(|s| risky(s).is_some())
            .map_or(0, |e| e + 1)
    } else {
        segs.len()
    };
    let mut k = 0;
    while k < end {
        // a run of moves that cannot touch: cut together
        let mut e = k;
        while e < end && risky(&segs[e]).is_none() {
            e += 1;
        }
        if e > k {
            for (seg, eff) in segs[k..e].iter().zip(sim::apply_banded(
                stock,
                &segs[k..e],
                &cutters[k..e],
                step,
                threads,
            )) {
                tally(&mut st, seg, &eff);
            }
            k = e;
            continue;
        }
        let seg = &segs[k];
        if let Some((t, b)) = risky(seg) {
            let contact = |tip: [f64; 3], a: f64| contact_radial(stock, bar, tip, a, &b);
            if let Some(c) = worst(seg, step, t, &contact) {
                rep.add(c);
            }
        }
        let eff = sim::apply_segment(stock, seg, cutters[k].as_ref(), step);
        tally(&mut st, seg, &eff);
        k += 1;
    }
    (st, rep)
}

/// [`sim3::simulate3`] that also checks the tool bodies (see [`simulate_rotary`]); `top` is
/// the block's starting top.
pub fn simulate_block(
    hm: &mut Heightmap3,
    tp: &Toolpath,
    cutter_for: &dyn Fn(u32) -> Option<Cutter>,
    body_for: &dyn Fn(u32) -> Option<ToolBody>,
    step: f64,
    top: f64,
) -> (SimStats, CollisionReport) {
    let mut st = SimStats::default();
    let mut rep = CollisionReport {
        program: tp.name.clone(),
        ..Default::default()
    };
    for seg in sim3::segments3(std::slice::from_ref(tp)) {
        if let Some(t) = seg.tool {
            if let Some(b) = body_for(t).filter(|b| lowest_tip(&seg) + b.lowest() < top) {
                let contact = |tip: [f64; 3], _a: f64| contact_block(hm, top, tip, &b);
                if let Some(c) = worst(&seg, step, t, &contact) {
                    rep.add(c);
                }
            }
        }
        let c = seg.tool.and_then(cutter_for);
        let eff = sim3::apply_segment3(hm, &seg, c.as_ref(), step);
        tally(&mut st, &seg, &eff);
    }
    (st, rep)
}

fn tally(st: &mut SimStats, seg: &Segment, e: &sim::SegmentEffect) {
    st.samples += e.samples;
    st.cells_cut += e.cells_cut;
    if e.rapid_hit {
        st.rapid_hits.push(seg.move_index);
    }
}

/// Fill in each collision's G-code line from the post's per-move line map.
pub fn attach_lines(rep: &mut CollisionReport, lines: &[usize]) {
    for c in &mut rep.collisions {
        c.line = lines.get(c.move_index).copied();
    }
}
