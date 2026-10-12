#!/usr/bin/env python3
"""Format this mod's workspace, excluding downloaded external path dependencies."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1",
    ], cwd=ROOT))
    members = set(metadata["workspace_members"])
    packages = sorted(package["name"] for package in metadata["packages"] if package["id"] in members)
    command = ["cargo", "fmt"]
    for package in packages:
        command.extend(["--package", package])
    if args.check:
        command.extend(["--", "--check"])
    print(f"Formatting {len(packages)} mod workspace packages; external engine source stays external.", flush=True)
    return subprocess.run(command, cwd=ROOT).returncode


if __name__ == "__main__":
    raise SystemExit(main())
