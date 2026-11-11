# Serialized commands (CUR010)

C03's five Service operations compose the accepted encrypted C04 writer and C05
frozen instructions. This is a synthetic-only private library boundary, not a
signed host, a complete financial domain engine, or production activation.
`Service::new` takes ownership of one EncryptedStore, dataset/instance identity,
HostGrants, CommandPlanner and a fresh TrustedClock. It rejects Production and
awaits the rejected store's close. The trusted owner maps the file to the dataset
and establishes its instance/session identities; caller DTOs cannot do this.

## Trusted seams and dependency direction

Service imports the already-owned C03 DTOs from currents-protocol. Protocol still
depends only on types; service has no SQLx, cipher or accounting dependency.
Storage exposes PreparedCommit, CommitControl, CommitReceipt, command_record and
audited_prefix. These are private typed adapters over existing history and native
transactions, not public SQL, native commands or callbacks. Existing wire types,
schema attestation, source pins and checkpoint codec v1 are unchanged.

CommandPlanner prepares a typed DecisionPlan and C05 AccountingBatchV1 at the
exact supplied AccountingPrefixV1, with effects, warnings, approval scope,
manifest/decision identifiers and exact policy reference. It owns domain meaning
and must reject unsupported operations. Service rejects caller-supplied decision
or manifest records, stamps decision actor/grant/time/command metadata itself and
binds the entire plan in closure_digest. Every frozen native account/posting must
be covered by both the effects and approval account scope. Storage validates the frozen manifest,
its prefix and limits, then publishes effects, result, invalidation declarations,
domain records and native state through the one accepted transaction. There are
no separate result or invalidation authorities. Future projections must consume
the committed invalidation declarations rather than claim they already ran.

HostGrants is a trusted, non-wire interface. authenticate checks the actual current
session, client, dataset, instance, expiry, capability and revocation. Command,
effect and query checks enforce the current scope. authorize_result requires the
original authorized scope or fresh owner authority using the admitted identity
or durable decision provenance and effects; it must cover accounts, record IDs,
amounts and other fields before revealing a digest conflict. Missing-result
lookups also require authorized scope. The host's approval verifier checks the
helper proof against the complete supplied challenge. These are explicit host
obligations, not properties granted by possession of an arbitrary UUID. No
production HostGrants or financial CommandPlanner implementation is shipped.
Synthetic implementations live in integration-test support only.

## Preview and admission

Preview binds the canonical stable body digest, expected head, exact effects,
warnings, approval scope and complete frozen-plan digest. Its internal challenge
also binds random nonce, client, grant, command UUID, dataset, instance, session
and issue time. It lives only in memory. The host publishes it to its approved
helper out of band; it is not a bearer field added to history or PreviewV1.
Verification is read-only so it can repeat at execution. The service consumes the
challenge once at admission. It expires exactly 300 seconds after issuance and
rejects a clock earlier than issuance. A regenerated challenge invalidates the
old one. Only owner-approved commands are supported here; standing-rule adoption
needs its owning internal evaluator and is not silently labeled owner-approved.

Submit authenticates and authorizes before any command lookup. An authorized
same-ID/same-digest durable retry returns the original result before a stale-head
check and without a second approval. A changed digest fails. The admission ledger
serializes duplicate detection and queued/running state. Fresh host time, session,
capability, grant, effects and approval checks run after admission waits and again
when work reaches the single writer. Stale queued plans fail without effects.
Queries authorize before planning/output construction, recheck after writer waits,
and require matching dataset, processed head and full ReadBasis. Historical
projection semantics remain with the query owner; this boundary refuses a stale
basis rather than labeling it current.

Admitted work owns its inputs in a retained task. Dropping the submit future or
losing its reply does not cancel the writer. Completed durable entries leave the
in-memory ledger; history owns their retry identity and scope. Finished task
handles are reaped. Uncommitted settled identities remain bounded tombstones for
this owner lifetime so changing their digest cannot bypass failed admission.
There are at most 1,024 previews and 1,024 non-durable admissions/tombstones, with
128 MiB serialized retained-plan/request budgets for each collection. Capacity
exhaustion rejects before admission; it never evicts an uncertain command. A
closed/reopened audited owner may resolve absent work without retained tombstones.
Message/depth limits remain C03's 1 MiB/depth64; frozen-plan limits do not enlarge
the transport. Live authorization bytes never enter authoritative data.

## Settlement, close and restart

Admission distinguishes queued, running, commit-submitted and settled; the
commit-submitted phase is derived from the native writer's atomic CommitControl.
Canceling queued work can return DefinitelyNotCommitted. Canceling running work
requests rollback and returns Pending until successful rollback is observed.
COMMIT submission wins atomically against cancellation; a completed commit cannot
be undone. Commit/rollback uncertainty returns ResultUnknown, blocks further
mutation and requires closing/reopening the store. It never becomes a negative
result merely because a result row was not observed.

Construction begins unaudited even though encrypted open performs its own
checks. recover completes the accepted structural authority audit, then compares
its exact tip and native checkpoint under the writer lock. Only that completion
can authorize absence. This does not claim the later C06 financial replay engine
is implemented. close stops admission, revokes host sessions, cancels queued and
rollback-capable work, drains submitted COMMIT and awaits all encrypted connection
shutdown before returning. Subsequent client calls are locked; a new owner needs
fresh host authentication. Failure to drain does not claim a fully closed owner.

The four named tests cover dropped replies/new capabilities, changed digests,
unauthorized result probes, blocked writer status, pre-audit absence, stale plans,
queued revocation/expiry, mismatched approvals, scope canaries, actual precommit
rollback, close races, lost COMMIT acknowledgment and subprocess death before
COMMIT settles. Reopen checks native/domain prefixes and durable results. Fixed
fault controls are absent from default production features. Process kill is not
physical power-loss qualification. Synthetic passes enable no production gates.
