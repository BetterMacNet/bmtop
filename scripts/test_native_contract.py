#!/usr/bin/env python3
"""Compile real GPU C function bodies against a synthetic adapter; no SDK claim."""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def main() -> int:
    compiler = shutil.which(os.environ.get("CC", "clang"))
    if compiler is None:
        print("SKIPPED: clang not available; no native contract tests executed")
        return 77
    source = (ROOT / "crates/bmtop-macos/src/bmtop_sys.c").read_text()
    start = source.index("static uint64_t sum_app_usage(")
    end = source.index("/* GPU metadata", start)
    with tempfile.TemporaryDirectory(prefix="bmtop-native-contract-") as directory:
        build = Path(directory)
        (build / "production_gpu_contract.inc").write_text(source[start:end])
        executable = build / "gpu-contract"
        command = [compiler, "-std=c11", "-Wall", "-Wextra", "-Werror", "-fsanitize=address,undefined", "-fno-omit-frame-pointer",
                   "-I", str(build), str(ROOT / "tests/native/gpu_enumeration_harness.c"), "-o", str(executable)]
        subprocess.run(command, check=True, timeout=60)
        subprocess.run([str(executable)], check=True, timeout=30)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
