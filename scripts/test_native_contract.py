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
    with tempfile.TemporaryDirectory(prefix="bmtop-native-contract-") as directory:
        build = Path(directory)
        compile_contract(
            compiler,
            build,
            source[source.index("static uint64_t sum_app_usage("):source.index("/* GPU metadata")],
            ROOT / "tests/native/gpu_enumeration_harness.c",
            "gpu",
        )
        compile_contract(
            compiler,
            build,
            source[source.index("static int64_t stats_number("):source.index("/* ---- 每进程 GPU")],
            ROOT / "tests/native/disk_io_harness.c",
            "disk_io",
        )
        compile_layout_contract(compiler, build)
    return 0


def compile_layout_contract(compiler: str, build: Path) -> None:
    executable = build / "ffi-layout"
    command = [
        compiler,
        "-std=c11",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-I",
        str(ROOT / "crates/bmtop-macos/src"),
        str(ROOT / "tests/native/ffi_layout_harness.c"),
        "-o",
        str(executable),
    ]
    subprocess.run(command, check=True, timeout=60)
    subprocess.run([str(executable)], check=True, timeout=30)


def compile_contract(compiler: str, build: Path, body: str, harness: Path, name: str) -> None:
    (build / f"production_{name}_contract.inc").write_text(body)
    executable = build / name
    command = [
        compiler,
        "-std=c11",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-fsanitize=address,undefined",
        "-fno-omit-frame-pointer",
        "-I",
        str(build),
        str(harness),
        "-o",
        str(executable),
    ]
    subprocess.run(command, check=True, timeout=60)
    subprocess.run([str(executable)], check=True, timeout=30)

if __name__ == "__main__":
    raise SystemExit(main())
