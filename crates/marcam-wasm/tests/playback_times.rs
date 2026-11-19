//! The playback's machine time per segment: non-decreasing, ending at the job's estimated
//! total, the same with and without the stock (`path_only`), and nothing simulated for a
//! path-only playback.

use marcam_wasm::api;
use serde_json::json;

fn floats(b64: &str) -> Vec<f32> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut bytes = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in b64.bytes() {
        let Some(v) = val(c) else { continue };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((acc >> bits) as u8);
        }
    }
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

#[test]
fn segment_times_add_up_to_the_estimate() {
    let job = api::EXAMPLE_JOB;
    let planned = api::plan_and_post(job, "makera-z1-4axis", &[], "{}");
    let total = planned["time_total_s"].as_f64().unwrap();
    let opts = json!({"machine": "makera-z1-4axis", "res": "coarse"});
    let path = api::sim_open(
        job,
        &json!({"machine": "makera-z1-4axis", "path_only": true}).to_string(),
        &[],
    );
    assert_eq!(path["path_only"], json!(true));
    let t = floats(path["times"].as_str().unwrap());
    assert_eq!(t.len(), path["segments"].as_u64().unwrap() as usize);
    assert!(t.windows(2).all(|w| w[1] >= w[0]), "times never go back");
    let last = *t.last().unwrap() as f64;
    assert!(
        (last - total).abs() < 0.01 * total + 1.0,
        "{last} s against the estimate {total} s"
    );
    // nothing to seek: no stock was made
    assert!(api::sim_seek(10).get("error").is_some());
    // with the stock: the same times
    let full = api::sim_open(job, &opts.to_string(), &[]);
    assert_eq!(full["times"], path["times"]);
    api::sim_close();
}
