#!/usr/bin/env python3
"""Verify every vendored file and reverse the isolated worker-lifetime patch."""
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[2]
base = root / "patches/sqlx-sqlite"
manifest = json.loads((base / "manifest.json").read_text())
vendor = root / "vendor/sqlx-sqlite"
pins = json.loads((root / "tools/versions.lock.json").read_text())
upstream = next(p for p in pins["artifacts"] if p["name"] == "sqlx-sqlite")
if any(upstream[k] != manifest[k] for k in ("version", "source", "sha256")):
    raise SystemExit("SQLx source pin differs from inventory")
files = {str(p.relative_to(vendor)) for p in vendor.rglob("*") if p.is_file()}
if files != set(manifest["files"]):
    raise SystemExit("SQLx file inventory differs from vendored tree")
for name, entry in manifest["files"].items():
    if hashlib.sha256((vendor / name).read_bytes()).hexdigest() != entry["patched_sha256"]:
        raise SystemExit(f"SQLx checksum mismatch: {name}")
patch = base / "worker-lifetime.patch"
if hashlib.sha256(patch.read_bytes()).hexdigest() != manifest["patch_sha256"]:
    raise SystemExit("SQLx patch checksum mismatch")
for name, digest in manifest["license_files"].items():
    if hashlib.sha256((base / name).read_bytes()).hexdigest() != digest:
        raise SystemExit(f"SQLx license checksum mismatch: {name}")
with tempfile.TemporaryDirectory(prefix="currents-sqlx-source-") as directory:
    reconstructed = Path(directory)
    for name in files:
        destination = reconstructed / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(vendor / name, destination)
    subprocess.run(["git", "apply", "--reverse", str(patch)], cwd=reconstructed, check=True)
    for name, entry in manifest["files"].items():
        path = reconstructed / name
        if entry["upstream_sha256"] is None:
            if path.exists():
                raise SystemExit(f"SQLx patch leaves added file: {name}")
        elif hashlib.sha256(path.read_bytes()).hexdigest() != entry["upstream_sha256"]:
            raise SystemExit(f"SQLx patch does not reconstruct upstream: {name}")
print(f"Verified sqlx-sqlite {manifest['version']}: {len(files)} files, reversible worker-lifetime patch")
