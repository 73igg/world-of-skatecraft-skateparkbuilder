#!/usr/bin/env python3
"""Build matching, pinned vmangos server and database images from source."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REVISION = "0e3ff01e76d4758e8a7c3108b2717cc785ed56fa"
DEPLOY_REVISION = "df0f03e9f4a826980001cbb047a00301145744ce"
DEPLOY_URL = "https://github.com/mserajnik/vmangos-deploy.git"
DEPLOY_VERSION = "1"
PATCH = ROOT / "setup/vmangos/skatecraft.patch"


def labels() -> dict[str, str]:
    return {
        "io.skatecraft.core": REVISION,
        "io.skatecraft.deploy": DEPLOY_REVISION,
        "io.skatecraft.patch": hashlib.sha256(PATCH.read_bytes()).hexdigest(),
    }


def image_names() -> dict[str, str]:
    digest = hashlib.sha256("\n".join(labels().values()).encode()).hexdigest()[:16]
    return {kind: f"world-of-skatecraft/vmangos-{kind}:5875-{digest}"
            for kind in ("server", "database")}


def source_checkout() -> Path:
    # Never reset a player's server checkout: it contains their configuration and storage.
    source = ROOT / ".setup/vmangos-deploy-source"
    source.parent.mkdir(exist_ok=True)
    if not (source / ".git").exists():
        subprocess.run(["git", "clone", "--quiet", "--no-checkout", DEPLOY_URL, str(source)], check=True)
    if subprocess.run(["git", "-C", str(source), "cat-file", "-e", DEPLOY_REVISION],
                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
        subprocess.run(["git", "-C", str(source), "fetch", "--quiet", "origin", DEPLOY_REVISION], check=True)
    subprocess.run(["git", "-C", str(source), "checkout", "--quiet", DEPLOY_REVISION], check=True)
    return source


def prepare_context(deploy: Path, context: Path, jobs: int) -> None:
    # git show reads the pinned source, even if somebody edited the scratch checkout.
    paths = subprocess.check_output([
        "git", "-C", str(deploy), "ls-tree", "-rz", "--name-only", DEPLOY_REVISION, "--", "docker",
    ]).split(b"\0")
    for item in paths:
        if not item:
            continue
        relative = Path(item.decode("utf-8"))
        target = context / relative
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("Unsafe Docker source path")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(subprocess.check_output([
            "git", "-C", str(deploy), "show", f"{DEPLOY_REVISION}:{relative.as_posix()}",
        ]))
    dockerfile = context / "docker/server/Dockerfile"
    text = dockerfile.read_text(encoding="utf-8")
    marker = 'make -j"$(nproc)"'
    if text.count(marker) != 1:
        raise ValueError("The pinned Dockerfile changed; refusing an unchecked build.")
    dockerfile.write_text(text.replace(marker, f"make -j{jobs}"), encoding="utf-8", newline="\n")
    database_file = context / "docker/database/Dockerfile"
    text = database_file.read_text(encoding="utf-8")
    marker = 'git clone "$VMANGOS_WORLD_DB_REPOSITORY_URL" /build/database'
    if text.count(marker) != 1:
        raise ValueError("The pinned database Dockerfile changed; refusing an unchecked build.")
    database_file.write_text(text.replace(marker, marker.replace("git clone ", "git clone --depth 1 ")),
                             encoding="utf-8", newline="\n")
    shutil.copyfile(PATCH, context / "docker/patches/zz-skatecraft.patch")


def image_matches(image: str) -> bool:
    result = subprocess.run(["docker", "image", "inspect", image], capture_output=True, text=True)
    if result.returncode:
        return False
    actual = json.loads(result.stdout)[0].get("Config", {}).get("Labels") or {}
    return all(actual.get(key) == value for key, value in labels().items())


def build_images(jobs: int = 1) -> dict[str, str]:
    if not 1 <= jobs <= 64:
        raise ValueError("Build jobs must be between 1 and 64")
    images = image_names()
    missing = [kind for kind, image in images.items() if not image_matches(image)]
    if not missing:
        return images
    source = source_checkout()
    with tempfile.TemporaryDirectory(prefix="source-server-build-", dir=ROOT / ".setup") as temporary:
        context = Path(temporary).resolve()
        if not context.is_relative_to((ROOT / ".setup").resolve()):
            raise ValueError("Build context escaped the setup directory.")
        prepare_context(source, context, jobs)
        for kind in missing:
            print(f"Building matching {kind} image ({jobs} jobs)", flush=True)
            command = ["docker", "build", "-t", images[kind], "-f", str(context / f"docker/{kind}/Dockerfile"),
                       "--build-arg", "VMANGOS_REVISION=" + REVISION,
                       "--build-arg", "VMANGOS_FAIL_ON_PATCH_ERROR=1"]
            if kind == "server":
                command += ["--build-arg", "VMANGOS_CLIENT_VERSION=5875"]
            for key, value in labels().items():
                command += ["--label", f"{key}={value}"]
            subprocess.run(command + [str(context)], check=True)
    return images


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=int(os.environ.get("SKATECRAFT_BUILD_JOBS", "1")))
    args = parser.parse_args()
    for kind, image in build_images(args.jobs).items():
        print(f"{kind}: {image}")


if __name__ == "__main__":
    main()
