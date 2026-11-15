#!/usr/bin/env python3
"""Verify the pinned snapshot and the separately reviewable private patch inventory."""
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[2]
manifest = json.loads((root / "patches/doubleentry/manifest.json").read_text())
for name, entry in manifest["files"].items():
    path = root / "vendor/doubleentry" / name
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != entry["patched_sha256"]:
        raise SystemExit(f"source checksum mismatch: {name}")
    if name.startswith("tests/") and entry["upstream_sha256"] != actual:
        raise SystemExit(f"upstream test changed: {name}")
patch = root / "patches/doubleentry/atomic-write.patch"
if hashlib.sha256(patch.read_bytes()).hexdigest() != manifest["patch_sha256"]:
    raise SystemExit("atomic patch checksum mismatch")
key_patch = root / "patches/doubleentry/key-init.patch"
if hashlib.sha256(key_patch.read_bytes()).hexdigest() != manifest["initialization_patch_sha256"]:
    raise SystemExit("key-init patch checksum mismatch")
authority_patch = root / "patches/doubleentry/authority.patch"
if hashlib.sha256(authority_patch.read_bytes()).hexdigest() != manifest["authority_patch_sha256"]:
    raise SystemExit("authority patch checksum mismatch")
multi_patch = root / "patches/doubleentry/multi-op.patch"
if hashlib.sha256(multi_patch.read_bytes()).hexdigest() != manifest["multi_operation_patch_sha256"]:
    raise SystemExit("multi-op patch checksum mismatch")
with tempfile.TemporaryDirectory(prefix="currents-source-") as directory:
    reconstructed = Path(directory)
    for name in manifest["files"]:
        destination = reconstructed / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(root / "vendor/doubleentry" / name, destination)
    subprocess.run(["git", "apply", "--reverse", str(multi_patch)], cwd=reconstructed, check=True)
    subprocess.run(["git", "apply", "--reverse", str(authority_patch)], cwd=reconstructed, check=True)
    subprocess.run(["git", "apply", "--reverse", str(key_patch)], cwd=reconstructed, check=True)
    subprocess.run(["git", "apply", "--reverse", str(patch)], cwd=reconstructed, check=True)
    for name, entry in manifest["files"].items():
        path = reconstructed / name
        if entry["upstream_sha256"] is None:
            if path.exists():
                raise SystemExit(f"patch does not remove added file: {name}")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != entry["upstream_sha256"]:
            raise SystemExit(f"patch does not reconstruct pinned source: {name}")
print(f"Verified {len(manifest['files'])} pinned source files; upstream tests unchanged")
