//! Simulation resolution: finer grids measure known features at least as well, and
//! playback equals the batch simulation at every preset.

use fourcam::dropcutter::Cutter;
use fourcam::job::StockBox;
use fourcam::sim::{self, Resolution};
use fourcam::sim3::{simulate3, Heightmap3, Playback3};
use fourcam::toolpath::Builder;
use fourcam::Toolpath;

fn tp(b: Builder) -> Toolpath {
    Toolpath {
        name: "t".into(),
        title: "t".into(),
        tool: None,
        notes: vec![],
        reset_a: false,
        safe_r: 15.0,
        moves: b.moves,
        origin_comment: None,
        plunge_limit: None,
    }
}

const VBIT: Cutter = Cutter::Cone {
    radius: 1.2,
    half_angle: std::f64::consts::FRAC_PI_4,
};

/// A V-groove 0.3 deep round a R12 bar at X -10.03 (between cells at every preset), from
/// A -30 to +30; the deepest simulated radius near the top, off by how much.
fn groove_error(res: &Resolution) -> f64 {
    let g = sim::Grid::new(-20.0, 0.0, res.dx, res.na);
    let mut st = sim::RadialMap::filled(g, 12.0);
    let mut b = Builder::new();
    b.tool_change(2, 10000);
    b.rapid(Some(-10.03), Some(15.0), Some(-30.0));
    b.plunge(11.7, 60.0);
    b.cut(-10.03, 11.7, 30.0, 300.0, 60.0);
    sim::simulate(&mut st, &tp(b), &|_| Some(VBIT), res.dx.min(0.01));
    let i = ((-10.03 - g.x0) / g.dx) as usize;
    let deepest = (i.saturating_sub(2)..=i + 2)
        .flat_map(|i| (0..g.na).map(move |j| (i, j)))
        .map(|(i, j)| st.at(i, j))
        .fold(f64::INFINITY, f64::min);
    (deepest - 11.7).abs()
}

#[test]
fn finer_rotary_grids_find_the_groove_bottom_better() {
    // the cell centres sit up to half a cell beside the groove: on a 90 degree V the depth
    // the grid sees is off by at most dx / 2, a bound that shrinks with each finer preset
    let mut bounds = Vec::new();
    for n in ["coarse", "normal", "fine", "ultra"] {
        let r = Resolution::preset(n).unwrap();
        let e = groove_error(&r);
        assert!(e <= r.dx / 2.0 + 1e-3, "{n}: {e} > {}", r.dx / 2.0);
        bounds.push((r.dx, e));
    }
    assert!(bounds[3].1 < bounds[0].1 / 4.0, "{bounds:?}");
}

/// A 10 x 2 mm V-carve-like slot: a flat 2 mm wide channel cut with a 2 mm flat end mill
/// along Y = 5, on a 20 x 10 block; the width the grid sees across X = 7, off by how much.
fn slot_width_error(cell: f64) -> f64 {
    let stock = StockBox {
        min: [0.0, 0.0, -5.0],
        max: [20.0, 10.0, 0.0],
    };
    let mut hm = Heightmap3::new(&stock, cell);
    let mut b = Builder::new();
    b.tool_change(1, 10000);
    b.rapid3(None, None, Some(3.0));
    b.rapid3(Some(2.0), Some(5.03), None);
    b.line3(2.0, 5.03, -1.0, [0.0, 0.0, 1.0], 60.0);
    b.line3(12.0, 5.03, -1.0, [0.0, 0.0, 1.0], 300.0);
    b.rapid3(None, None, Some(3.0));
    let st = simulate3(
        &mut hm,
        &tp(b),
        &|_| Some(Cutter::Flat { radius: 1.0 }),
        0.01,
    );
    assert!(st.rapid_hits.is_empty());
    let i = (7.0 / cell) as usize;
    let cut = (0..hm.ny).filter(|&j| hm.z[j * hm.nx + i] < -0.5).count();
    (cut as f64 * cell - 2.0).abs()
}

#[test]
fn finer_block_grids_measure_widths_better() {
    let errs: Vec<f64> = [0.2, 0.1, 0.05, 0.025]
        .iter()
        .map(|&c| slot_width_error(c))
        .collect();
    assert!(errs[3] <= errs[0] && errs[3] <= 0.05, "{errs:?}");
    // never worse than one cell
    for (e, c) in errs.iter().zip([0.2, 0.1, 0.05, 0.025]) {
        assert!(*e <= c + 1e-9, "{errs:?}");
    }
}

#[test]
fn playback_equals_batch_at_every_preset() {
    // a block: a pocket pass and a drill, played to the end and back to the middle
    let stock = StockBox {
        min: [0.0, 0.0, -5.0],
        max: [24.0, 12.0, 0.0],
    };
    let mut b = Builder::new();
    b.tool_change(1, 10000);
    b.operation(0, "pocket");
    b.rapid3(None, None, Some(3.0));
    b.rapid3(Some(3.0), Some(4.0), None);
    b.line3(3.0, 4.0, -1.0, [0.0, 0.0, 1.0], 60.0);
    for k in 0..6 {
        let x = if k % 2 == 0 { 20.0 } else { 3.0 };
        b.line3(x, 4.0 + k as f64 * 0.7, -1.0, [0.0, 0.0, 1.0], 300.0);
        b.line3(x, 4.0 + (k + 1) as f64 * 0.7, -1.0, [0.0, 0.0, 1.0], 300.0);
    }
    b.rapid3(None, None, Some(3.0));
    b.operation(1, "drill");
    b.rapid3(Some(12.0), Some(9.0), None);
    b.line3(12.0, 9.0, -3.0, [0.0, 0.0, 1.0], 60.0);
    b.rapid3(None, None, Some(3.0));
    let t = tp(b);
    let cutter = |_| Some(Cutter::Flat { radius: 1.5 });
    for name in ["coarse", "normal", "fine", "ultra"] {
        let cell = Resolution::preset(name).unwrap().cell_for(&stock);
        let mut batch = Heightmap3::new(&stock, cell);
        simulate3(&mut batch, &t, &cutter, (cell * 0.5).min(0.1));
        let mut pb = Playback3::new(
            Heightmap3::new(&stock, cell),
            std::slice::from_ref(&t),
            &cutter,
            (cell * 0.5).min(0.1),
        );
        let n = pb.len();
        pb.seek(n);
        assert_eq!(pb.stock.z, batch.z, "{name}");
        pb.seek(n / 2);
        pb.seek(n);
        assert_eq!(pb.stock.z, batch.z, "{name} after going back");
        assert_eq!(pb.stock.op, batch.op, "{name}");
    }
    // rotary too, at the coarse and fine presets
    for name in ["coarse", "fine"] {
        let r = Resolution::preset(name).unwrap();
        let g = sim::Grid::new(-20.0, 0.0, r.dx, r.na);
        let mut b = Builder::new();
        b.tool_change(2, 10000);
        b.rapid(Some(-10.0), Some(15.0), Some(-30.0));
        b.plunge(11.7, 60.0);
        b.cut(-10.0, 11.7, 30.0, 300.0, 60.0);
        b.cut(-5.0, 11.5, 90.0, 300.0, 60.0);
        b.rapid_r(15.0);
        let t = tp(b);
        let mut batch = sim::RadialMap::filled(g, 12.0);
        sim::simulate(&mut batch, &t, &|_| Some(VBIT), 0.05);
        let mut pb = sim::Playback::new(
            sim::RadialMap::filled(g, 12.0),
            std::slice::from_ref(&t),
            &|_| Some(VBIT),
            0.05,
        );
        let n = pb.len();
        pb.seek(n);
        assert_eq!(pb.stock.r, batch.r, "{name}");
    }
}

#[test]
fn playback_memory_stays_bounded_on_fine_grids() {
    // 2M cells x 6 bytes: snapshots spread out so they stay near 256 MB in all
    let every = sim::checkpoint_spacing(1_000_000, 2_000_000 * 6);
    assert!(every * 22 >= 1_000_000 && every >= 1000, "{every}");
    assert_eq!(sim::checkpoint_spacing(30_000, 100_000 * 6), 1000);
}
