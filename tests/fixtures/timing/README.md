# Hand-timed fixtures

Small programs whose cutting time can be worked out on paper. The test
`crates/fourcam-core/tests/timing.rs` plans each one and checks that:

- the estimate at the programmed feeds (no acceleration, no axis limits) matches the
  hand figure;
- the full estimate, from the toolpath and from the posted G-code, is at most a few
  percent above the estimate without acceleration (8 % for the drilling, which stops at
  every peck).

The hand working is in the test, next to each fixture.

| Fixture | Operation | Hand figure |
| --- | --- | --- |
| `facing` | face a 40 x 40 block, 4 mm flat, stepover 2 | 3 + 20 x 42 + 19 x 2 mm at 600 mm/min, 1 mm plunge: 89.1 s |
| `contour` | 30 x 30 square outside, 2 levels | 2 x (136 mm at 600 + 1 mm plunge): 29.2 s |
| `pocket` | 30 x 20 pocket, 2 levels, stepover 2 | 2 x ((250 + 114.5) mm at 600 + 1 mm plunge): 74.9 s |
| `drilling` | 4 holes 3 mm deep, pecks of 1 | 4 x 4.4 mm at 60 mm/min: 17.6 s |
| `raster_plate` | 3D raster over a flat 30 x 20 plate (`plate.stl`) | 350 mm at 600, 1 mm plunge: 36 s |
| `turning` | turn a 10 mm bar to 8 mm, 2 mm per rev, in each feed mode | 3039.4 mm at 240 mm/min + 7 mm of plunges: 766.8 s |
