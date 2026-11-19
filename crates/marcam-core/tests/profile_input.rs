//! Revolved profiles as people write them: front to back (descending x) plans exactly like
//! back to front; steps anywhere, duplicates, a single point or none plan without panicking;
//! x going back and forth, or a point that is not a number, is refused with the reason.

use serde_json::{json, Value};

fn knob() -> Value {
    serde_json::from_str(include_str!("../../../examples/knob/knob.job.json")).unwrap()
}

fn with_points(points: Value) -> Value {
    let mut j = knob();
    j["part"]["profile"]["points"] = points;
    j
}

fn plan(j: &Value) -> Result<String, String> {
    let job = marcam::Job::from_json(&j.to_string()).map_err(|e| e.to_string())?;
    let tps = marcam::plan(&job).map_err(|e| e.to_string())?;
    Ok(format!("{tps:?}"))
}

#[test]
fn a_profile_written_front_to_back_plans_like_back_to_front() {
    let pts = knob()["part"]["profile"]["points"].clone();
    let mut rev = pts.as_array().unwrap().clone();
    rev.reverse();
    assert_eq!(
        plan(&with_points(json!(rev))).unwrap(),
        plan(&knob()).unwrap()
    );
    // with a step: the same geometry either way round
    let up = json!([[-20.0, 4.0], [-10.0, 4.0], [-10.0, 6.0], [0.0, 6.0]]);
    let down = json!([[0.0, 6.0], [-10.0, 6.0], [-10.0, 4.0], [-20.0, 4.0]]);
    assert_eq!(
        plan(&with_points(down)).unwrap(),
        plan(&with_points(up)).unwrap()
    );
}

#[test]
fn odd_profiles_plan_or_say_why_never_panic() {
    let cases = [
        json!([[-20.0, 2.0], [-20.0, 8.0], [0.0, 8.0]]),
        json!([[-20.0, 8.0], [0.0, 8.0], [0.0, 2.0]]),
        json!([[-5.0, 2.0], [-5.0, 8.0]]),
        json!([[-5.0, 8.0]]),
        json!([[-20.0, 8.0], [-20.0, 8.0], [0.0, 8.0], [0.0, 8.0]]),
        json!([
            [-20.0, 8.0],
            [-10.0, 8.0],
            [-10.0, 4.0],
            [-10.0, 6.0],
            [0.0, 6.0]
        ]),
        json!([]),
        json!([[-20.0, 0.0], [0.0, 0.0]]),
        json!([[-20.0, 50.0], [0.0, 50.0]]),
    ];
    for pts in cases {
        let j = with_points(pts.clone());
        let r = std::panic::catch_unwind(|| plan(&j));
        assert!(r.is_ok(), "panicked on {pts}");
        if let Ok(Ok(s)) = r {
            assert!(!s.contains("NaN"), "NaN in the toolpath of {pts}");
        }
    }
}

#[test]
fn a_zigzag_or_a_bad_point_is_refused_with_the_reason() {
    let e = plan(&with_points(json!([
        [-20.0, 8.0],
        [-5.0, 8.0],
        [-10.0, 6.0],
        [0.0, 6.0]
    ])))
    .unwrap_err();
    assert!(
        e.contains("points 2 and 3") && e.contains("x goes back"),
        "{e}"
    );
    let e = plan(&with_points(json!([[-20.0, 8.0], [0.0, -1.0]]))).unwrap_err();
    assert!(e.contains("profile point 2"), "{e}");
}
