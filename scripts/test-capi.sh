#!/bin/sh
# The C ABI end to end: build the library, compile examples/embed/c/plan.c with the
# platform's C compiler, run it on the knob, and run the C# P/Invoke example when a .NET SDK
# is installed (skipped, saying so, otherwise).
set -e
cd "$(dirname "$0")/.."
cargo build -q -p fourcam-capi --release
EXE=$(cargo run -q -p fourcam-capi --release --example cc_run -- examples/embed/c/plan.c target/release target/capi | tail -1)
OUT=$("$EXE" examples/knob/knob.project.json)
echo "$OUT"
echo "$OUT" | grep -q '"name":"4cam"'
echo "$OUT" | grep -q '"gcode":"(Knob radial profile'
if command -v dotnet >/dev/null 2>&1; then
  LIB=$(pwd)/target/release
  OUT=$(cd examples/embed/csharp && PATH="$LIB:$PATH" LD_LIBRARY_PATH="$LIB" DYLD_LIBRARY_PATH="$LIB" \
    DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 dotnet run -c Release -- ../../knob/knob.project.json)
  echo "$OUT"
  echo "$OUT" | grep -q "knob_1_profile: 21787 lines, 0 issue(s)"
else
  echo "no dotnet: skipping the C# example"
fi
echo "C ABI OK"
