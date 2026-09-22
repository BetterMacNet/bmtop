#!/usr/bin/env python3
"""macOS PTY benchmark. Never stores terminal contents or enables FPS/sudo.

The measurements include startup and shutdown. wait4 CPU accounting may include
reaped child tools: it is not advertised as exclusive collector or renderer CPU.
Only relative runs with matching scenarios may be used as a regression gate.
"""
from __future__ import annotations
import argparse
import errno
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from typing import Any


def percentile(values: list[float], percent: float) -> float | None:
    if not 0 < percent <= 100:
        raise ValueError("percent must be in (0, 100]")
    if not values:
        return None
    if any(not math.isfinite(value) for value in values):
        raise ValueError("non-finite measurements are invalid")
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * percent / 100) - 1)]


def parse_interval(value: str) -> int:
    if value.endswith("ms"):
        millis = int(value[:-2])
    elif value.endswith("s"):
        millis = int(value[:-1]) * 1000
    else:
        raise ValueError("interval must end in ms or s")
    if not 250 <= millis <= 60_000:
        raise ValueError("interval must be between 250ms and 60s")
    return millis


def quality_failures(report: dict[str, Any]) -> list[str]:
    diagnostics = report["runtime"]
    issues = []
    if diagnostics["samples"] < 2:
        issues.append("fewer than two samples; measurement is inconclusive")
    if diagnostics["sample_errors"] or diagnostics["detail_errors"]:
        issues.append("collector/detail errors during measurement")
    # Event-driven rendering: initial layout + sample/detail/input changes.
    budget = (diagnostics["samples"] + diagnostics["input_events"]
              + diagnostics["detail_service"]["total_observations"] + 4)
    if diagnostics["renders"] > budget:
        issues.append(f"idle render budget exceeded: {diagnostics['renders']} > {budget}")
    return issues


def compare(report: dict[str, Any], baseline: dict[str, Any], limit: float) -> list[str]:
    if not math.isfinite(limit) or limit < 0:
        raise ValueError("regression allowance must be finite and nonnegative")
    # Runtime version/architecture/OS/scenario mismatch must not silently pass.
    for key in ["scenario", "platform", "runtime_version"]:
        if report[key] != baseline[key]:
            raise ValueError(f"incompatible baseline: {key} differs")
    issues = []
    for key in ["cpu_seconds_wait4", "rss_p95_bytes"]:
        old, current = baseline[key], report[key]
        if old is None or old <= 0 or current is None:
            issues.append(f"{key}: missing/zero baseline; regression is inconclusive")
        elif current > old * (1 + limit / 100):
            issues.append(f"{key}: {100 * (current / old - 1):.1f}% regression exceeds {limit:.1f}%")
    return issues


def _rss_bytes(pid: int) -> int | None:
    # ps observes only the bmtop PID, not its descendants. Unit is KiB on macOS.
    try:
        result = subprocess.run(["/bin/ps", "-p", str(pid), "-o", "rss="],
                                check=False, capture_output=True, text=True, timeout=1)
        value = result.stdout.strip()
        return int(value) * 1024 if result.returncode == 0 and value else None
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None


def measure(binary: Path, seconds: float, interval: str, mode: int) -> dict[str, Any]:
    """Real subprocess/PTTY run; caller is responsible for platform eligibility."""
    with tempfile.TemporaryDirectory(prefix="bmtop-benchmark-") as directory:
        diagnostics_path = Path(directory) / "timings.json"  # exclusive-create by bmtop
        args = [str(binary), "--no-config", "--lang", "en", "--privacy", "strict", "-i", interval,
                "--diagnostics-file", str(diagnostics_path), "top", "--mode", str(mode)]
        started = time.monotonic()
        pid, master = pty.fork()
        if pid == 0:
            environment = os.environ.copy()
            for key in ["BMTOP_CONFIG", "BMTOP_LANG", "BMTOP_INTERVAL", "BMTOP_PRIVACY"]:
                environment.pop(key, None)
            environment.update({"TERM": "xterm-256color", "LANG": "C", "LC_ALL": "C"})
            try:
                os.execve(str(binary), args, environment)
            except OSError:
                os._exit(127)
        # Dimensions are part of the comparison identity, not inferred from the host.
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 48, 160, 0, 0))
        rss: list[float] = []
        output_bytes = 0
        deadline = started + seconds
        next_rss = started
        quit_sent = False
        waited = False
        eof = False
        status = None
        usage = None
        try:
            while True:
                reaped, candidate_status, candidate_usage = os.wait4(pid, os.WNOHANG)
                if reaped:
                    waited, status, usage = True, candidate_status, candidate_usage
                    break
                now = time.monotonic()
                if now >= deadline and not quit_sent:
                    os.write(master, b"q")
                    quit_sent = True
                if now >= deadline + 5:
                    raise TimeoutError("TUI did not exit after q; no benchmark accepted")
                if now >= next_rss and not quit_sent:
                    value = _rss_bytes(pid)
                    if value is not None:
                        rss.append(float(value))
                    next_rss = now + 0.5
                if eof:
                    time.sleep(0.03)
                else:
                    ready, _, _ = select.select([master], [], [], 0.05)
                    if ready:
                        try:
                            chunk = os.read(master, 65536)
                            eof = not chunk
                            output_bytes += len(chunk)  # never save terminal contents
                        except OSError as error:
                            if error.errno != errno.EIO:
                                raise
                            eof = True
            if status is None or os.waitstatus_to_exitcode(status) != 0:
                code = os.waitstatus_to_exitcode(status) if status is not None else None
                raise RuntimeError(f"bmtop exited with {code}; no successful benchmark")
            if not quit_sent:
                raise RuntimeError("bmtop exited before the requested window")
            if usage is None:
                raise RuntimeError("missing child resource accounting")
            # A failed/aborted TUI may leave an empty exclusive-created file.
            with diagnostics_path.open() as source:
                runtime = json.load(source)
            return {
                "benchmark_version": 1,
                "measurement_kind": "real_macos_pty_including_startup",
                "scenario": {"seconds": seconds, "interval_ms": parse_interval(interval), "mode": mode,
                             "terminal_columns": 160, "terminal_rows": 48, "privacy": "strict", "fps": False},
                "platform": {"system": platform.system(), "release": platform.release(), "architecture": platform.machine()},
                "runtime_version": runtime["diagnostics_version"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "elapsed_seconds": time.monotonic() - started,
                "cpu_seconds_wait4": usage.ru_utime + usage.ru_stime,
                "rss_p95_bytes": percentile(rss, 95), "rss_samples": len(rss),
                "terminal_output_bytes": output_bytes, "runtime": runtime,
            }
        finally:
            if not waited:
                # pty.fork creates the child session/group; never signal our parent.
                try:
                    os.killpg(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                try:
                    os.waitpid(pid, 0)
                except ChildProcessError:
                    pass
            os.close(master)


def save_new(path: Path, report: dict[str, Any]) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as destination:
        json.dump(report, destination, ensure_ascii=False, indent=2, allow_nan=False)
        destination.write("\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/bmtop"))
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--interval", default="5s")
    parser.add_argument("--mode", type=int, choices=range(1, 10), default=1)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--max-regression-pct", type=float, default=20)
    args = parser.parse_args()
    try:
        if sys.platform != "darwin":
            raise RuntimeError("real hardware benchmark requires macOS; no synthetic numbers will be emitted")
        if args.output.exists() or args.output.is_symlink():
            raise ValueError("output must not already exist")
        interval_ms = parse_interval(args.interval)
        if not math.isfinite(args.seconds) or args.seconds < interval_ms / 1000 * 2 + 2:
            raise ValueError("measurement must last at least 2 intervals + 2 seconds")
        binary = args.binary.resolve(strict=True)
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise ValueError("binary is not executable")
        report = measure(binary, args.seconds, args.interval, args.mode)
        failures = quality_failures(report)
        if args.baseline:
            with args.baseline.open() as source:
                baseline = json.load(source)
            failures.extend(quality_failures(baseline))
            failures.extend(compare(report, baseline, args.max_regression_pct))
        report["gate_failures"] = failures
        save_new(args.output, report)
        print("PASS" if not failures else "FAIL", args.output)
        for failure in failures:
            print(f"  {failure}")
        return int(bool(failures))
    except (OSError, RuntimeError, ValueError, KeyError, TimeoutError) as error:
        print(f"benchmark: {error}", file=sys.stderr)
        return 2

if __name__ == "__main__":
    raise SystemExit(main())
