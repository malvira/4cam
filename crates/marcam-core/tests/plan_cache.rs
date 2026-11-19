//! The per-operation plan cache: planning with it gives exactly the toolpaths planning from
//! scratch gives, and after an edit only the operations it touches are planned again.

use marcam::{plan_with_mesh, plan_with_mesh_cached, Job, Mesh, PlanCache};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every setup of every example, as jobs with their meshes.
fn examples() -> Vec<(String, Job, Option<Mesh>, PathBuf)> {
    let libs = marcam::profiles::builtin_libraries();
    let mut out = Vec::new();
    for dir in std::fs::read_dir(root().join("examples"))
        .unwrap()
        .flatten()
    {
        let d = dir.path();
        for f in std::fs::read_dir(&d).unwrap().flatten() {
            let p = f.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if !(name.ends_with(".marcam.toml") || name.ends_with(".job.json")) {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let jobs = marcam::project::jobs_from_any_with(&text, &libs, &|src| src.load(&d).ok())
                .unwrap();
            for (id, job) in jobs {
                let mesh = marcam::load_mesh(&job, &d).unwrap();
                out.push((format!("{name}/{id}"), job, mesh, d.clone()));
            }
        }
    }
    out
}

fn with(job: &Job, f: impl FnOnce(&mut Value)) -> Job {
    let mut v = serde_json::to_value(job).unwrap();
    f(&mut v);
    serde_json::from_value(v).unwrap()
}

fn ops(job: &Job) -> usize {
    job.programs.iter().map(|p| p.operations.len()).sum()
}

#[test]
fn cached_plans_are_the_same_as_fresh_ones_and_only_edits_are_planned() {
    let all = examples();
    assert!(all.len() >= 6, "{}", all.len());
    for (name, job, mesh, _) in &all {
        let _: &Path;
        let fresh = plan_with_mesh(job, mesh.as_ref()).unwrap();
        let mut cache = PlanCache::new();
        let first = plan_with_mesh_cached(job, mesh.as_ref(), &mut cache).unwrap();
        assert_eq!(first, fresh, "{name}: first cached plan");
        assert_eq!(
            cache.take_counts(),
            (0, ops(job)),
            "{name}: everything planned the first time"
        );
        let again = plan_with_mesh_cached(job, mesh.as_ref(), &mut cache).unwrap();
        assert_eq!(again, fresh, "{name}: replayed");
        assert_eq!(
            cache.take_counts(),
            (ops(job), 0),
            "{name}: nothing planned again"
        );
        // the last operation's feed: that operation alone
        let (pi, oi) = (
            job.programs.len() - 1,
            job.programs.last().unwrap().operations.len() - 1,
        );
        let edited = with(job, |v| {
            let o = &mut v["programs"][pi]["operations"][oi];
            if let Some(f) = o["feed"].as_f64() {
                o["feed"] = serde_json::json!(f * 0.9);
            }
        });
        let fresh2 = plan_with_mesh(&edited, mesh.as_ref()).unwrap();
        let cached2 = plan_with_mesh_cached(&edited, mesh.as_ref(), &mut cache).unwrap();
        assert_eq!(cached2, fresh2, "{name}: after an edit");
        let (hits, misses) = cache.take_counts();
        assert!(
            misses <= 1 && hits == ops(job) - misses,
            "{name}: the last operation only ({hits}, {misses})"
        );
        // the first operation's feed: it, and (only if its end changed) those after it
        let edited = with(job, |v| {
            let o = &mut v["programs"][0]["operations"][0];
            if let Some(f) = o["feed"].as_f64() {
                o["feed"] = serde_json::json!(f * 0.9);
            }
        });
        let fresh3 = plan_with_mesh(&edited, mesh.as_ref()).unwrap();
        let cached3 = plan_with_mesh_cached(&edited, mesh.as_ref(), &mut cache).unwrap();
        assert_eq!(cached3, fresh3, "{name}: after editing the first operation");
        let (_, misses) = cache.take_counts();
        assert!(
            misses <= 1,
            "{name}: a feed does not move where the operation ends ({misses} planned)"
        );
        // the stock: everything again
        let edited = with(job, |v| {
            v["clearance"] = serde_json::json!(v["clearance"].as_f64().unwrap_or(3.0) + 1.0)
        });
        let fresh4 = plan_with_mesh(&edited, mesh.as_ref()).unwrap();
        let cached4 = plan_with_mesh_cached(&edited, mesh.as_ref(), &mut cache).unwrap();
        assert_eq!(cached4, fresh4, "{name}: after a clearance change");
        assert_eq!(
            cache.take_counts(),
            (0, ops(job)),
            "{name}: a job-wide change plans everything"
        );
    }
}
