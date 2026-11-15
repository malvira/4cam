# 4cam

An open-source CAM engine for 4th-axis (rotary) machining, written in Rust.

You describe the stock, a revolved part profile, the tools and the operations in a
JSON job file; 4cam plans machine-neutral toolpaths, posts them to G-code for a
machine profile, and sanity-checks the result.

Status: milestone 0. It reproduces, byte for byte, the programs for a knurled knob
that were cut on a Makera Z1 (`tests/golden/`).

## Operations

- `rotary_turning` - rough the stock down to the profile in radial levels with a
  flat endmill moving helically along X, then finish in one helical pass.
- `helical_knurl` - diamond knurl: families of helical V grooves over a conical body.
- `flank_chamfer` - chamfer an edge with the flank of a V-bit while the part turns.

## Usage

```bash
cargo run -p fourcam-cli -- run examples/knob/knob.job.json --out out
```

```bash
cargo run -p fourcam-cli -- verify out/knob_1_profile.nc
```

```bash
cargo run -p fourcam-cli -- toolpath examples/knob/knob.job.json
```

`run` writes one `.nc` per program (default machine: `machines/makera-z1.json`, or
pass `--machine`) and verifies each. `verify` flags feed moves where Z falls faster
than the plunge limit and X/A rapids below the safe radius.

## Coordinates

A rotates about X (degrees). X0 is the front face of the part, which extends
toward -X. Y0/Z0 is the rotation axis, so Z is the radius at the tool tip. Units mm.

## Layout

- `crates/fourcam-core` - library (`fourcam`): job schema, geometry, strategies,
  toolpath IR, post-processor, verifier.
- `crates/fourcam-cli` - the `4cam` binary.
- `machines/` - machine profiles.
- `examples/` - example jobs.
- `reference/` - the original scripts the engine was ported from.

## License

Apache-2.0. See `LICENSE`.
