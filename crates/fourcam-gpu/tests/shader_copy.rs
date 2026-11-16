//! The crate carries its own copy of the shader (a published crate cannot reach outside it);
//! it must stay the web app's.
#[test]
fn shader_is_the_web_apps() {
    let here = include_str!("../src/stamp.wgsl").replace("\r\n", "\n");
    let web = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web/gpu/stamp.wgsl"
    ))
    .expect("web/gpu/stamp.wgsl")
    .replace("\r\n", "\n");
    assert!(
        here == web,
        "crates/fourcam-gpu/src/stamp.wgsl differs from web/gpu/stamp.wgsl: copy it over"
    );
}
