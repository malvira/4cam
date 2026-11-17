# Changelog

## Unreleased

### Renamed: 4cam is now marcam (2026-11-17)

The GitHub repository is now [malvira/marcam](https://github.com/malvira/marcam). Everything
that carried the old name was renamed:

| Was | Now |
| --- | --- |
| crates `fourcam-core`, `-cli`, `-wasm`, `-py`, `-gpu`, `-capi`, `-fixtures` | `marcam-core`, `marcam-cli`, ... |
| Rust library `fourcam` | `marcam` |
| command `4cam` | `marcam` (`4cam` is kept as an alias) |
| C library `fourcam_c`, header `fourcam.h`, functions `fourcam_*` | `marcam_c`, `marcam.h`, `marcam_*` |
| Python module `fourcam`, FreeCAD package `fourcam_freecad`, macro `4cam_plan.FCMacro` | `marcam`, `marcam_freecad`, `marcam_plan.FCMacro` |
| npm package and web files `fourcam.js` / `fourcam.wasm` | `marcam`, `marcam.js` / `marcam.wasm` |
| SolidCraft plugin `fourcam.zip` | `marcam.zip` |
| project files `.4cam.json` (`"format": "4cam-project"`) | `.marcam.json` (`"marcam-project"`) |
| profile formats `4cam-machine`, `4cam-tools` | `marcam-machine`, `marcam-tools` |
| `$FOURCAM_PROFILES`, `%APPDATA%cam\profiles` | `$MARCAM_PROFILES`, `%APPDATA%\marcam\profiles` |

Still read: `.4cam.json` projects, `4cam-machine` and `4cam-tools` profiles,
`$FOURCAM_PROFILES` and an existing `4cam` profile folder (when there is no `marcam` one),
and the web app's settings saved under `4cam.*`.

The G-code is unchanged: no program text carries the name.
