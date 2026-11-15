//! Chamfer an edge with the flank of a V-bit (a flat endmill could only stair-step it).

use crate::job::FlankChamfer;
use crate::numeric::rad;
use crate::toolpath::Builder;

pub fn flank_chamfer(b: &mut Builder, vbit_angle: f64, safe_r: f64, op: &FlankChamfer) {
    let [edge_x, edge_r] = op.edge;
    b.comment("--- front chamfer with V-bit flank ---");
    // flank surface: r = r_tip + |x_tip - x| / tan(half angle), through the edge
    let k = 1.0 / rad(vbit_angle / 2.0).tan();
    let x_tip = edge_x + op.tip_offset;
    let n = (op.revolutions * 360.0 / op.segment_deg) as usize;
    for &extra in &op.passes {
        let r_tip = edge_r - k * (x_tip - edge_x).abs() + extra;
        b.rapid_r(safe_r);
        b.rapid_x(x_tip);
        b.plunge(r_tip, op.plunge_feed);
        for _ in 0..n {
            let a = b.a + op.segment_deg;
            b.cut(x_tip, r_tip, a, op.feed, op.plunge_feed);
        }
    }
    b.rapid_r(safe_r);
}
