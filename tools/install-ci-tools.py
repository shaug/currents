#!/usr/bin/env python3
"""Install qualified CLI tools from official checksum-pinned artifacts, no credentials."""
import hashlib
import io
import json
import pathlib
import platform
import subprocess
import tarfile
import urllib.request

root = pathlib.Path(__file__).resolve().parent.parent
inventory = json.loads((root / 'tools/versions.lock.json').read_text())
destination = root / 'target/ci-tools/bin'
destination.mkdir(parents=True, exist_ok=True)
platform_key = {('Darwin', 'arm64'): 'darwin_arm64', ('Linux', 'x86_64'): 'linux'}.get((platform.system(), platform.machine()))
if platform_key is None:
    raise SystemExit('BLOCKED: unqualified CI tool platform')
for name in ('actionlint', 'gitleaks'):
    matches = [a for a in inventory['artifacts'] if a['name'].startswith(name + '_') and platform_key in a['name'] and a['name'].endswith('.tar.gz')]
    if len(matches) != 1:
        raise SystemExit('BLOCKED: ambiguous tool artifact')
    item = matches[0]
    data = urllib.request.urlopen(item['source'], timeout=60).read()
    if hashlib.sha256(data).hexdigest() != item['sha256']:
        raise SystemExit('BLOCKED: tool checksum mismatch')
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        member = archive.getmember(name)
        if not member.isfile():
            raise SystemExit('BLOCKED: tool must be a regular file')
        (destination / name).write_bytes(archive.extractfile(member).read())
        (destination / name).chmod(0o755)
subprocess.run(['cargo', 'install', 'cargo-deny', '--version', '=0.20.2', '--locked', '--root', str(destination.parent)], check=True)
print(destination)
