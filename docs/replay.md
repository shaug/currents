# Complete frozen replay

C06 compares the ordered authoritative commits, ordered recognition and frozen
posting data, command-result/native-head identity mapping, and native canonical
leaves/head. Equal balances do not establish correspondence. Domain replay takes
only typed accepted commits. The storage adapter reads retained instruction bytes
and runs their recorded native semantics; it does not ask a current command
planner, wall clock, network adapter, or UUID generator for new decisions.

## Host lifecycle

A trusted host opens the existing source and a fresh encrypted destination, then
calls `ReplayCoordinator::rebuild(source.history(), destination)`. Destination
ownership is consumed and closure is awaited on both success and failure. Source
ownership remains with the caller, which must also await its close. A nonempty
destination is rejected. Failed destinations remain untrusted and are never
activated. Each reconstructed commit must match the complete source commit,
including its intermediate native head, before the next commit proceeds.

`Service::recover_into(destination)` holds the existing serial writer boundary,
withholds the trusted prefix during reconstruction, verifies that the original
source is still valid and unchanged, and publishes the verified prefix only on
success. `recover()` without a destination returns `IncompleteBasis/Recover`.
Unknown COMMIT outcomes still require close/reopen before this recovery step;
replay does not reopen a poisoned live writer. Local replay health is distinct
from source staleness, coverage and freshness. Current queries reject a different
basis or processed head rather than returning silently stale results.

This is not active-generation switching. Rebuilding another file does not make a
damaged original writable. C13/CUR-032 owns subsequent file/key activation.

## Damage and evidence inspection

A forensic source uses an existing-file read-only/query-only cipher connection,
never creates missing files or changes journal mode, and never exposes a writer.
A trust failure accompanies evidence inspection. Required authority/schema,
reference, retained-blob or semantic failures remain failures. Native projection
and leaf damage may be reconstructed from intact authoritative frozen instructions;
all recorded intermediate expected heads still have to match the reconstruction.
An unavailable source leaf observation is explicit, not a fabricated equality.
The rebuilt report contains the destination's real fully audited leaf hash and
retains the source damage in `problems`.

A complete source rewrite together with all checkpoints cannot be detected without
an independent saved checkpoint. These checks are corruption/correspondence
checks, not protection against a privileged attacker rewriting every authority.
Unsupported semantic versions fail explicitly; no record is skipped to create
apparently healthy totals. Snapshot acceleration remains CUR-030-owned.

## Qualification

Run `cargo test --locked -p currents-storage --test cur_011` and
`cargo xtask ticket CUR-011`. The four named assertions cover ordered identity,
date/organization changes despite equal balances, ambient-input independence,
distinct damage classes, and reproducible headless replay hashes. Cross-platform
qualification compares the exact `replay-v1` report on macOS arm64 and Linux
x86_64 at the same source candidate and fixture hashes. Current/MSRV and predecessor
regressions remain separate gates. No production or signed-native readiness is
implied by synthetic replay.

## Synthetic command-line verifier

`currents-verify --synthetic SOURCE DESTINATION` opens an existing read-only
source and reserves a new destination without overwriting. Both use the public
fixture key consisting of 32 bytes of `0x61`. This is deliberately public test
material, not a way to encrypt private financial data. There is no production
mode or key argument/environment input. Direct source symlinks, existing
outputs, output symlinks and same-file destinations are rejected. Output includes
only status and safe correspondence hashes. A reconstructed destination with
source damage is reported as a source trust failure, never a successful source
activation.

Recovery retains the verified latest-record domain projection with its exact
prefix. After each of its own commits, the service moves that projection forward
over the new commit with the same per-commit checks as replay: retained content,
domain records and the accounting index, which must match the committed prefix.
It does not rebuild a second native ledger for that commit. If the step fails,
the projection stays stale and current queries return `IncompleteBasis` until a
complete replay installs the next verified projection. A newly opened service
still needs a complete replay first. The trusted planner cannot bypass that check
by returning only a matching head label. No new query/economic policy is
introduced here.

Association, segment, allocation and occurrence-match records, or recognitions
requiring allocation/leg ownership semantics, currently return
`UnsupportedVersion`: their owning algorithms are not delivered by C06.
`ReadOnlyStore::into_forensic` preserves a failed full-replay result on the same
read-only source; structural audit success alone does not certify these semantics.

Addressed parse products retain their immutable product IDs and supported schema
version in the latest-record projection. Reparse products coexist with prior
products; legacy anonymous products remain only in ordered authority and do not
receive a fabricated ID. Receipt capture-metadata blobs are copied byte-for-byte
with all retained content, preserving capture time independently of receipt time.
