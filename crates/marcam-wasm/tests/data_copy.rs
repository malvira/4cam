//! The crate carries its own copy of the example job (a published crate cannot reach outside
//! it); it must stay the examples' knob.
#[test]
fn example_job_is_the_knob() {
    let here = include_str!("../data/knob.job.json").replace("\r\n", "\n");
    let ex = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/knob/knob.job.json"
    ))
    .expect("examples/knob/knob.job.json")
    .replace("\r\n", "\n");
    assert!(
        here == ex,
        "crates/marcam-wasm/data/knob.job.json differs from examples/knob/knob.job.json"
    );
}
