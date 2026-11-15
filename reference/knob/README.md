# Reference: knurled knob

The original Python scripts that 4cam milestone 0 was ported from.

- `knob.py` - generates the knob STL (tapered diamond-knurled body with a boss).
- `knob_set.py` - companion model script for the knob set.
- `knob_rotary.py` - writes the two 4th-axis programs for the Makera Z1:
  `knob_1_profile.nc` (T1 flat endmill, rough + finish) and `knob_2_knurl.nc`
  (T2 90° V-bit, diamond knurl + front chamfer).

The G-code these produced, which was actually cut on the Z1, is kept in
`tests/golden/`. `examples/knob/knob.job.json` describes the same part as a 4cam
job, and `cargo test` checks that 4cam reproduces the golden files.
