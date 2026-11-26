#!/usr/bin/env python3
"""Install qualified CLI tools from official checksum-pinned artifacts, no credentials."""
import hashlib
import io
import json
import pathlib
import platform
import sys
import tarfile
import urllib.request

# Only the dependency-policy gate runs cargo-deny; other jobs skip its download.
arguments = sys.argv[1:]
if arguments not in ([], ['--without-cargo-deny']):
    raise SystemExit('usage: install-ci-tools.py [--without-cargo-deny]')
root = pathlib.Path(__file__).resolve().parent.parent
inventory = json.loads((root / 'tools/versions.lock.json').read_text())
destination = root / 'target/ci-tools/bin'
destination.mkdir(parents=True, exist_ok=True)
platform_key = {('Darwin', 'arm64'): 'darwin_arm64', ('Linux', 'x86_64'): 'linux'}.get((platform.system(), platform.machine()))
if platform_key is None:
    raise SystemExit('BLOCKED: unqualified CI tool platform')


def install(item, member_name, name):
    """Downloads one pinned archive, checks its SHA-256 and installs one regular file from it."""
    data = urllib.request.urlopen(item['source'], timeout=60).read()
    if hashlib.sha256(data).hexdigest() != item['sha256']:
        raise SystemExit('BLOCKED: tool checksum mismatch')
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        member = archive.getmember(member_name)
        if not member.isfile():
            raise SystemExit('BLOCKED: tool must be a regular file')
        (destination / name).write_bytes(archive.extractfile(member).read())
        (destination / name).chmod(0o755)


for name in ('actionlint', 'gitleaks'):
    matches = [a for a in inventory['artifacts'] if a['name'].startswith(name + '_') and platform_key in a['name'] and a['name'].endswith('.tar.gz')]
    if len(matches) != 1:
        raise SystemExit('BLOCKED: ambiguous tool artifact')
    install(matches[0], name, name)
if not arguments:
    # The official release archive, built by cargo-deny's own release workflow.
    stem = {'darwin_arm64': 'cargo-deny-0.20.2-aarch64-apple-darwin',
            'linux': 'cargo-deny-0.20.2-x86_64-unknown-linux-musl'}[platform_key]
    matches = [a for a in inventory['artifacts'] if a['name'] == stem + '.tar.gz']
    if len(matches) != 1:
        raise SystemExit('BLOCKED: ambiguous tool artifact')
    install(matches[0], stem + '/cargo-deny', 'cargo-deny')
print(destination)
