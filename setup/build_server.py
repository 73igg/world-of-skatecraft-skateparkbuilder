#!/usr/bin/env python3
"""Build the patched vmangos image locally using only vmangos-deploy's Docker source."""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REVISION = "0e3ff01e76d4758e8a7c3108b2717cc785ed56fa"
PATCH = ROOT / "setup/vmangos/skatecraft.patch"


def prepare_context(deploy: Path, context: Path, jobs: int) -> Path:
    # Copy only explicitly tracked Docker source, never storage, configs or game data.
    paths = subprocess.check_output(["git", "-C", str(deploy), "ls-files", "-z", "--", "docker"]).split(b"\0")
    for item in paths:
        if not item:
            continue
        relative = item.decode("utf-8")
        source = deploy / relative
        if source.is_symlink() or not source.resolve().is_relative_to((deploy / "docker").resolve()):
            raise ValueError("Unsafe Docker source path: " + relative)
        target = context / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    dockerfile = context / "docker/server/Dockerfile"
    text = dockerfile.read_text(encoding="utf-8")
    marker = 'make -j"$(nproc)"'
    if text.count(marker) != 1:
        raise ValueError("The pinned vmangos-deploy Dockerfile changed; refusing an unchecked build.")
    dockerfile.write_text(text.replace(marker, f"make -j{jobs}"), encoding="utf-8", newline="\n")
    shutil.copyfile(PATCH, context / "docker/patches/zz-skatecraft.patch")
    return dockerfile


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("deploy", nargs="?", type=Path, default=ROOT / "server")
    parser.add_argument("--jobs", type=int, default=int(os.environ.get("SKATECRAFT_BUILD_JOBS", "4")))
    args = parser.parse_args()
    if not 1 <= args.jobs <= 64:
        parser.error("--jobs must be between 1 and 64")
    deploy = args.deploy.resolve()
    compose = deploy / "compose.yaml"
    text = compose.read_text(encoding="utf-8-sig")
    image = "world-of-skatecraft/vmangos-server:5875-" + hashlib.sha256(PATCH.read_bytes()).hexdigest()[:12]
    pattern = r"(?m)(image:\s*)(?:ghcr\.io/mserajnik/vmangos-server:5875|world-of-skatecraft/vmangos-server:[^\s]+)"
    rewritten, count = re.subn(pattern, lambda match: match[1] + image, text)
    if count == 0:
        raise ValueError("compose.yaml has no recognized vmangos image; refusing to replace another server.")
    present = subprocess.run(["docker", "image", "inspect", image], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if present.returncode:
        state = ROOT / ".setup"
        state.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="source-server-build-", dir=state) as temporary:
            context = Path(temporary).resolve()
            if not context.is_relative_to(state.resolve()):
                raise ValueError("Build context escaped the setup directory.")
            dockerfile = prepare_context(deploy, context, args.jobs)
            print(f"Building {image} from vmangos {REVISION} ({args.jobs} jobs)", flush=True)
            subprocess.run([
                "docker", "build", "-t", image, "-f", str(dockerfile),
                "--build-arg", "VMANGOS_REVISION=" + REVISION,
                "--build-arg", "VMANGOS_CLIENT_VERSION=5875",
                "--build-arg", "VMANGOS_FAIL_ON_PATCH_ERROR=1", str(context),
            ], check=True)
    compose.write_text(rewritten, encoding="utf-8", newline="\n")
    print(f"{compose} now runs {image}")


if __name__ == "__main__":
    main()
