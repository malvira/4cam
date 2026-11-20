//! The vise's jaws: the depth of the stock they grip (a default from the stock, or given),
//! and a warning when a cut reaches them (past the stock's Y faces below their top).

use marcam::project::{Project, StockDef};
use marcam::{jaw_check, load_mesh, plan_with_mesh};
use std::path::Path;

const PLATE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/plate");

fn plate() -> Project {
    let text = std::fs::read_to_string(Path::new(PLATE).join("plate.marcam.toml")).unwrap();
    Project::from_json_any(&text).unwrap().0
}

fn warnings(p: &Project) -> Vec<String> {
    let libs = marcam::profiles::builtin_libraries();
    let job = p.setup_job(&p.setups[0], &libs).unwrap();
    let mesh = load_mesh(&job, Path::new(PLATE)).unwrap();
    let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
    jaw_check(&job, &tps).into_iter().map(|(_, w)| w).collect()
}

#[test]
fn the_jaws_grip_3_mm_of_a_block_by_default_and_a_bar_s_40_percent() {
    let libs = marcam::profiles::builtin_libraries();
    // the plate's block is 10 high: 3 mm (a third would be 3.33)
    let p = plate();
    let job = p.setup_job(&p.setups[0], &libs).unwrap();
    assert_eq!(job.jaws, Some(3.0));
    // a thin plate, 6 high: a third, 2 mm
    let mut thin = plate();
    thin.setups[0].stock = StockDef::Box {
        min: [0.0, 0.0, -6.0],
        max: [60.0, 40.0, 0.0],
    };
    assert_eq!(
        thin.setup_job(&thin.setups[0], &libs).unwrap().jaws,
        Some(2.0)
    );
    // given
    let mut given = plate();
    given.setups[0].holding.jaws = Some(5.0);
    assert_eq!(
        given.setup_job(&given.setups[0], &libs).unwrap().jaws,
        Some(5.0)
    );
    // a round bar Ø20 lying in the vise: 8 mm
    let mut bar = plate();
    bar.setups[0].stock = StockDef::Cylinder {
        radius: 10.0,
        length: 60.0,
    };
    let job = bar.setup_job(&bar.setups[0], &libs).unwrap();
    assert!((job.jaws.unwrap() - 8.0).abs() < 1e-3, "{:?}", job.jaws);
}

#[test]
fn a_cut_below_the_jaws_top_is_warned_about() {
    // the plate as it is: nothing reaches past its Y faces (its holes go deep, inside it)
    assert!(warnings(&plate()).is_empty(), "{:?}", warnings(&plate()));
    // gripped 8.5 of 10 (their top at Z-1.5): the outline's tool runs 1 mm past the Y faces
    // down to Z-2.5, into the jaws
    let mut high = plate();
    high.setups[0].holding.jaws = Some(8.5);
    let w = warnings(&high);
    assert!(
        w.len() == 1 && w[0].contains("contour") && w[0].contains("cuts into the vise jaws"),
        "{w:?}"
    );
    // the outline run round the whole block's edge: past the Y faces, into the jaws
    let mut edge = plate();
    edge.setups[0].holding.jaws = Some(9.0);
    for o in &mut edge.setups[0].operations {
        if let marcam::job::Operation::Contour(c) = &mut o.op {
            c.path = vec![[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]];
        }
    }
    let w = warnings(&edge);
    assert!(
        w.iter().any(|x| x.contains("cuts into the vise jaws")),
        "{w:?}"
    );
}

#[test]
fn a_shank_that_reaches_the_jaws_is_a_collision() {
    let libs = marcam::profiles::builtin_libraries();
    let machine = marcam::profiles::builtin_machine("makera-carvera-4axis")
        .unwrap()
        .post;
    // the outline's tool with 1 mm of flute under a 6 mm shank, its tip 1 mm past the Y
    // faces down to Z-2.5
    let check = |grip: f64| {
        let mut p = plate();
        p.setups[0].holding.jaws = Some(grip);
        let mut job = p.setup_job(&p.setups[0], &libs).unwrap();
        // a tool of its own for the outline (the pocket's shank would collide with the stock)
        let t = job
            .programs
            .iter()
            .flat_map(|pr| &pr.operations)
            .find_map(|o| match o {
                marcam::job::Operation::Contour(c) => Some(c.tool),
                _ => None,
            })
            .unwrap();
        let mut own = job.tool(t).unwrap().clone();
        own.number = 9;
        own.flute_length = Some(1.0);
        own.shank_diameter = Some(6.0);
        job.tools.push(own);
        for pr in &mut job.programs {
            for o in &mut pr.operations {
                if let marcam::job::Operation::Contour(c) = o {
                    c.tool = 9;
                }
            }
        }
        let mesh = load_mesh(&job, Path::new(PLATE)).unwrap();
        let tps = plan_with_mesh(&job, mesh.as_ref()).unwrap();
        marcam::collision_check(&job, &tps, &machine)
            .unwrap()
            .iter()
            .flat_map(|r| r.collisions.clone())
            .collect::<Vec<_>>()
    };
    // gripped 9.8 of 10 (their top Z-0.2): the shank (from Z-1.5 up) beside the block
    let hit = check(9.8);
    assert!(
        hit.iter()
            .any(|c| c.contact == marcam::collision::Contact::ShankOnJaws && c.tip[1] < 2.0),
        "{hit:?}"
    );
    // gripped 3 mm: the jaws far below
    let low = check(3.0);
    assert!(
        low.iter()
            .all(|c| c.contact != marcam::collision::Contact::ShankOnJaws),
        "{low:?}"
    );
}
