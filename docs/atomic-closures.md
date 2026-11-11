# Atomic native closures (CUR007)

This private C04 boundary extends the accepted ordered authority transaction.
It does not implement service admission/grants, financial intent freezing,
production activation, or institutional mutations. C05 validates frozen instructions, and `AccountingAdapter::prepare_closure`
constructs opaque prepared operations in ordered chunks of at most 600 records; no public native constructor, SQL callback, or raw transaction is
available to clients.

## Execution and visibility

`Writer::append_closure` accepts the existing typed `HistoryAppend`, opaque
`PreparedNativeOperations`, and one `ClosureControl`. The legacy optional native
operation must be absent for this entrypoint. One native-owned transaction locks
the dataset and resolves a prior command before checking a stale head or applying
native effects. Each top-level native operation rereads the full header and
constructs a fresh validator snapshot against that same transaction. Native
validation, canonical leaf encoding and persistence are unchanged. A native
batch retains its 1,000-item limit. Unchanged native success is accepted, while
an unchanged `BatchRejected` rolls back the entire closure.

Native size, account count, entry count, watermark, last booking and period
metadata are refreshed between operations. The final checkpoint is captured
from transaction-local nodes after the last operation and before named domain
publication. Checkpoint codec v1 remains unchanged. The native execution phase
closes when authority insertion starts; a failed native operation cannot be
followed by a successful partial COMMIT. Readers see old or final committed
history, never intermediate closure effects.

## Bounds and staged instructions

Count logical native operations, including every item inside native batches,
against the 20,000-operation closure limit. Count the actual canonical frozen
instruction bytes against 67,108,864 bytes. Both bounds are inclusive. Native
posting, Document and canonical leaf limits remain independently enforced.

These are not the storage limits on structured items, domain commits or staged
source files. Large instruction bytes use the existing encrypted staging/chunk
mechanism rather than a plaintext temporary manifest or a larger inline record.
C05 owns the frozen instruction representation and its bounded authoritative
manifest. Storage requires exactly one manifest and verifies its canonical
instruction content digest/byte count, separate prebinding digest and fixed
expectations through the accepted C05 decoder. Inside the locked transaction,
its complete prefix must match the prior domain sequence/digest and native head.
The legacy single-operation append cannot publish a frozen manifest. Storage
does not select new financial semantics.

## Retry, cancellation and uncertain outcomes

A persisted command lookup checks canonical bytes, commit digest, command ID,
result state and result commit binding. Same-ID/digest retry returns its original
result before stale-head rejection and native execution; changed digest fails.
Closure retry also requires canonically identical records and decisions, including
the entire manifest. Canonical wire identity handles normalized Amount values
consistently even when their original in-memory representation differs.
`command_result` returning `None` is not proof that an admitted operation settled.
Await the writer or reopen and audit after a crash before concluding noncommit.
Service authentication and durable admission tracking remain CUR010.

`request_cancel` requests rollback before COMMIT submission; it is not itself a
rollback receipt. The writer checks cancellation between native operations and
atomically arbitrates cancellation against COMMIT submission. Only an awaited
successful rollback supports `DefinitelyRolledBack`. A cancellation racing a
submitted COMMIT cannot promise rollback. Commit/rollback I/O uncertainty stays
`OutcomeUnknown`, followed by reopened/audited command lookup or identical retry.
Progress counts are nonauthoritative.

## Synthetic qualification

The four named assertions are registered separately from predecessor gates.
The end-to-end cases freeze real C05 metadata and 600 originals, then exercise
600 reversals plus 600 replacements, canonical sequential replay,
independent reader visibility, final-chunk validation failure, actual SQLite
`SQLITE_FULL`, subprocess kill, confirmed precommit
cancellation and lost COMMIT acknowledgment. Current evidence must identify the
exact source, toolchain and encrypted Mac/Linux target; old spike results do not
qualify this extension.

Fixed fault hooks are gated by the existing synthetic-test-fixtures feature
and native testing feature. They accept neither caller SQL nor callbacks and
are absent from production defaults. SQLite `max_page_count` is set only on the
actual synthetic writer transaction's pager at the final chunk: a different
reader connection would not exercise the writer's allocation failure. This is
real SQLite database-size exhaustion, not a claim to fill the host filesystem.
Process kill demonstrates crash recovery, not physical power-loss durability.

CUR010's private typed Writer::commit adapter and admission lifecycle are described
in [serialized commands](serialized-commands.md). Its final cancellation check has
an additional synthetic-only pause before COMMIT, after native/domain work, for
service-level rollback and close-race assertions. Production cancellation and
COMMIT arbitration remain the same atomic control, with no raw callback.
