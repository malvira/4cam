# GPU acceleration: findings and plan

**Question.** Mar asked whether 4cam should use the GPU when one is available, natively with
wgpu (Vulkan/DX12/Metal) and in the browser with WebGPU. Any GPU path needs an automatic
CPU fallback, and the G-code must not depend on whether a GPU is present.

This note records:
- what takes time today;
- what cheaper CPU changes gained;
- where a GPU can help without risking G-code determinism.

Every number here comes from `cargo run --release -p fourcam-core --example profile` on the
development PC.

## The machine

- CPU: Intel Core i9-13900K, 24 cores / 32 threads; 64 GB RAM.
- GPU: NVIDIA GeForce RTX 5080, plus the Intel UHD Graphics 770 in the CPU.
- Headless CI has no GPU, so every GPU path needs a CPU fallback, and the tests must pass
  without a GPU.

## Where the time goes (single thread, before any change)

| Example | Stage | ms | Size |
| --- | --- | ---: | --- |
| knob (Z1) | plan (turning, knurl, chamfer) | 3 | 36 914 moves |
| knob (Z1) | post / verify / time estimate | 37 / 18 / 57 | 36 933 lines |
| knob (Z1) | gouge check | 632 | 21 649 cut moves |
| knob (Z1) | simulate, Normal grid (0.1 mm x 1.5°) | 1 925 | 66 480 cells |
| knob (Z1) | simulate, Fine grid (0.05 mm x 0.5°) | **22 849** | 398 880 cells |
| knob (Z1) | playback to the end (Normal) | 4 194 | 36 898 segments |
| knob mesh finish (Z1) | plan (rotary rough + 2 finishes) | **14 450** | 51 032 moves |
| knob mesh finish (Z1) | gouge check | **13 149** | 50 918 cut moves |
| knob mesh finish (Z1) | simulate, Fine | 16 980 | 399 600 cells |
| plate (Carvera) | plan (incl. 3D raster over the dome) | **21 221** | 16 929 moves |
| plate (Carvera) | simulate 3-axis, Fine | 805 | 153 600 cells |
| plaque (Carvera) | plan (image relief rough + finish + engraving) | 4 965 | 95 894 moves |
| plaque (Carvera) | simulate 3-axis, Fine | 1 385 | 230 400 cells |
| ring (Z1) | simulate, Fine | 13 126 | 734 400 cells |
| ring (Z1) | playback to the end (Normal) | 2 270 | 22 650 segments |

Post, verify and the time estimate never take more than 0.2 s. The cost is in two places:
- **Drop-cutter queries**, used for planning and gouge checks.
- **Rotary stock simulation and playback**, which grow with grid resolution: Fine is about
  6 times Normal's cells and about 10 times its time.

## Cheaper CPU wins first (done, exact)

Three changes to the drop-cutters' spatial index return exactly the same answers, because a
maximum doesn't depend on the order it's taken in:
- **Sort each bin by the triangles' top**, and stop at the first triangle that can't beat
  the best contact found. Rotary bins sort by largest radius, 3-axis bins by top Z.
- **3-axis bins of half the tool radius**, plus a bounding-box test before the exact contact
  calculation.
- **Compute each surface point once per 3-D raster**, not once per step-down level.

| Stage | Before | After |
| --- | ---: | ---: |
| plate: plan | 21 221 ms | **1 467 ms** (14x) |
| plaque: plan | 4 965 ms | **1 825 ms** |
| knob mesh finish: plan | 14 450 ms | **7 396 ms** |
| knob mesh finish: gouge check | 13 149 ms | **6 723 ms** |

All 12 G-code files of the five examples are byte-identical before and after, and the Z1
goldens are unchanged.

## Determinism: why the GPU must not write G-code

WebGPU's shading language (WGSL) has no 64-bit floats, and native `f64` on consumer GPUs is
optional and slow. A drop-cutter evaluated on the GPU in 32-bit floats differs from the CPU
in the last digits. After rounding to the post's 3 decimals that changes some coordinates,
so the G-code would depend on whether a GPU was present. Bit-exact `f32` emulation of the
`f64` code is not realistic.

**Rule: toolpaths and G-code are always computed on the CPU.** The GPU is only for:
- material-removal simulation and playback, compared with the CPU within a stated tolerance;
- display;
- maybe acceleration structures whose answers are re-checked exactly on the CPU.

## Options for what remains

1. **CPU threads, bit-identical** (`std::thread::scope` or rayon).
   - *Simulation:* split the grid into bands of rows, one per thread. Each thread replays
     every tool position in order but stamps only its own rows. Each cell sees its stamps
     in the same order as single-threaded, so heights, operation tags and counts are
     identical.
   - *Drop-cutter:* evaluate the sample points of a raster row or pass in parallel, then
     emit the moves in order.
   - *Reach:* CLI, Python and native hosts get up to the core count. In the browser, WASM
     threads need cross-origin isolation (COOP/COEP headers) and a threaded build, so the
     web app stays single-threaded unless it is served that way.
2. **GPU stamping of the stock heightmap** (wgpu natively, WebGPU in the browser).
   - *Method:* one compute pass per batch of tool positions. A thread per (position, cell
     under the tool) computes the cut height and keeps the lowest with `atomicMin` on a
     quantized height (1 µm), with the operation tag packed in the low bits.
   - *Fit:* the simulation is all independent minimum operations, so it suits a GPU,
     especially Fine/Ultra grids and the browser's playback.
   - *Differences from the CPU:* `f32` and the 1 µm quantization; on equal heights the tag
     is the lowest, not the last one written. These matter for preview and reports only.
   - *Costs:* wgpu is a large dependency. The browser needs a separate JS + WGSL path,
     because our WASM build uses raw exports with no wasm-bindgen; the WGSL can be shared.
     WebGPU ships in Chrome and Edge, with partial support elsewhere.
3. **GPU drop-cutter for previews only** (for example interactive range edits). Not
   recommended: planning is now fast enough, and the G-code must come from the CPU anyway.

## Recommended plan

- **A. Exact CPU index changes.** Done; numbers above.
- **B. Threaded simulation and drop-cutter, bit-identical, on by default natively.** Tested
  by comparing against the single-threaded results bit for bit. This removes most of the
  Fine-grid wait in the CLI and Python with no new dependency.
- **C. GPU simulation prototype in a separate crate** (`fourcam-gpu`, wgpu), behind a
  feature flag, falling back to the CPU when no adapter is found. Tests compare the GPU
  heightmap with the CPU's within the quantization, and skip cleanly without a GPU. A
  benchmark runs on the knob and ring at Fine and Ultra.
- **D. If C pays off: WebGPU in the web app** for Simulate and machining playback at fine
  grids. The same WGSL is driven from JS, with the WASM simulation as fallback.
- **Never:** GPU for toolpaths, G-code, the verifier or the time estimates.

Results of B and C are added below as they land.
