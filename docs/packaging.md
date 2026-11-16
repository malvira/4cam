# Packaging

Everything is prepared and checked, and **nothing is published**. Publishing to a public
registry waits for the maintainer's explicit go-ahead. `sh scripts/package-dry-run.sh`
rebuilds and re-checks all of it into `target/pkg/`.

The names are free on every registry (checked 2026-11-16): `fourcam*` on crates.io,
`fourcam` on PyPI and on npm.

## crates.io: five crates

| Crate | What it is | Packaged |
| --- | --- | --- |
| `fourcam-core` | the engine (library `fourcam`), with the built-in machine profiles | 44 files, 156 KiB |
| `fourcam-gpu` | the WebGPU stock simulation (optional for the CLI) | 7 files |
| `fourcam-wasm` | the JSON API shared by the bindings, and the WebAssembly exports | 9 files |
| `fourcam-capi` | the C ABI (`fourcam_c`, `include/fourcam.h`) | 10 files |
| `fourcam-cli` | the `4cam` command | 7 files |

`cargo publish --dry-run --workspace` packages every crate, builds each one on its own
against the others, and aborts the uploads.

The crates carry their own data, because a published crate cannot reach outside its
folder:
- the built-in profiles moved to `crates/fourcam-core/profiles/`;
- the GPU shader is copied into `crates/fourcam-gpu/src/stamp.wgsl`;
- the example job is copied into `crates/fourcam-wasm/data/`.

Tests check that each copy matches the original. Tests and benchmarks, which read the
repository's examples, are excluded from the packages.

Not published: `fourcam-py` (goes to PyPI), `fourcam-fixtures` and the SolidCraft adapter
(`publish = false`).

To publish, in dependency order (cargo does the order itself):
`cargo publish --workspace`, with a crates.io token.

## PyPI: `fourcam`

- `crates/fourcam-py/pyproject.toml` builds with maturin: one abi3 wheel serves Python 3.9+
  on a platform.
- Here that is `fourcam-0.1.0-cp39-abi3-win_amd64.whl` (1.2 MB), plus the sdist
  `fourcam-0.1.0.tar.gz` (200 KB).
- The wheel passes the Python tests in a fresh virtualenv. The sdist builds into a wheel
  from source, which passes them too.
- For the other platforms (Linux manylinux, macOS), build the wheels in CI: one
  `maturin build --release` per runner.

To publish: `maturin upload target/pkg/python/*` (or `twine upload`), with a PyPI token.

## npm: `fourcam`

- `scripts/build-npm.sh` assembles `target/pkg/npm`: the ES-module wrapper, `fourcam.wasm`,
  an `init()` entry that loads the bundled module in Node and browsers, TypeScript types,
  README and LICENSE.
- The package plans the knob from Node. It packs to `target/pkg/fourcam-0.1.0.tgz`
  (602 kB, 7 files), and `npm publish --dry-run` accepts it.

To publish: `npm publish target/pkg/npm` while logged in to npm.

## Not yet

- Signed releases and GitHub release assets (CLI binaries, the C library, the wheels).
- A conda package.
