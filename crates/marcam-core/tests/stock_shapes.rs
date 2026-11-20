//! Bars that are not round on the rotary axis (Mar: "the full range of stock"): square,
//! rectangular and hexagonal sections. Their corners set the clearances and where roughing
//! starts; the simulation starts from the section itself (radius per angle); the project
//! reads and writes them.

use marcam::job::{Section, Stock};
use marcam::project::{Project, StockDef};
use marcam::{plan_with_mesh, simulate_job};
use std::f64::consts::PI;

const KNOB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/knob/knob_indexed.marcam.toml"
);

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn a_section_s_radius_at_each_angle() {
    // a square 20 across, a flat on top: 10 at the flats, 10√2 at the corners
    let sq = Section::Polygon {
        sides: 4,
        across_flats: 20.0,
        angle: 0.0,
    };
    assert!(close(sq.radius_at(PI / 2.0), 10.0) && close(sq.radius_at(0.0), 10.0));
    assert!(close(sq.radius_at(PI / 4.0), 10.0 * 2f64.sqrt()));
    assert!(close(sq.corner_radius(), 10.0 * 2f64.sqrt()));
    // a hexagon 20 across the flats: corners at 20/√3, every 60° from 30° off the top flat
    let hex = Section::Polygon {
        sides: 6,
        across_flats: 20.0,
        angle: 0.0,
    };
    assert!(close(hex.radius_at(PI / 2.0), 10.0));
    assert!(close(
        hex.radius_at(PI / 2.0 + PI / 6.0),
        20.0 / 3f64.sqrt()
    ));
    // turned by 30°: a corner on top
    let turned = Section::Polygon {
        sides: 6,
        across_flats: 20.0,
        angle: 30.0,
    };
    assert!(close(turned.radius_at(PI / 2.0), 20.0 / 3f64.sqrt()));
    // a rectangle 30 wide (Y) by 10 high (Z)
    let r = Section::Rect {
        width: 30.0,
        height: 10.0,
        angle: 0.0,
    };
    assert!(close(r.radius_at(0.0), 15.0) && close(r.radius_at(PI / 2.0), 5.0));
    assert!(close(r.corner_radius(), 15f64.hypot(5.0)));
    // every angle: never past the corners, never inside the flats
    for k in 0..360 {
        let phi = k as f64 * PI / 180.0;
        for s in [sq, hex, turned, r] {
            let rr = s.radius_at(phi);
            assert!(rr <= s.corner_radius() + 1e-9, "{s:?} at {k}°: {rr}");
        }
        assert!(hex.radius_at(phi) >= 10.0 - 1e-9);
    }
}

/// The knob on a bar of `stock`.
fn knob_on(stock: StockDef) -> Project {
    let (mut p, _) = Project::from_json_any(&std::fs::read_to_string(KNOB).unwrap()).unwrap();
    p.setups[0].stock = stock;
    // (a bar's corners are deeper than a round bar's: longer flutes)
    for t in &mut p.tools {
        if let marcam::project::ToolEntry::Inline(t) = t {
            t.flute_length = Some(t.flute_length.unwrap_or(0.0).max(25.0));
        }
    }
    p
}

#[test]
fn a_hex_bar_is_planned_from_its_corners_and_simulated_from_its_flats() {
    // 21 across the flats: corners at 12.12, flats at 10.5 (the round knob's bar)
    let p = knob_on(StockDef::Hex {
        across_flats: 21.0,
        length: 40.0,
        angle: 0.0,
    });
    let libs = marcam::profiles::builtin_libraries();
    let job = p.setup_job(&p.setups[0], &libs).unwrap();
    let corner = 21.0 / 3f64.sqrt();
    assert!(close(job.stock.radius, corner), "{}", job.stock.radius);
    assert!(matches!(
        job.stock.section,
        Some(Section::Polygon { sides: 6, .. })
    ));
    let tps = plan_with_mesh(&job, None).unwrap();
    // the G-code's rapids are at the safe radius, not a rounding below it
    let machine = marcam::profiles::builtin_machines()
        .into_iter()
        .find(|m| m.id == "makera-z1-4axis")
        .unwrap()
        .post;
    for tp in &tps {
        let issues = marcam::verify_gcode(
            &marcam::post(tp, &machine),
            &marcam::limits_for(tp, &machine),
        );
        assert!(
            issues.is_empty(),
            "{}: {:?}",
            tp.name,
            issues.iter().map(|i| i.to_string()).collect::<Vec<_>>()
        );
    }
    // the clearance is above the corners
    assert!(
        tps.iter().all(|tp| tp.safe_r > corner),
        "{:?}",
        tps.iter().map(|t| t.safe_r).collect::<Vec<_>>()
    );
    // the simulation starts from the hexagon: at the flats 10.5, at the corners 12.12
    let res = simulate_job(&job, &[], None, 0.5, 360, 0.02).unwrap();
    let start = marcam::sim::RadialMap::of_stock(res.stock.grid, &job.stock);
    let na = res.stock.grid.na;
    let ring = &start.r[..na];
    let (lo, hi) = ring
        .iter()
        .fold((f32::MAX, 0f32), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(
        (lo as f64 - 10.5).abs() < 0.01 && (hi as f64 - corner).abs() < 0.1,
        "{lo} .. {hi}"
    );
    // cut: the part's surface is reached all round, nothing of the part taken
    let cut = simulate_job(&job, &tps, None, 0.5, 360, 0.02).unwrap();
    for (name, _st, dev) in &cut.programs {
        assert!(dev.max_gouge < 0.05, "{name}: gouge {}", dev.max_gouge);
    }
}

#[test]
fn square_and_rectangular_bars_plan_and_simulate() {
    let libs = marcam::profiles::builtin_libraries();
    for stock in [
        StockDef::Square {
            side: 21.0,
            length: 40.0,
            angle: 0.0,
        },
        StockDef::Rect {
            width: 24.0,
            height: 21.0,
            length: 40.0,
            angle: 0.0,
        },
    ] {
        let p = knob_on(stock.clone());
        let job = p.setup_job(&p.setups[0], &libs).unwrap();
        let tps = plan_with_mesh(&job, None).unwrap();
        assert!(
            tps.iter().all(|tp| tp.safe_r > job.stock.radius),
            "{stock:?}"
        );
        let cut = simulate_job(&job, &tps, None, 0.5, 360, 0.02).unwrap();
        for (name, _st, dev) in &cut.programs {
            assert!(
                dev.max_gouge < 0.05,
                "{stock:?} {name}: gouge {}",
                dev.max_gouge
            );
        }
    }
}

#[test]
fn the_shapes_round_trip_through_the_project_file() {
    for stock in [
        StockDef::Square {
            side: 19.05,
            length: 50.0,
            angle: 0.0,
        },
        StockDef::Rect {
            width: 25.4,
            height: 12.7,
            length: 60.0,
            angle: 15.0,
        },
        StockDef::Hex {
            across_flats: 22.0,
            length: 45.0,
            angle: 30.0,
        },
    ] {
        let p = knob_on(stock.clone());
        let text = marcam::project_file::project_to_toml(&p).unwrap();
        let (back, _) = Project::from_json_any(&text).unwrap();
        assert_eq!(back.setups[0].stock, stock, "{text}");
        // and the job's bar has the section's corners
        let job = back
            .setup_job(&back.setups[0], &marcam::profiles::builtin_libraries())
            .unwrap();
        let want: Stock = stock.cylinder();
        assert!(close(job.stock.radius, want.radius));
    }
}

#[test]
fn a_bar_lying_in_a_vise_is_a_block_round_it_with_its_top_from_the_section() {
    // a round bar Ø20 x 50 lying along X: the block 50 x 20 x 20, Z0 at its top, the axis at
    // (10, -10)
    let round = Stock {
        radius: 10.0,
        length: 50.0,
        section: None,
    };
    let (sb, axis) = round.lying();
    assert!(
        close(sb.max[0], 50.0)
            && (sb.max[1] - 20.0).abs() < 1e-3
            && (sb.min[2] + 20.0).abs() < 1e-3
    );
    assert!(
        (axis[0] - 10.0).abs() < 1e-3 && (axis[1] + 10.0).abs() < 1e-3,
        "{axis:?}"
    );
    // a hex 20 across flats, a flat on top: 20 high, 23.09 wide (corners at the sides)
    let hex = Stock::of_section(
        Section::Polygon {
            sides: 6,
            across_flats: 20.0,
            angle: 0.0,
        },
        50.0,
    );
    let (hb, hax) = hex.lying();
    assert!(
        (hb.min[2] + 20.0).abs() < 1e-3 && (hb.max[1] - 40.0 / 3f64.sqrt()).abs() < 1e-2,
        "{hb:?}"
    );
    assert!((hax[1] + 10.0).abs() < 1e-3);
    // the heightmap starts at the bar's top: 0 over the axis, lower towards the sides
    let mut job = marcam::Job {
        stock: round.clone(),
        stock_box: Some(sb),
        stock_axis: Some(axis),
        ..serde_json::from_str(r#"{"schema_version":"0.1","stock":{"radius":10,"length":50},"part":{},"tools":[],"programs":[],"spindle_rpm":10000,"clearance":3}"#).unwrap()
    };
    job.clearance = 3.0;
    let hm = marcam::sim3::Heightmap3::of_job(&job, &sb, 0.5);
    let row = |y: f64| hm.z[((y / 0.5) as usize) * hm.nx];
    assert!(row(10.0).abs() < 0.02, "over the axis: {}", row(10.0));
    // off the axis: the circle's top, sqrt(100 - dy²) - 10, at each row's centre
    for j in 0..hm.ny {
        let (_, y) = hm.centre(0, j);
        let dy = y - 10.0;
        if dy.abs() < 9.5 {
            let want = (100.0 - dy * dy).sqrt() - 10.0;
            assert!(
                (hm.z[j * hm.nx] - want).abs() < 0.05,
                "row {j} at {y}: {} vs {want}",
                hm.z[j * hm.nx]
            );
        }
    }
}
