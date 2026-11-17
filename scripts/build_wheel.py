"""Build the `marcam` Python wheel without extra tools: cargo builds the pyo3 extension,
this script packs it (stdlib only). Output: target/wheels/marcam-<ver>-cp39-abi3-<plat>.whl

    python scripts/build_wheel.py
"""
import base64
import hashlib
import shutil
import subprocess
import sys
import sysconfig
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION = "0.1.0"


def main() -> Path:
    subprocess.run(
        ["cargo", "build", "--release", "-p", "marcam-py"],
        cwd=ROOT,
        check=True,
        env={**__import__("os").environ, "PYO3_PYTHON": sys.executable},
    )
    rel = ROOT / "target" / "release"
    if sys.platform == "win32":
        built, module = rel / "marcam.dll", "marcam.pyd"
    elif sys.platform == "darwin":
        built, module = rel / "libmarcam.dylib", "marcam.abi3.so"
    else:
        built, module = rel / "libmarcam.so", "marcam.abi3.so"
    plat = sysconfig.get_platform().replace("-", "_").replace(".", "_")
    tag = f"cp39-abi3-{plat}"
    dist = f"marcam-{VERSION}.dist-info"
    files = {
        module: built.read_bytes(),
        f"{dist}/METADATA": (
            "Metadata-Version: 2.1\n"
            f"Name: marcam\nVersion: {VERSION}\n"
            "Summary: 4th-axis rotary CAM engine (Python bindings)\n"
            "License: Apache-2.0\n"
            "Home-page: https://github.com/malvira/marcam\n"
            "Requires-Python: >=3.9\n"
        ).encode(),
        f"{dist}/WHEEL": (
            "Wheel-Version: 1.0\nGenerator: marcam build_wheel.py\n"
            f"Root-Is-Purelib: false\nTag: {tag}\n"
        ).encode(),
    }
    record = []
    for name, data in files.items():
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        record.append(f"{name},sha256={digest},{len(data)}")
    record.append(f"{dist}/RECORD,,")
    files[f"{dist}/RECORD"] = ("\n".join(record) + "\n").encode()
    out_dir = ROOT / "target" / "wheels"
    out_dir.mkdir(parents=True, exist_ok=True)
    whl = out_dir / f"marcam-{VERSION}-{tag}.whl"
    with zipfile.ZipFile(whl, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in files.items():
            z.writestr(name, data)
    print(whl)
    return whl


if __name__ == "__main__":
    main()
