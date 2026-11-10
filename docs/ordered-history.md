# Ordered authority (CUR008)

This private synthetic storage boundary extends C04. It does not implement C05
financial freezing, C10 admission/idempotency/restart resolution, C07 parsing,
semantic replay, archive activation, or a production host. Public clients receive
neither SQL nor native handles. The old storage `write_record` shortcut is retired;
its maintained native/accounting regression fixture remains below this boundary.

## Transaction and order

One initialized SQLCipher file is one dataset. `Writer::append(HistoryAppend)`
accepts canonical typed records, decisions, the expected prior domain head, a
committed result, opaque staged handles, and an optional already-prepared private
native operation. It does not construct native operations. The native-owned opaque
transaction runs at most one operation, obtains its checkpoint in the same
transaction, then performs only named Currents writes. Any failed record/reference
check rolls back the native effect too. Commit/rollback I/O uncertainty is not
reported as definite noncommit. CUR007 will extend native execution to multiple
ordered operations, without changing this transaction ownership.

Positions are all records in vector order, followed by decision envelopes in their
vector order. Explicit decision commit/item/command bindings must agree. Every
record variant is preserved, including anonymous ParseProduct, Allocation and
OccurrenceMatch values. The complete canonical DomainCommitV1 is stored, alongside
ordered item bytes/digests and a reference index. A commit digest is SHA-256 of
RFC8785 canonical DomainCommitV1 with its own `digest` set to 32 zero bytes. It
includes previous_digest, native mapping and result. Item digests and raw content
digests are independent SHA-256 values. SQL guards enforce contiguous sequence,
previous digest, foreign keys, uniqueness, and immutable committed rows, including
replacement attempts. Read/open verifies canonical bytes, item order/digests,
reference index, content digests, commit chain and checkpoint encoding. It never
skips a malformed record.

Readers pin an existing cutoff and one SQL snapshot. `HistoryStream::next` holds
one bounded commit, rather than a whole archive. Convenience `read_prefix` rejects
materialization beyond 64 MiB. Structured items are limited to 1 MiB, commits to
64 MiB and 20,000 items, and normalized observations to 10,000 per commit. These
are storage bounds, not C05's future native operation/closure qualification.

## Native checkpoint codec v1

This small C04/C05 prerequisite is established by CUR008 and consumed by CUR009.
It does not freeze future financial commands, dates, precision or Documents.
The source is pinned doubleentry 0.9.0 commit
3c514c97169a98d96f0668e6addfd92e1e8f6f69. Use its unsigned
`Checkpoint::new(KeyName::from(ledger_id), TreeHead).text()` with origin
`currents/dataset-v1`, no extension lines, and exact UTF-8 bytes (including final
newline). This retains native size and root. `NativeHead.semantics_version = 1`;
`NativeHead.checkpoint = SHA256(exact checkpoint text)`. Persist the text with
each domain commit and verify origin, canonical round trip, empty extensions,
version and digest. Unknown versions fail closed. The version identifies this
pinned checkpoint adapter, not a claim that C05 financial semantics are complete.

Golden vectors in fixtures/contracts/v1/native-checkpoint.json cover an empty
domain history (native genesis), a real monetary append, and unchanged no-effect
mapping. Native and domain sequence numbers are never equated. Native roots are
assembled using Session::size, RootPlan and transaction-local stored nodes, never
by a separate reader that cannot see uncommitted writes.

## Source storage and staging

Staging is in encrypted tables in the same keyed file; no plaintext staging path
or key-bearing URI is accepted. Handles are opaque and not serializable. Files
are bounded to 100 MiB with 1 MiB chunks. Each chunk and whole content is checked
before publication. Identical bytes share one digest-addressed content object,
while every retrieval retains its distinct receipt. Changed content creates a
new immutable observation and prior-version edge. Publishing staged bytes,
receipts, observations, decisions and the receipt's cursor is one transaction.
No receipt or cursor is exposed before commit. Previously published incomplete
sub-batches retain their incomplete/truncated flags; a later complete receipt
never rewrites them. Provider completeness claims and sanitization belong to the
acquisition adapter; storage cannot infer them. Unpublished staging is not
history; later retention owns its approved cleanup.

## Reference inventory and limits

Definitions are indexed by kind, ID and explicit version (zero for unversioned
identities). Distinct definition versions coexist. Anonymous values get no
fabricated IDs. Account, unit, facet, planned-leg and nested posting identities
are indexed from their defining typed records. Stable family references check
existence/kind, not a hidden latest-version selection. Explicit PolicyReference
and forecast commitment versions require that exact version. References resolve
only within this file and at or before the referring commit, including same-
commit forward references. Decision actor/grant and source connection/device IDs
are provenance identities, not nonexistent authoritative actor/device tables.

The exhaustive AuthoritativeRecordV1 match in storage/references.rs documents
record-specific edges. It checks observation predecessor/tombstone, parse source,
facets, allocations, event/leg/association/recognition/segment closure, decision
and review references, policies, account/unit roles, organization and planning
references. A missing ID or wrong kind fails the transaction. ParseProduct and
Allocation remain stored anonymous values. C02/C07 must add exact immutable
parse-product/version references before adoption can distinguish parser reruns
with shared facet lineage. A fabricated RecordId cannot target an anonymous parse
product. Storage does not claim that absent provenance contract or semantic
allocation conservation is implemented.

## Integrity and compatibility

Hash chains detect corruption and disagreement with independently retained
checkpoints. An attacker rewriting every local record and digest consistently
cannot be detected without an independent checkpoint. Tests demonstrate both
sides; no stronger claim is made. SQL triggers protect the supported engine, not
a privileged attacker. Process exit tests do not prove physical power-loss safety.
Schema attestation includes the exact authority migration. Pre-CUR008 files fail
closed rather than receiving an in-place authoritative migration; the later
migration owner must supply backup/staging/activation qualification.

Context-specific policies are not interchangeable. Decision governance pins an
exact DatasetPolicy bundle; that is structural provenance, not an executable
policy-definition claim. HolidayCalendar references are rejected until C02/C12
provide a versioned calendar definition. Forecast assumption references resolve
only to the exact embedded assumption in that saved run, never another run with
the same UUID. Facet references are scoped to their observation, and planned-leg
identities to their commitment family; a matching UUID in another context is not
sufficient. These structural checks do not supply the missing exact parser or
occurrence-generation semantics.

The following is a new coordinator-approved C02 structural clarification, not an
inference from repeated IDs in old shape fixtures: DatasetPolicy opening_policy
and withholding_policy are DatasetPolicy family links. A self-link permits
structural bootstrap only; it does not prove usable adopted rules. Recognition
`date_policy` is explicitly UnsupportedVersion until its owner publishes an
addressable date-policy contract. It is never silently coerced to governance.
