#!/usr/bin/env python3
"""Reject game assets, generated data, binaries and credentials in Git snapshots.

Reads the Git index or committed objects, including intermediate commits in a push.
It does not traverse the working directory or follow the local WoW junction.
"""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import threading

ROOT = Path(__file__).resolve().parents[1]
UPSTREAM_BASE = "2f97dcc8ba1232d1c3e108f1ef0fa6d06fb61fee"
SOURCE_ROOTS = {".cargo", ".github", ".githooks", "crates", "docs", "reference", "scripts", "setup", "third_party", "tools"}
ROOT_FILES = {
    ".gitattributes", ".gitignore", "AGENTS.md", "CONTRIBUTING.md", "Cargo.lock",
    "Cargo.toml", "INSTALL.md", "LICENSE-APACHE", "LICENSE-MIT", "NOTICE", "README.md",
    "TROUBLESHOOTING.md", "play.bat", "play.ps1", "play.sh", "rebuild.bat", "rust-toolchain.toml", "setup.bat", "shell.nix",
}
APPROVED_MEDIA: dict[str, str] = {
    'docs/media/skatebuilder.mp4': '489925d95471fa66ac8ff13db8c91d7f51055b4f76b6ce5ffc9ce6ba210e8dba',
    'docs/media/skatebuilder-preview.png': '9eb891dbe366c8c7ea1e91eb36790ea84e96bc99d772dd9c914286b0b78f3147',
    'docs/media/respawn-flag.mp4': 'b8b22c9ccf05860ab64aaf4394abae23a2e7be146dc9a986689698ab5c1fba8a',
    'docs/media/respawn-flag-preview.png': '3c40f42cd17e48cd7f25cfd0a5f7b6f74a11985bce77ee0c133bc04996f0cf9b',
    'docs/media/new-abilities.png': 'a81d714e6c5bda759f1290fae38a518c954b4b3761fabdb81a9ad6097f7ed1c2',
    'docs/media/object-catalog.png': '23c5799b38997f9840e667706814aebbe2e60fdeae810d76573f369ab2040758',
}
SOURCE_SUFFIXES = {".rs", ".c", ".h", ".toml", ".lock", ".md", ".wgsl", ".py", ".sh", ".ps1", ".bat", ".patch", ".yml", ".yaml", ".nix"}
UI_FILES = {
    "ContainerFrameAdapters.xml", "FrameXMLFixes.xml", "GameMenuAdapters.xml",
    "KeyBindingsPage.xml", "OptionsFrame.xml", "ScriptLogFrame.xml", "ScrollTemplates.xml",
    "SpellBookAdapters.xml", "layer.toc",
}
REFERENCE_FILES = {
    "1.12-event-catalog.tsv", "1.12-event-firesites.tsv", "1.12-events.tsv", "1.12-globals.tsv",
    "1.12-message-catalog.tsv", "1.12-shapes.tsv", "1.12-verb-events.tsv",
}
SECRET_PATTERNS = (
    re.compile(rb"gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}"),
    re.compile(rb"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----"),
    re.compile(rb"AKIA[0-9A-Z]{16}"),
)


def git(*args: str) -> bytes:
    return subprocess.check_output(["git", "-C", str(ROOT), *args], stderr=subprocess.PIPE)


def permitted_path(name: str) -> bool:
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or "\\" in name:
        return False
    if name in APPROVED_MEDIA:
        return True
    if len(path.parts) == 1:
        return name in ROOT_FILES
    if path.parts[0] not in SOURCE_ROOTS:
        return False
    if any(part in {".git", "__pycache__", "fixtures", "benilla-config", "skate-data", "skate-audio", "extracted", "storage"} for part in path.parts):
        return False
    if path.suffix in SOURCE_SUFFIXES:
        return True
    if path.parent.as_posix() == "crates/benilla-app/assets/ui" and path.name in UI_FILES:
        return True
    if path.parent.as_posix() == "reference" and path.name in REFERENCE_FILES:
        return True
    return name in {".githooks/pre-push", "third_party/lua-src/LICENSE", "third_party/kira/LICENSE-MIT", "third_party/kira/LICENSE-APACHE"}


def snapshot(revision: str | None) -> list[tuple[str, str, str]]:
    entries = []
    if revision is None:
        for entry in git("ls-files", "--stage", "-z").split(b"\0"):
            if not entry:
                continue
            header, name = entry.split(b"\t", 1)
            mode, oid, stage = header.decode().split()
            if stage != "0":
                raise ValueError("Resolve merge conflicts before publishing.")
            entries.append((name.decode("utf-8"), mode, oid))
    else:
        for entry in git("ls-tree", "-rz", revision).split(b"\0"):
            if not entry:
                continue
            header, name = entry.split(b"\t", 1)
            mode, kind, oid = header.decode().split()
            if kind != "blob":
                raise ValueError("Submodules are not allowed in the source-only repository.")
            entries.append((name.decode("utf-8"), mode, oid))
    return entries


def audit(revisions: list[str | None]) -> int:
    names: dict[str, set[str]] = {}
    failures = []
    entries = 0
    for revision in revisions:
        for name, mode, oid in snapshot(revision):
            entries += 1
            if mode not in {"100644", "100755"}:
                failures.append(f"{name}: links and submodules are forbidden")
            if not permitted_path(name):
                failures.append(f"{name}: outside the source-only file allowlist")
            names.setdefault(oid, set()).add(name)
    # Git streams blobs out of its object database; never read a local game install.
    proc = subprocess.Popen(["git", "-C", str(ROOT), "cat-file", "--batch"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    def write_queries():
        assert proc.stdin is not None
        proc.stdin.write(("\n".join(names) + "\n").encode())
        proc.stdin.close()
    writer = threading.Thread(target=write_queries)
    writer.start()
    assert proc.stdout is not None
    for _ in names:
        oid, kind, size = proc.stdout.readline().decode().strip().split()
        body = proc.stdout.read(int(size))
        if proc.stdout.read(1) != b"\n" or kind != "blob":
            raise ValueError("Invalid Git blob stream.")
        paths = names[oid]
        media_paths = paths.intersection(APPROVED_MEDIA)
        for name in sorted(media_paths):
            if hashlib.sha256(body).hexdigest() != APPROVED_MEDIA[name]:
                failures.append(f"{name}: documentation media differs from the approved content")
            if int(size) >= 100 * 1024 * 1024:
                failures.append(f"{name}: documentation media exceeds GitHub's per-file limit")
        # An approved video/image cannot be copied to a source path to bypass the gate.
        source_paths = paths.difference(APPROVED_MEDIA)
        if not source_paths:
            continue
        name = sorted(source_paths)[0]
        if int(size) > 2_000_000:
            failures.append(f"{name}: exceeds the source file size limit")
        try:
            body.decode("utf-8")
            if b"\0" in body:
                raise ValueError("NUL")
        except (UnicodeError, ValueError):
            failures.append(f"{name}: binary or non-UTF-8 content is forbidden")
        if any(pattern.search(body) for pattern in SECRET_PATTERNS):
            failures.append(f"{name}: credential material detected (value withheld)")
        if any(path.endswith(".patch") for path in source_paths) and b"GIT binary patch" in body:
            failures.append(f"{name}: binary patch payload is forbidden")
    writer.join()
    if proc.wait():
        raise ValueError("Git could not read the source objects.")
    if failures:
        for failure in sorted(set(failures)):
            print("REJECTED:", failure, file=sys.stderr)
        return 1
    media_count = sum(bool(paths.intersection(APPROVED_MEDIA)) for paths in names.values())
    print(f"Source audit passed: {len(revisions)} snapshot(s), {entries} paths, {len(names)} unique blobs ({media_count} approved documentation media); no game assets or credential patterns.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    choices = parser.add_mutually_exclusive_group()
    choices.add_argument("--index", action="store_true", help="Audit the staged snapshot (default).")
    choices.add_argument("--revision", help="Audit a committed snapshot.")
    choices.add_argument("--since", help="Audit every commit after this revision, through HEAD.")
    choices.add_argument("--pre-push", action="store_true", help="Read Git pre-push refs from stdin and audit every new snapshot.")
    args = parser.parse_args()
    if args.pre_push:
        revisions = set()
        for line in sys.stdin:
            _, local_oid, _, _ = line.split()
            if set(local_oid) == {"0"}:
                continue
            # Also audit the tip when there are no commits beyond the original base.
            revisions.add(local_oid)
            revisions.update(git("rev-list", local_oid, "^" + UPSTREAM_BASE).decode().splitlines())
        return audit(sorted(revisions)) if revisions else 0
    if args.since:
        revisions = git("rev-list", "HEAD", "^" + args.since).decode().splitlines()
        return audit(revisions or ["HEAD"])
    return audit([args.revision] if args.revision else [None])


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (subprocess.CalledProcessError, ValueError) as error:
        print(f"Source audit failed: {error}", file=sys.stderr)
        raise SystemExit(1)
