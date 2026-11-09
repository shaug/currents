# Private atomic composition patch (CUR-005)

The snapshot is doubleentry 0.9.0 at
`3c514c97169a98d96f0668e6addfd92e1e8f6f69`, with its MIT and Apache-2.0 notices.
`manifest.json` records upstream and patched SHA-256 for every source file.
`atomic-write.patch` is independently reviewable; `python3 patches/doubleentry/verify.py`
checks the snapshot, patch and byte-identical upstream tests/canonical vectors.

The SQLite-only addition accepts one native operation at precision 18 plus a
named bounded canonical Currents record. It cannot accept SQL, a callback, a
connection initializer or a collection of operations. Native planning,
validation, persistence, canonical leaves, schema guard and result types remain
upstream code. Native batch operations retain their existing semantics; this is
not CUR-007 multi-operation closure support.

Native metadata must exist before `currents_initialize` creates the namespaced
composition table. A failed named record write rolls back native changes. Native
rejection (including the native `BatchRejected` result) rolls back the entire
transaction. Exact retry does not append the record twice; changed bytes or a
new native effect under an existing command ID abort. Every rollback is awaited.
Any error after submitting COMMIT is conservatively unknown. Test-only COMMIT
transport faults cover an I/O error before dispatch and loss of acknowledgement
after real commit; these do not certify physical disk/power-loss behavior.

## Staged contracts

This is the CUR-005 transaction seam, not complete authoritative history. The
`currents_atomic_records` table is a private composition substrate. CUR-008 owns
total ordering, domain commit chains, immutable evidence and command-result
storage. C05 frozen batch construction remains CUR-009. The private accounting
adapter serializes existing `AuthoritativeRecordV1` values, keeps native types
opaque, and sanitizes failures. The storage writer preserves `OutcomeUnknown`.
Neither exposes a raw native constructor, SQL hook or plaintext opener. CUR-006
owns encrypted construction. Therefore the aggregate C04
`commit(PreparedCommit)` API is not claimed implemented here.

The standalone upstream test manifest selects the same reviewed SQLCipher source
as the workspace. It runs without keys using synthetic temporary files: this is
not key initialization, encrypted connection qualification or production access.
The only injected I/O modes are compiled under `cfg(test)` inside the native
crate and cannot be selected by production clients or imports.

## Reproduction

- `cargo test --locked -p currents-storage --test cur_005`
- `cargo xtask ticket CUR-005`
- `cargo xtask check quality`
- `cargo xtask check msrv`

The first command runs the actual pinned native boundary assertions and complete
upstream suite with SQLite/testing features. PostgreSQL is outside the selected
runtime. Native full-suite stdout is captured on failure, not replaced by a
hard-coded historical pass count.
