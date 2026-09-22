#!/usr/bin/env python3
"""Standard-library-only structural checks. NOT a Rust compiler or macOS test."""
import ast
import json
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]

def main() -> int:
    counts = {"toml": 0, "json": 0, "python": 0, "shell": 0}
    ignored = {".git", "target", ".cargo-cache", "__pycache__", ".pytest_cache", "verification"}
    for path in sorted(ROOT.rglob("*")):
        relative = path.relative_to(ROOT)
        if not path.is_file() or ignored.intersection(relative.parts):
            continue
        if path.suffix == ".toml" or path.name == "Cargo.lock":
            tomllib.loads(path.read_text()); counts["toml"] += 1
        elif path.suffix == ".json":
            json.loads(path.read_text()); counts["json"] += 1
        elif path.suffix == ".py":
            ast.parse(path.read_text(), filename=str(relative)); counts["python"] += 1
        elif path.suffix == ".sh":
            interpreter = "bash" if "bash" in path.read_text().splitlines()[0] else "sh"
            subprocess.run([interpreter, "-n", str(path)], check=True); counts["shell"] += 1
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    locked = {(p["name"], p["version"]) for p in lock["package"]}
    for member in workspace["workspace"]["members"]:
        manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
        package = manifest["package"]
        version = package["version"]
        if isinstance(version, dict):
            version = workspace["workspace"]["package"]["version"]
        if (package["name"], version) not in locked:
            raise ValueError(f"workspace package absent from Cargo.lock: {member}")
        for spec in manifest.get("dependencies", {}).values():
            if isinstance(spec, dict) and "path" in spec:
                if not (ROOT / member / spec["path"] / "Cargo.toml").is_file():
                    raise ValueError(f"missing path dependency in {member}: {spec['path']}")
    print(json.dumps({"status": "passed", "scope": "structural_only_not_Rust_compilation", "counts": counts}, indent=2))
    return 0

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"source check failed: {error}", file=sys.stderr)
        raise SystemExit(1)
