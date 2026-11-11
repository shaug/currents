# Pinned SQLx worker lifetime adaptation

CUR-006/C04 requires completion of native SQLCipher shutdown before a rejected,
timed-out or cancelled initialization returns, including replacement acquisitions.
SQLx 0.9.0 discards its native JoinHandle; `close_hard()` only drops the sender,
and pool close cannot recover raw connections rejected before pool registration.
The public callback cannot recover those workers. Only `sqlx-sqlite` is adapted;
SQLx core, pool accounting, native accounting and the client boundary are unchanged.

`vendor/sqlx-sqlite` is the complete official 0.9.0 crate, with immutable source
commit `003b698e99e024f3621b8043a2426fde5b741171` and SHA-256
`488e99c397a62007e4229aec669a179816339afc6d2620ca6fa420dbee2e982c`.
`manifest.json` inventories every upstream and adapted byte. The separate
`worker-lifetime.patch` reverses exactly to that source. Run `python3
patches/sqlx-sqlite/verify.py`. The crate's original license-reference files are
preserved; complete MIT and Apache-2.0 texts here are copied from the independently
checksum-verified official SQLx 0.9.0 facade archive in the existing pin inventory.

The default-off `join_worker_on_drop` option is enabled only by
`currents_cipher::connection_options`. It retains a native JoinHandle in a final
RAII field, after the command sender and shared native state. Startup cancellation
first drops its publication receiver, then joins. The worker never owns its own
JoinHandle: transfer occurs synchronously in the receiving future. Normal async
close still sends Shutdown and awaits acknowledgement, then joins the thread tail.
Cancelling close or dropping/rejecting a connection synchronously joins instead.
Destructor panic is not propagated; explicit close reports a crashed worker.

This fallback blocks the dropping thread until native work completes. Acquisition
timeout is therefore not a strict wall-clock bound. Currents uses a finite native
busy timeout and no extension, shared-cache or user callback APIs, but arbitrary
external callbacks/extensions or raw-handle ownership can prevent teardown.
This is not a universal bounded-shutdown claim. Callers must release borrowed
connections and explicitly await pool close before runtime teardown.

Standalone qualification uses the checked-in driver lockfile and its explicit
sys path patch plus platform-specific test-only CommonCrypto/vendored OpenSSL
selection. It never uses registry SQLite. Run from the repository root:

```
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}" cargo test --locked --manifest-path vendor/sqlx-sqlite/Cargo.toml --lib --features bundled,unlock-notify,time -- --test-threads=1
```

`cargo xtask qualify cipher`, full core/quality, and MSRV gates execute the driver
suite. It includes unchanged upstream unit tests and cfg(test)-only worker probes
with separate subprocess watchdogs. A native controller, independent of the
current-thread Tokio runtime, releases startup/exit gates and real DELETE-journal
`BEGIN EXCLUSIVE` locks only once joining begins. Tests cover startup cancellation,
a real wrong-key encrypted fixture, initial/replacement acquisition timeout,
caller cancellation, row backpressure and close cancellation. Each asserts every
created worker joined before the return/close boundary, immediate runtime teardown
and keyed reopen. A separate private cipher unit test uses its real two-second
acquisition deadline and three-second exclusive lock; disabling the actual
`connection_options` opt-in makes it fail before lock release. The original four CUR-006 assertions and 1,000-cycle suite remain.

The only non-lifecycle source hunk is an explicit `expect(deprecated)` on the
unchanged upstream `types::time::formats` module. Vendoring removes Cargo's
registry lint cap and exposes its 40 time 0.3.55 deprecation diagnostics. This
module-level, reasoned compatibility expectation preserves exact parser behavior;
it does not relax workspace warnings, change time versions, or suppress lifecycle
diagnostics. The upstream seven unit tests are unchanged.
