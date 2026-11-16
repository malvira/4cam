//! Material removal for 3-axis programs: the block as a Z heightmap (one top per XY cell).
//!
//! Each feed move sweeps the cutter's surface over the cells it passes; a rapid that would
//! lower any cell is reported (it cut material at rapid speed).

use crate::dropcutter::Cutter;
use crate::job::StockBox;
use crate::{Move, Toolpath};

#[derive(Debug, Clone)]
pub struct Heightmap3 {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    /// Top Z per cell, row-major (index j * nx + i).
    pub z: Vec<f64>,
}

impl Heightmap3 {
    pub fn new(stock: &StockBox, cell: f64) -> Heightmap3 {
        let nx = (((stock.max[0] - stock.min[0]) / cell).ceil() as usize).max(1);
        let ny = (((stock.max[1] - stock.min[1]) / cell).ceil() as usize).max(1);
        Heightmap3 {
            x0: stock.min[0],
            y0: stock.min[1],
            cell,
            nx,
            ny,
            z: vec![stock.max[2]; nx * ny],
        }
    }

    /// Top of the cell containing (x, y) (clamped to the block).
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let i = ((x - self.x0) / self.cell).max(0.0) as usize;
        let j = ((y - self.y0) / self.cell).max(0.0) as usize;
        self.z[j.min(self.ny - 1) * self.nx + i.min(self.nx - 1)]
    }

    /// Centre of cell (i, j).
    pub fn centre(&self, i: usize, j: usize) -> (f64, f64) {
        (
            self.x0 + (i as f64 + 0.5) * self.cell,
            self.y0 + (j as f64 + 0.5) * self.cell,
        )
    }

    /// Lower every cell under the cutter with its tip at (x, y, z); returns cells changed.
    pub fn stamp(&mut self, c: &Cutter, x: f64, y: f64, z: f64) -> usize {
        let r = c.radius();
        let lim = r * (1.0 - 1e-9);
        let lo = |v: f64, o: f64| (((v - r - o) / self.cell).floor()).max(0.0) as usize;
        let hi = |v: f64, o: f64, n: usize| {
            ((((v + r - o) / self.cell).floor()) as isize).min(n as isize - 1)
        };
        let (i0, j0) = (lo(x, self.x0), lo(y, self.y0));
        let (i1, j1) = (hi(x, self.x0, self.nx), hi(y, self.y0, self.ny));
        if i1 < 0 || j1 < 0 {
            return 0;
        }
        let mut n = 0;
        for j in j0..=j1 as usize {
            for i in i0..=i1 as usize {
                let (cx, cy) = self.centre(i, j);
                let d = (cx - x).hypot(cy - y);
                if d >= lim {
                    continue;
                }
                let zt = z + c.height(d);
                let k = j * self.nx + i;
                if zt < self.z[k] - 1e-6 {
                    self.z[k] = zt;
                    n += 1;
                }
            }
        }
        n
    }

    /// Cut a 3-axis toolpath, sampling each move every `step` mm. Returns the indices (in
    /// `tp.moves`) of rapids that removed material (should be none).
    pub fn cut(
        &mut self,
        tp: &Toolpath,
        cutter_for: &dyn Fn(u32) -> Option<Cutter>,
        step: f64,
    ) -> Vec<usize> {
        let mut p: Option<[f64; 3]> = None;
        let mut c: Option<Cutter> = None;
        let mut rapid_hits = Vec::new();
        for (idx, m) in tp.moves.iter().enumerate() {
            let (to, rapid) = match *m {
                Move::ToolChange { tool, .. } => {
                    c = cutter_for(tool);
                    continue;
                }
                Move::Rapid3 { x, y, z } => {
                    let q = p.unwrap_or([f64::NAN, f64::NAN, f64::INFINITY]);
                    (
                        [x.unwrap_or(q[0]), y.unwrap_or(q[1]), z.unwrap_or(q[2])],
                        true,
                    )
                }
                Move::Line3 { x, y, z, .. } => ([x, y, z], false),
                _ => continue,
            };
            if let (Some(c), Some(a)) = (c.as_ref(), p) {
                if a.iter().chain(to.iter()).all(|v| v.is_finite()) {
                    let len =
                        ((to[0] - a[0]).powi(2) + (to[1] - a[1]).powi(2) + (to[2] - a[2]).powi(2))
                            .sqrt();
                    let n = ((len / step).ceil() as usize).max(1);
                    let mut hit = 0;
                    for k in 0..=n {
                        let t = k as f64 / n as f64;
                        hit += self.stamp(
                            c,
                            a[0] + (to[0] - a[0]) * t,
                            a[1] + (to[1] - a[1]) * t,
                            a[2] + (to[2] - a[2]) * t,
                        );
                    }
                    if rapid && hit > 0 {
                        rapid_hits.push(idx);
                    }
                }
            }
            p = Some(to);
        }
        rapid_hits
    }
}
