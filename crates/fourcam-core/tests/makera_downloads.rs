//! Runs against Makera's real Fusion 360 downloads when they are cached in
//! profiles/_downloads/Fusion-Profiles.zip (git-ignored; fetch it from
//! https://www.makera.com/pages/software -> Fusion 360). Skips otherwise.

use fourcam::profiles::{builtin_machine, import, Imported, LibShape};
use std::path::PathBuf;

#[test]
fn imports_makera_fusion_downloads_when_cached() {
    let zip = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../profiles/_downloads/Fusion-Profiles.zip");
    let Ok(bytes) = std::fs::read(&zip) else {
        eprintln!("skipped: {} not present", zip.display());
        return;
    };
    let entries = fourcam::archive::entries(&bytes).unwrap();
    let get = |name: &str| fourcam::archive::read(&bytes, name, &entries).unwrap();

    // Z1 4-axis machine definition (an .f3d with an embedded .mch, zstd-compressed)
    let imp = import(&get("Makera Z1 4 axis.f3d"), "Makera Z1 4 axis.f3d").unwrap();
    let Imported::Machine(m) = imp.item else {
        panic!("not a machine")
    };
    let ours = builtin_machine("makera-z1-4axis").unwrap();
    for l in ["X", "Y", "Z"] {
        let (a, b) = (
            m.kinematics.linear(l).unwrap(),
            ours.kinematics.linear(l).unwrap(),
        );
        assert_eq!((a.min, a.max), (b.min, b.max), "axis {l}");
    }
    let a = m.kinematics.rotary("A").unwrap();
    assert_eq!(a.axis, [1.0, 0.0, 0.0]);
    assert!(a.continuous);
    assert_eq!(m.spindle.unwrap().max_rpm, ours.spindle.unwrap().max_rpm);
    assert_eq!(m.max_feed, ours.max_feed);
    assert_eq!(m.post.feed_mode, ours.post.feed_mode);

    // Carvera and Carvera Air 4-axis: A about X, 6-tool changer, same feed mode
    for f in [
        "Makera Carvera 4-axis Harmonic Community Version 1.4.6.f3d",
        "Makera Carvera Air 4-axis Community Version 1.4.6.f3d",
    ] {
        let imp = import(&get(f), f).unwrap();
        let Imported::Machine(m) = imp.item else {
            panic!("{f}")
        };
        assert_eq!(
            m.kinematics.rotary("A").unwrap().axis,
            [1.0, 0.0, 0.0],
            "{f}"
        );
        assert_eq!(m.tool_changer.as_ref().unwrap().slots, Some(6), "{f}");
        assert_eq!(m.max_feed, Some(4000.0), "{f}");
        assert!(
            imp.notes.iter().any(|n| n.contains("no travel limits")),
            "{f}: {:?}",
            imp.notes
        );
    }

    // every tool library imports; the starter library's T1 is in the O-flute file
    let mut total = 0;
    for (name, _) in entries.iter().filter(|(n, _)| n.ends_with(".tools")) {
        let imp = import(&get(name), name).unwrap();
        let Imported::Tools(lib) = imp.item else {
            panic!("{name}")
        };
        total += lib.tools.len();
        if name.contains("O Flute") {
            assert!(lib
                .tools
                .iter()
                .any(|t| t.shape == LibShape::Flat { diameter: 3.175 }
                    && t.flute_length == Some(12.0)
                    && t.name.contains("Metal")));
        }
    }
    eprintln!("imported {total} tools from Makera's Fusion libraries");
    assert!(total > 100, "{total}");
}
