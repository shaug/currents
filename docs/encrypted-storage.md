# Encrypted connection initialization

CUR-006 owns the private SQLCipher boundary, not production activation, host
Keychain UI, accounting batch construction or archive/restore semantics.

`DatasetKey::new([u8;32])` accepts a raw host-supplied key without Debug, Clone,
serialization, command-line or environment transport. `CipherProfileV1` has no
mutable settings: SQLCipher 4, 4096-byte pages, HMAC-SHA512, encrypted header,
SQLCipher-managed salt, raw256 key and the target's expected crypto provider.
The runtime requires SQLCipher4.20.0; the fixed source inventory retains
SQLx0.9.0/sys0.37.0 and the reviewed official source adaptation.

`open_pool` is trusted private-library plumbing shared by storage and the
transfer-only library. SQLx options never contain the key; statement and slow
logging are disabled and statement caching is disabled. Every initial, lazy or
replacement pool connection verifies the cipher, applies key/profile before its
first schema access, authenticates existing contents, and enforces WAL/FULL,
foreign keys, recursive triggers and memory-only temp storage. Missing cipher,
wrong key, unsupported provider and rejected initialization fail closed. A failed
initial acquisition explicitly awaits pool close. `CipherPool::close(self)`
awaits all connections and clears the callback's retained secret. Callers must
release acquired connections and await close before runtime teardown.

`EncryptedStore::open(path, key, profile)` creates native metadata through the
accounting adapter before the namespaced Currents atomic-record table. Existing
files must match the reviewed 125-object schema fingerprint before native open;
missing tables/triggers are not silently repaired. The attestation includes a
length-prefixed hash of ordered object type/name/table/SQL and separate migration
source hash. `fixtures/storage/schema-v1.json` is generated evidence of those
objects; changing native schema requires regenerating and reviewing it. The
separate `key-init.patch` only introduces trusted prepared-pool construction.
`atomic-write.patch` and original upstream test/source hashes remain preserved;
`patches/doubleentry/verify.py` reverses both patches to verify provenance.

C01 reserves key/profile/sys ownership to cipher, not all SQLx use. The private
storage and accounting adapters now import exact workspace SQLx for schema and
pool construction. They cannot import sys directly; domain/service/transfer
remain barred from SQLx, and only accounting calls doubleentry. Cipher still
has no owner/domain/accounting dependencies. No raw native handle or SQL enters
a client-facing API.

Run `cargo test --locked -p currents-storage --test cur_006` for four behavioral
assertions. `cargo xtask qualify cipher` additionally verifies source/features,
exact assertion execution and one SQLite/codec definition in the actual host
test executable and transfer static library. Independently run on macOS arm64
(CommonCrypto) and Linux x86_64 (vendored OpenSSL). Gate reports bind the current
host and exact artifact digests; neither cross compilation nor a report from
another host qualifies the local target. On Mac, set `CURRENTS_NM` to an LLVM nm
compatible with the selected Rust LLVM version (for this host,
`/opt/homebrew/opt/llvm/bin/llvm-nm`); older Apple nm cannot inspect new Rust
bitcode members, and that failure is Blocked, not ignored. Its executed version
is recorded in the report. Production gates remain disabled until
all their separate owning prerequisites pass.

Tests use synthetic keys and payloads only. They cover actual DB, WAL, rollback,
SQLite temp, staging, logs and immediate process-exit artifacts, twelve held and
replaced pool connections, a genuinely slow query, and 1,000 independent
runtime open/write/await-close/teardown cycles. This does not certify privileged
memory/core dumps, swap, physical power-loss durability, iOS, Keychain access or
signed deployment. Secret clearing is best effort and cannot erase caller or
native-library copies retroactively. Synthetic success is not permission to
activate real household data.
