# Shared implementation contracts

These names and boundaries are requirements for future implementation, not existing APIs. Read [the technical design](../technical-design.md) first. Each owning ticket publishes its exact versioned schema and golden examples before consumers merge. A referenced contract is part of each issue's scope; consumers may not invent an alternative signature or change semantics. Private storage types are never wire types. Rust functions below return `Result<T, EngineError>` unless the signature explicitly says otherwise. Collections are bounded by the design, not implicitly unlimited. Async functions own their inputs across await points.

<a id="c01"></a>

## C01: Workspace and verification, owner CUR-001

Files: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`, `xtask/src/main.rs`, `xtask/src/checks.rs`, `tools/versions.lock.json`.

Workspace members: `currents-types`, `currents-protocol`, `currents-cipher`, `currents-transfer`, `currents-domain`, `currents-accounting`, `currents-storage`, `currents-service`, `currents-planning`, `currents-acquisition`, `currents-ffi`, `currents-cli`, `currents-mcp`, `currents-verify`, and `xtask`. All are `publish=false`. Types has no storage/platform/runtime dependencies; domain and planning depend on types; accounting depends on types and the patched kernel; storage depends on types/accounting; service composes domain, planning and storage; acquisition yields typed evidence to service, never writes storage; protocol converts types; FFI/CLI/MCP call service or the Mac transport. The verifier composes storage and replay without Apple APIs/network. Only accounting calls native accounting APIs. Private currents-cipher owns the reviewed SQLCipher connection profile/key initialization shared by storage and transfer; it depends only on SQLx/sys and secret wrappers, not service/domain/accounting. currents-transfer is an iOS transfer-only staticlib depending on types/protocol/cipher and serialization, never owner/service/domain/accounting. Platform glue may call system crypto, never implement accounting.

`cargo xtask check <lane>` executes explicit checked-in commands for `quality`, `core`, `msrv`, `native`, `security`, `deep`, `release`, `all`. `cargo xtask ticket CUR-NNN` dispatches the owning issue's exact listed test command(s) and fails on zero matching tests or unavailable prerequisites. `cargo xtask qualify <gate>` produces a versioned pass/fail report plus inputs and evidence, never a fabricated green result. `cargo xtask gates --profile private-activation --prerequisites-for CUR-048` checks only signed-host/key/restore/inventory prerequisites. `cargo xtask gates --profile production --prerequisites-for CUR-054` checks every final-release prerequisite but excludes CUR-054 itself. The CUR-054 suite runs next and persists its own QualificationReportV1; only then does `cargo xtask gates --profile production` audit all results. Profile checks never invoke themselves or count absent reports as passed. Commands are created by their owning tickets, registered centrally, and are not available yet. No implementation-era command is claimed executed by this plan.

<a id="c02"></a>

## C02: Foundational values and record schema, owner CUR-002

Files: `crates/currents-types/src/{ids,amount,time,records,errors}.rs`, `crates/currents-protocol/src/{v1,canonical}.rs`, `schemas/v1/*.json`, `fixtures/contracts/v1/*.json`.

ID newtypes over UUID: `DatasetId`, `InstanceId`, `CommandId`, `RecordId`, `AccountId`, `UnitId`, `LegId`, `RecognitionId`, `CommitmentId`, `OccurrenceId`, `GrantId`, `SessionId`, `PairingId`; `CommitSeq(u64)` serializes as a decimal string; `Digest([u8;32])` as lowercase hex. `Amount { coefficient:i128, scale:u8, unit_id:UnitId }`; `UnitDefinition { id, exponent:u8, native_code:String, kind:Currency|Security|Analytical }`. `Amount::checked_rescale(scale:u8)->Result<Amount,EngineError>` and `canonical_bytes<T:Serialize>(&T)->Result<Vec<u8>,EngineError>` enforce design sections 5/11/16. Arithmetic multiplies/divides through bounded `BigInt` intermediates, then checked i128 conversion; do not deserialize arbitrary huge integers before size limits.

`TemporalValue = Date { civil, original_text } | Instant { utc, original_offset, original_zone, precision, original_text } | Unknown { original_text }`; `DateScope { start:Option<Date>, end:Option<Date>, basis:Effective|Posting }`; `ReadBasis { dataset_id, through_commit, effective_scope, evaluation_instant, query_semantics_version, selection_policy_version, unit_catalog_version }`. Dates use `time::Date`; instants use `time::OffsetDateTime`; source unknown/null/absent values are distinct enums. IANA timezone data and calendar definitions are explicit versioned input, not an ambient conversion.

`AuthoritativeRecordV1` is a tagged enum for dataset/policy/catalog definitions, account components/aliases/perimeter, import receipts, observation versions, parse products, economic events/legs, associations/allocations, decisions/recognitions/segments, reconciliation claims, organization records and assignments/rules/review, commitments/matches/scenarios/forecast runs. C10 gives the exact `OrganizationAssignmentV1` schema. CUR-002 creates schemas for every required field in design section 3 and the owning subsystem extends its variant before use. A semantic version change never silently ignores unknown variants. `DomainCommitV1 { seq, previous_digest, records:Vec<AuthoritativeRecordV1>, decisions:Vec<DecisionEnvelopeV1>, native_head_after, command_result, digest }`; decision envelopes carry actor/grant identity, recorded time, policy, command/digest and supersession, never bearer authorization. `DomainState` is the replayed typed state indexed by stable IDs; it is not authoritative by itself. `DecisionPlan { records, invalidations:Vec<RecordId>, accounting:AccountingIntent }` is a proposed effect, never writes by itself. `AccountingIntent` is a typed enum of metadata declaration, recognition, correction and no-effect; the frozen native form is C05.

`EngineError` has the stable classes listed in design section 10, affected scope and safe next action; no raw SQL, secrets, path, or private excluded fields in external errors. `Health = Healthy | SourcePartial | SourceStale | Locked | ProjectionLagging | IntegrityFailed | MigrationRequired | RecoveryInProgress`; independent source conditions may coexist.

CUR-008 structural reference clarification (new foundational contract): `DecisionEnvelopeV1.policy` pins exact `DatasetPolicy(id,version)`. `DatasetPolicy.opening_policy` and `withholding_policy` are family links to DatasetPolicy, not selection of a latest revision or executable policy definitions; bootstrap self-links prove closure only. C05 adds an exact versioned DateConversionPolicy role for Recognition date-policy references; HolidayCalendar remains unsupported until its owner supplies an addressable definition. Forecast assumption versions resolve only inside that saved run's expanded assumption snapshots. No date/calendar/adoption semantics are implied. See [ordered history](../ordered-history.md).

<a id="c03"></a>

## C03: Protocol, preview and service, owners CUR-002, CUR-010

The implemented synthetic C03 lifecycle and private trusted-host/planning seams
are specified in [serialized commands](../serialized-commands.md). Service consumes
the existing protocol DTOs through service→protocol→types; it has no direct
accounting, SQLx or native dependency. Public wire DTOs are unchanged. Actual
financial planners, signed-host authority and production activation remain with
their named owners.

`CommandBodyV1 { protocol_version, dataset_id, command_id, expected_head, operation:OperationV1 }`; `AuthorizationEnvelope { session, capability, approval }` is nonserializable into history. Operation variants are the command families in design section 10 plus explicit setup/intake variants `SetAccountCatalog(SetAccountCatalogV1)`, `SetDatasetPolicy(SetDatasetPolicyV1)`, `RecordManualPosition(ManualPositionV1)` and `RecordLoanPayment(LoanPaymentV1)`; their payloads use the typed contract below. `RequestV1 = Preview(CommandBodyV1) | Submit { body, authorization } | Query(QueryV1) | GetCommandResult { dataset_id, command_id, digest } | Cancel { command_id }`. Queries carry `ReadBasis` and scope. `CommandResultV1 { command_id, digest, state:Committed|Pending|ResultUnknown|DefinitelyNotCommitted, commit:Option<CommitSeq>, effects, health }`. `PreviewV1 { digest, expected_head, effects, warnings, approval_scope, closure_digest }`. `RequestContext { authenticated_client, session, grant_id, instance_id }` is supplied by the trusted host, not caller DTOs. `TrustedClock::now()->OffsetDateTime` is a host adapter sampled freshly for authorization at admission and queued execution. Bind-time timestamps and saved ReadBasis evaluation_instant cannot establish current capability/preview expiry. Test clock injection is private synthetic-only.

`Service::preview(ctx:RequestContext,body:CommandBodyV1)->Result<PreviewV1,EngineError>`; `async Service::submit(ctx,body,authorization)->Result<CommandResultV1,EngineError>`; `async Service::command_result(ctx,id:CommandId,digest:Digest)->Result<CommandResultV1,EngineError>`; `async Service::cancel(ctx,id)->Result<CommandResultV1,EngineError>`; `async Service::query(ctx,query:QueryV1)->Result<QueryResponseV1,EngineError>`. `QueryResponseV1` is a tagged union of the typed outputs below with processed head, basis and health. Admission records distinguish queued/running/commit-submitted/settled. Missing result is not definite noncommit until admitted work settles and restart audit completes. Identical-ID retry precedes stale-head check after authorization; changed digest errors. All DTOs have canonical golden vectors, depth/size limits and no lossy JSON numeric sequences.

<a id="c04"></a>

## C04: Private transactional storage, owners CUR-005–008

Files: `crates/currents-storage/src/{cipher,writer,schema,history,staging}.rs`, `crates/currents-storage/migrations/*.sql`, `vendor/doubleentry/`, `patches/doubleentry/{manifest.json,atomic-write.patch,key-init.patch,multi-op.patch}`.

`DatasetKey` is a non-Debug, zeroizing secret newtype acquired by the host. `CipherProfileV1` fixes SQLCipher 4/page4096/HMAC-SHA512/encrypted-header/raw256 and crypto-provider identity. `async EncryptedStore::open(path:&Path,key:DatasetKey,profile:CipherProfileV1)->Result<EncryptedStore,EngineError>` is private host/storage API; before all pool schema access it sets profile/key and proves cipher identity. `async EncryptedStore::close(self)->Result<(),EngineError>` awaits every connection. `StagedBatchHandle` is a private storage token for bounded encrypted content. `ValidatedEvidenceBatchV1 { digest, observations, receipts, coverage, cursor }` lives in currents-types and contains validated immutable records, not paths or storage handles. Service resolves a StagedBatchHandle to this typed batch before invoking the pure domain function; chunks remain accessed by typed content references. `HistoryReader::read_prefix(through:CommitSeq)->Result<Vec<DomainCommitV1>,EngineError>` has a streaming equivalent for large histories; no full archive-sized materialization.

CUR-006 implementation details and qualification limits are in [encrypted storage](../encrypted-storage.md). `DatasetKey` and the immutable `CipherProfileV1` live in private currents-cipher; its `CipherPool` is trusted adapter plumbing with consuming awaited close. Storage owns `EncryptedStore` and schema attestation; accounting alone constructs the native store from the initialized pool. These private adapters may import the workspace's exact SQLx dependency for C04, but only cipher imports sys or initializes keys/profiles. No raw native handle, SQL or callback is exported to clients. Aggregate history/batch and production-host activation remain with their named owners.

`async Writer::commit(prepared:PreparedCommit)->Result<CommitReceipt,CommitFailure>` privately composes named Currents-table writes and frozen native ops on one SQLx transaction. `PreparedCommit { body, decision_plan, frozen_batch, prior_head, result }`; `CommitReceipt { commit, native_head, result }`; `CommitFailure = DefinitelyRolledBack(EngineError) | OutcomeUnknown(CommandId)`. No external callback or SQL. Each next op must see prior uncommitted state and refreshed native metadata; native validator/persistence remain unchanged. Imports with no native ops map unchanged native head. `NativeHead` checkpoint codec v1 is the narrow C04/C05 prerequisite established by CUR-008 in [ordered history](../ordered-history.md): pinned unsigned canonical native checkpoint text, SHA-256 digest, semantics_version=1; CUR-009 consumes it. Canonical leaves and frozen operation semantics remain selected by C05; outer domain sequencing never assumes one-to-one native commits.

<a id="c05"></a>

## C05: Accounting adapter, owner CUR-009

Versioned C05 wire instructions, bounded content manifests, exact date-role definitions and source-prefix-bound nonauthoritative indexes are specified in [accounting freeze](../accounting-freeze.md). Native checkpoint codec v1 is unchanged. C04 executes only validated opaque operations. Large instructions are encrypted content references, never oversized inline records.

Files: `crates/currents-accounting/src/{batch,numbers,catalog,dates,templates}.rs`.

`AccountingBatchV1 { native_semantics, operations:Vec<FrozenNativeOperationV1>, preconditions, decision_digest, result_expectations }` freezes the tagged native command payloads, IDs, accounts, unit mappings, dates and Documents at the pinned native semantics; no native memo salt. `AccountingAdapter::freeze(intent:&AccountingIntent,state:&DomainState)->Result<AccountingBatchV1,EngineError>` and `AccountingAdapter::validate_limits(batch:&AccountingBatchV1)->Result<(),EngineError>`. Prebinding hash excludes itself, generated Document hash and outer/native result fields; native document is added afterward. Exact one `SqlStore<Sqlite,18>`; native metadata/policy date floor 0001, end 9999, no seals/drift maximum, immutable paths/unit exponents. USD exponent 2. Bookkeeping date mapping and reversal dates follow design section 16. Metadata declarations commit before financial preview. Enforce batch 1,000/postings 1,024/documents 16/leaf 65,535 bytes; atomic closure 20,000 operations/64 MiB; large closure uses encrypted staged digest. No raw native public escape hatch.

<a id="c06"></a>

## C06: Replay, owner CUR-011; snapshots CUR-030

Files: `crates/currents-domain/src/replay.rs`, `crates/currents-service/src/replay.rs`, `crates/currents-storage/src/{audit,rebuild,snapshot}.rs`.

`ReplayEngine::apply(state:&mut DomainState,commit:&DomainCommitV1)->Result<(),EngineError>` uses frozen decisions only; `async ReplayCoordinator::rebuild(source:HistoryReader,destination:EncryptedStore)->Result<ReplayReport,EngineError>` lives in currents-service; verifier composes it headlessly. Domain ReplayEngine only applies typed records with no storage dependency; `ReplayReport { through, domain_hash, posting_hash, native_head, native_leaf_hash, identity_map_hash, problems }`. `SnapshotV1 { dataset_id, through, prefix_digest, schema_version, projector_version, replay_state, state_hash, basis }`; `SnapshotManager::select(prefix:&HistoryReader,candidate:SnapshotV1)->Result<Option<DomainState>,EngineError>` discards incompatible caches but never masks corrupt authority. Native ledger always full replay until a separately proven restoration API exists. Create at 10,000 records/clean close 1,000 tail; monthly unlocked full comparison; restore/upgrades full replay.

A commit that records a reconciliation claim, confirmation or dispute holds
exactly that one record and its frozen manifest, one `Reconciliation`
decision and no invalidations. Replay derives the claim again with
`ReconciliationEngine::record` from a request rebuilt from the claim, or the
confirmation or dispute again with `confirm` or `dispute`, at the prior
history, and rejects the commit when the derived record differs. Replay
rejects a confirmation whose claim no longer applies at the prior history.

<a id="c07"></a>

## C07: Acquisition and source identity, owners CUR-012, CUR-015, CUR-041

Files: `crates/currents-acquisition/src/{adapter,capture,parse,identity,refresh}.rs`, `crates/currents-domain/src/{evidence,relationship_evidence,relationship_parse,associations,association_admission,association_duplicates,expiry}.rs`.

`AdapterCapabilitiesV1` defines endpoint allowlist, credential scopes, IDs/revision order, deletion/lifecycle, history, units/components/balance semantics, paging and freshness; unknown is explicit. `CaptureBatchV1 { connection_id, source_identity, parser_version, coverage, cursor, observations, sanitized_contract, original_capture_time }`; `ParseProductV1 { source_version_id, parser_id, parser_version, facets:Vec<MovementFacetV1> }`; `MovementFacetV1 { lineage_key, kind, amount, relationships, temporal }`. Facet lineage survives parse/provider revisions; payroll has separate net/gross/withholding/contribution facets.

CUR012 adds the compatible authoritative `AddressedParseProductV1 { id:RecordId, schema_version:u32, product:ParseProductV1 }` under the `AddressedParseProduct` record kind. Schema versions 1 and 2 are supported. IDs are frozen trusted acceptance input and distinct from source/facet IDs. This is a new explicit decision/evidence reference role with same/prior-prefix closure; legacy unaddressed `ParseProductV1` remains unchanged and gains no fabricated address. The bounded in-process profile/context/batch types and rejection policy are documented in [synthetic import profiles](../formats/import-profiles.md). No provider precedence policy is introduced.

### CUR-017 balance kind

Schema version 2 adds `balance_kinds:Vec<BalanceFacetKindV1 { facet:RecordId, kind:BalanceKindV1 }>`, where `BalanceKindV1` is `Posted`, `Available`, `Statement` or `Unknown`. A version-2 product names exactly one kind for each `Balance` facet, in ascending facet ID order, and none for any other facet. A version-1 product has no `balance_kinds` field, keeps its canonical bytes and admission rules, and reports every balance as `Unknown`; its wire form with the field is refused. Every other version is refused with `UnsupportedVersion`. Domain replay and storage admit both versions through `AddressedParseProductV1::validate`. The synthetic import profiles still emit version-1 movement products, and evidence batch admission still requires version 1. Every addressed product of one observation must report the same kind for a facet; a version-1 or unaddressed product reports `Unknown`. Reconciliation refuses an observation whose products disagree.

`FileParser::parse(input:BoundedEvidence,profile:ParserProfileV1)->Result<CaptureBatchV1,EngineError>`; `EvidenceEngine::publish(state:&DomainState,batch:ValidatedEvidenceBatchV1)->Result<DecisionPlan,EngineError>`; `IdentityEngine::associate(state:&DomainState,request:AssociationRequestV1)->Result<DecisionPlan,EngineError>`. `AssociationRequestV1` specifies relationship type, legs/facets, exact allocations, survivor/corrections and reasons. `ReadOnlyAdapter` methods `describe_capabilities()->AdapterCapabilitiesV1`, `async fetch_changes(cursor:Option<String>)->Result<CaptureBatchV1,EngineError>`, `async fetch_snapshot(scope:SourceScopeV1)->Result<CaptureBatchV1,EngineError>`, `async resume_import(receipt:RecordId)->Result<CaptureBatchV1,EngineError>`; source scope is explicit accounts/time/type. No write institution methods. `RefreshCoordinator::next_attempt(state:&ConnectionState,now:OffsetDateTime)->RefreshDecision` is deterministic; 24 hours schedule opt-in,1/5/30 minutes retry then next scheduled, Retry-After, lock/auth failures suppress attempts.

### CUR015 complete relationship route

Financial `IdentityEngine::associate(state, &CompleteAssociationRequestV1)` lives
in **currents-domain**, not acquisition. Acquisition owns only the explicit
bounded `RelationshipSourceParser` profile and immutable descriptive capture.
`RelationshipCaptureV1` binds source/receipt identity, exact retained raw content,
original capture time, typed facets, and explicit revision/predecessor links.
C03's opaque `ImportEvidence::new_relationship` uses the same encrypted staged
content closure as ordinary imports; no wire bytes or provider authority leak
through this trusted in-process seam. C06 checks original bytes before projection
activation. Existing ordinary capture identities and legacy wire records remain
unchanged.

The additive `RelationshipParseRevisionV1` retains an exact original capture,
explicit optional prior parse, content-addressed correction profile and derived
row. `RelationshipSourceParser::correct` supports only the bounded synthetic
`amount-correction/1` profile: exact facet, expected prior amount and explicit new
amount. It never invents provider revisions or alters source identity, units,
roles or dates. `ImportEvidence::new_relationship_parse` binds this separate
input; domain and authenticated replay verify its deterministic derivation.
Publication has no financial effect. `CapturedFacetBindingV1.parse_revision` is
optional and omitted for existing original bindings, preserving their bytes.
Provider and parser precedence remain separate explicit chains. New financial
bindings require a unique current parse head across equivalent recaptures;
historical bindings retain their exact interpretation. See the
[qualification contract](../qualification/associations.md#explicit-synthetic-parser-corrections).

`AssociationRevisionV1` records immutable root/prior revision, kind, exact
captured-facet allocations, leg and recognition IDs, retired originals, residuals,
explicit survivor where applicable, owner interpretation and governing policy.
`CompleteAssociationRequestV1` supplies the explicit new events/legs/recognitions.
Transfer/payroll admission checks one prior prefix, source capacity, role, dated
postings, declared account/unit ownership and current allocation exclusivity.
Fresh finite `Recognitions` and `Corrections` batches retain original per-entry
dates in one bounded C05 manifest/C04 closure; declared retired originals equal
the native reversal set. No within-batch reversal chains, recursive scripts or
arbitrary segment replacement are introduced. Duplicate admission selects one
existing C08 posted single-leg survivor and reverses all other approved originals.
Same-family transfer/payroll remaps replace the entire current revision with fresh
identities; historical legs and evidence are never overwritten.

Explicit Clearing and TaxPrepayment internal roles are declared Asset accounts
with known creation boundaries, not inferred external openings. Payroll withholding
requires the owner to choose expense versus prepayment. `PendingExpiryV1` names
an existing pending adoption and explicit owner interpretation; absence, time and
tombstones cannot invoke it implicitly or reverse money. Reconnect reuses explicit
catalog aliases. Unsupported legacy multi-facet and general segment semantics
continue to reject. See [qualification and limits](../qualification/associations.md).

<a id="c08"></a>

## C08: Catalog, recognition and segments, owners CUR-013, CUR-016

Files: `crates/currents-domain/src/{recognition,segments,correction}.rs`.

`SetAccountCatalogV1 { account_id, component_id:RecordId, account_class, unit_id, normal_sign, aliases, lifecycle, display_name }` supports create/update of domain catalog under preview; immutable native paths/unit codes/classes cannot be silently changed. `CatalogEngine::set(state:&DomainState,request:&SetAccountCatalogV1)->Result<DecisionPlan,EngineError>` produces a metadata-only native declaration plan when needed; separate financial preview follows committed declaration head. `RecognitionRequestV1 { leg_ids, facet_allocations, postings, native_dates, interpretation, authority_basis }`; `CorrectionRequestV1 { active_recognition, replacement, remapping, reason }`; `SegmentRequestV1 { component, prior_segment, opening, included_activity, replacement_closure, approved_accounts, residual_disposition }` holds recognition IDs only and has no route; the replacement preview takes the drafts instead (see the CUR-016 segment replacement checkpoint). `RecognitionEngine::recognize(state:&DomainState,request:RecognitionRequestV1)->Result<DecisionPlan,EngineError>`; `CorrectionEngine::correct(state,&CorrectionRequestV1)->Result<DecisionPlan,EngineError>`; `SegmentEngine::replace(state:&DomainState,request:&SegmentReplacementV1)->Result<SegmentPreviewV1,EngineError>`. Each enforces facet and leg ownership, whole-entry closure, explicit dates, supported anchors, no overlapping segments, no silent residual plug; atomically invalidates dependent claims and guards organization mapping. It refuses a change that occurrence matches, basis results, saved forecast runs or review records depend on, until CUR-026, CUR-022, CUR-027 and CUR-020 define their correction rules.


### CUR013 complete additive command route

Historical `SetAccountCatalog` and `Recognize` tags and record bytes remain unchanged. Their incomplete payloads cannot establish the required ownership, date, parse and opening basis; the complete route uses `SetAccountCatalogComplete(CatalogRequestV1)` and `AdoptActivity(ActivityRequestV1)` instead. Both require schema version 1 and caller-supplied stable identities bound into preview/submit; neither samples identities or a clock.

`CatalogRequestV1` combines the existing catalog payload with a fresh revision ID, exact prior revision, explicit ownership and `AccountOriginV1`. A complete owner-approved creation request explicitly asserts new-account ownership and retains an unknown opening; any applicable accepted perimeter assertions must agree. No dangling predeclared Account reference, default share or perimeter rewrite is introduced. Internal control accounts require an explicit role (unresolved equity, opening equity, income flow or expense flow) and creation boundary. An immutable `AccountCatalogRevisionV1` binds metadata to the original component/account. Revision chains cannot fork, drop aliases or change native class/unit/sign/account identity. Creation freezes metadata declarations only; later metadata changes have no financial effect. Declaration must commit before financial preview at the new head.

`ActivityRequestV1` supplies an adoption ID, explicit event and legs, exact source observation/addressed parse product/facet/amount bindings, owner interpretation and an exact governing DatasetPolicy reference. `Pending` is an explicit adopted assertion, not inferred from source age, absence or arbitrary text. Pending activity has no native entry or hold. A posted adoption may name one exact prior pending adoption; consuming that exposure does not recognize its earlier amount. Immutable `ActivityAdoptionV1` records the approved sources and pending relationship, so reparsing or enrichment cannot silently replace the adopted meaning. This initial route retains C07's one-facet-per-observation boundary: retained parser products and explicit source-revision ancestors must preserve that facet lineage. Multi-facet decomposition is unsupported rather than guessed.

Financial terms separately identify recognition and lineage, postings, bookkeeping date and exact DateConversionPolicy. Governing DatasetPolicy and date conversion policy are distinct typed roles, never a latest-policy default. Reconstructed DomainState retains DatasetPolicy by exact (id, version), atomically with its processed prefix; date policies use C05's verified exact-version index. Later policy records do not reinterpret earlier adopted meaning. Unexplained movement requires an explicitly named unresolved-equity control account; it does not imply income or an absolute opening balance. Supported initial opening terms contain exact component, amount, evidence, cutoff and inclusion map. The opening amount equals its balance evidence in the source's convention, where a liability's balance is the positive amount owed. The opening leg posts that amount converted by the account's normal sign: a debit-normal account posts it unchanged, and a credit-normal account, such as a liability, posts its negation, a credit. This initial-anchor path supports civil-date boundaries: every opening posting effective date must equal the cutoff, and subsequent postings to the anchored account must be strictly after it independently of leg dates. Instant-valued posting dates needing conversion are rejected for these boundary checks rather than guessed; unrelated unanchored C05 date-policy conversion is unchanged. Replacement/correction and general association inference remain later owners' contracts. All new records require typed persisted reference closure and exact-prefix replay validation; a merely deserializable or unsupported version is not healthy history.

### CUR-016 bounded correction checkpoint

`CorrectionEngine::correct` previews an identity-only, one-for-one replacement against a validated prefix; it is not a write route. The private durable test path additionally emits a version-1 `RecognitionCorrectionRevisionV1` naming a previously adopted standalone recognition and its fresh replacement in the same commit. It declares `UnchangedStableLegs`; the old adoption, recognition, historical dates, amounts, postings and ownership stay unchanged. Replay re-admits exactly one revision and one replacement against the prior prefix, and C05 binds the whole reversal and replacement to the frozen native closure. Existing records and command tags are unchanged. A reader that does not recognize the new record cannot silently consume it.

The feature-gated synthetic host path rehydrates the exact prior from the same
Writer's audited retained history, then re-admits and re-freezes the proposed
correction; caller projections are not inputs to that write helper. Encrypted
tests exercise cancellation between reversal and replacement, exact retry, and
fresh rebuild. There is no default service or wire dispatch. This slice
now allows another identity-only correction of the authenticated live tip
of a standalone chain. Replayed activity retains one adoption root and
one native live tip. Reversing archived entries, revision forks, reused
posting identities, ownership changes and dependency remapping remain
unsupported. A real refund is new economic activity at its own date, not
compensation of an archived posting. General corrections and segment
replacement remain outside this checkpoint.

### CUR-016 reinterpretation checkpoint

`CorrectionEngine::reinterpret(state, revision_id, request, governing_policy,
unresolved_account)` corrects how a live standalone activity recognition is
interpreted while its legs stay. The replacement's external postings must be
the legs exactly: one posting per leg with its account, amount, effective date
and posting date. Its other postings may use income, expense or named
unresolved-equity control accounts. Adoption's recognition rules for those
legs, under the exact retained `governing_policy` version and the named
`unresolved_account`, must derive the replacement with the original lineage,
bookkeeping date and date policy. The planner refuses anything else, including
opening equity, clearing, an unbalanced entry and a missing or unretained
policy version.

The plan holds a version-2 `RecognitionCorrectionRevisionV1` whose
`ReinterpretedStableLegs` disposition records the governing policy, the
unresolved account and `changed_accounts`: every account whose net change is
nonzero on some date, in ID order. Its accounting intent is one `Correction`,
so C05 reverses the original once at its own dates and adds the replacement.
The old adoption, legs and recognition stay byte-identical. Replay derives the
plan again from the revision, and the chain audit checks each reinterpreted
link with the rules for retained history. A later identity correction or
reinterpretation starts from the live tip. Claims and organization follow the
dependent invalidation checkpoint. Corrections that change amounts, dates or
leg splits follow the leg correction checkpoint. Transfer, payroll and duplicate
families stay with CUR-015 association remaps, and openings with segment
replacement. There is no command route.

### CUR-016 leg correction checkpoint

`CorrectionEngine::replace_legs(state, revision_id, request, adoption)`
corrects a live standalone activity recognition whose legs change: a
provider-revised amount, a changed effective date or one leg split into
several. `adoption` is a fresh posted `ActivityRequestV1` with new event,
legs, recognition, lineage and postings, adopted by the rules of a new
adoption, and the recognition it derives is the request's one replacement.
It binds the same source facts as the retired adoption and no others. For
each fact the bound total stays the same, unless the new adoption binds a
later source revision of that facet, which it binds for that version's whole
amount. Only this adoption may bind one facet version to several legs, and
those portions must total its amount. The old facet version stays as
evidence.

`CorrectionPreviewV1` is `Ready(Box<DecisionPlan>)` or
`NeedsOrganizationDecision`, which lists each organization record on a
retired leg with that leg, and no plan. The owner answers through
`CorrectionRequestV1.remapping`, which holds `OrganizationRecordV1` successor
versions: exactly one per affected record, with the next version, the same
kind and parent, `supersedes` naming the record, no allocations and only
replacement legs as targets. A remap that drops, duplicates or adds a record
is refused, and so are records with allocations. The old record stays
byte-identical.

A ready plan holds a version-3 `RecognitionCorrectionRevisionV1` whose
`ReplacedLegs { retired_adoption, replacement_adoption }` disposition names
both adoptions, then the new event, legs, adoption and recognition, then the
organization successors. Its accounting intent is one `Correction`, so C05
reverses the original once at its own dates and adds the replacement at its
dates. The plan invalidates the original recognition, its legs and the claims
the change reaches; the old adoption, event and legs also retire for the
dependent check. The old adoption, event, legs and recognition stay
byte-identical, so replay through an earlier commit still shows them active.

Replay derives the whole commit again from the prior prefix and the commit's
own records. An adoption is retired once a `ReplacedLegs` revision names it
and the native index shows its recognition reversed; it then owns no facts,
and its replacement owns them. The chain audit follows each revision as one
edge: a leg correction checks the two adoptions and their facts, and the
replacement starts a chain that its adoption owns, so a later identity
correction, reinterpretation or leg correction starts from that live tip. The
planner refuses a facet another adoption owns, a correction of a retired
original and a second reversal. Merges of separate adoptions use the CUR-015
duplicate route, and associated families use CUR-015 remaps. Trade quantity
legs and allocation remaps stay unsupported: every correction path refuses a
recognition with a security-unit posting (see CUR-021 trades in C11). There is
no command route.

A purchase that consumed a pending adoption keeps it consumed exactly once.
The new adoption names the same pending adoption, and the same commit
releases the retired adoption's consumption. The planner and the chain
audit refuse a new adoption whose pending adoption differs from the retired
one's. Storage defines each pending consumption once: in the commit of
the `ReplacedLegs` revision, the new adoption refers to the existing
consumption instead of defining it again. Replay through an earlier commit
still shows the retired adoption consuming it.

### CUR-016 segment replacement checkpoint

`SegmentEngine::replace` previews the replacement of one account component's
active segment. `SegmentReplacementV1 { prior_segment, opening, backfill,
approved_accounts }` names the active segment, a posted opening draft
(`ActivityRequestV1` with balance evidence, whose `segment_id` names the
successor), ordered posted backfill drafts and the approved accounts. The new
cutoff may equal the old one (an anchor correction) or come before it.
Each backfill posts to the anchored account, after the new cutoff and through
the old one, with retained movement evidence in the account's unit. The
approved accounts must equal the accounts the plan touches.

`SegmentPreviewV1` is `Ready(Box<DecisionPlan>)`, `UnresolvedResidual(Amount)`,
`NeedsOrganizationDecision` (see the dependent invalidation checkpoint),
`TooLarge { operations, limit }` or `TooManyRecords { records, limit }` (see
the large closure checkpoint).
The residual is the new opening plus backfill on the account, minus the old
opening; the preview reports it and returns no plan. A ready plan holds each
draft's event, legs, adoption and recognition, then a successor
`AccountingSegmentV1` naming the prior segment, the new opening, the backfill
in order and the retired opening as its only closure. Its accounting intent is
one `Correction` that reverses the old opening once with its original dates,
then adds the new opening and each backfill with their own dates.

Replay re-admits a successor segment only by deriving the same plan again from
the prior prefix; the commit holds exactly that plan, then its frozen
manifest. C05 correspondence requires the old opening's reversal first, then
the new opening, then the backfills in order. Preview and replay refuse a
second replacement of a replaced segment, a successor that names itself and
any history with two active segments for one component. Ordinary adoption
still refuses activity at or before every opening cutoff.

The preview refuses a backfill leg on another external account unless it
joins a transfer family (see the cross-account closure checkpoint).
Dependent records follow the CUR-016 dependent invalidation checkpoint. There
is no command route, `OperationV1` variant or owner residual decision; the
legacy `ReplaceAccountingSegment` stays unsupported.

### CUR-016 cross-account closure checkpoint

The engine derives a replacement's closure; the request cannot choose it. The
closure is the old opening and the current recognitions of each CUR-015
transfer family that a backfilled leg joins. Recognitions dated after the old
cutoff keep their IDs. Ordinary adoption refuses activity at or before every
opening cutoff, so no other recognition on the account can sit at or before
it. Ordinary adoption also keeps refusing activity at or before a replaced
opening's cutoff.

`SegmentReplacementV1` adds `transfers` and `reemissions`. A
`TransferJoinV1 { revision, prior_revision, leg, source }` names a family's
current revision, the fresh ID of its next revision, the backfilled leg that
joins it and that leg's captured relationship fact. The fact decides the
family: it must be a current, posted movement with the leg's date and amount,
from the same capture source and group as every fact the family holds, and no
current family revision may already allocate it. That leg's draft posts the
anchored account against the clearing account the family already uses; only
a joined draft may post to clearing. A
`ReemissionV1 { id, original, replacement }` issues one recognition in the
closure again with fresh recognition and posting IDs and unchanged economics.
The preview refuses a request whose re-emissions name a recognition twice,
leave one out, name one outside the closure or change a posting, and a
re-emission of a recognition that posts to the anchored account. The approved
accounts must equal every account the plan touches, counterparts included.

A ready plan holds the drafts' records, each re-emitted `Recognition`, one new
`AssociationRevisionV1` per joined family, the successor segment and one
`SegmentReemissionV1 { id, schema_version, segment, original, replacement }`
per re-emission. The new revision follows the family's current revision. It
keeps every leg, allocation, amount and date, adds the joined leg and its
captured allocation last, names the re-emissions and the joined leg's
recognition, retires the current recognitions and records the new residual. This is C07's whole-revision remap
with fresh identities. The successor's `replacement_closure` lists the old
opening, then each original in request order. Its accounting intent is one
`Corrections` list: the old opening's correction, then one correction per
re-emission. C05 requires the old opening's reversal, the new opening and the
backfills, then each original's reversal followed by its re-emission.

The old opening's inclusion map may hold activity. Each fact in it must
appear exactly once in the new history: in a backfill's sources or in the new
opening's inclusion map. A replaced opening no longer owns its inclusion map.
Replay derives the whole plan again from the commit, including the joins and
re-emissions. The association audit accepts each re-emission only when one
family revision retires the original and names the re-emission, the original
is reversed, and the two recognitions differ only in their IDs. CUR-015
association records are never rewritten. Payroll and duplicate families,
changed transfer economics and other association revisions remain refused.

### CUR-016 dependent invalidation checkpoint

A segment replacement and a durable correction each assess the
records that depend on them. The change is the plan's corrections: each
reverses its active recognition and adds its replacements. Its net posting
change is kept by account, unit, effective date and posting date. A
replacement retires the prior segment, the old opening's adoption, event and
legs, each joined family's prior revision, and every reversed recognition
with its postings. A correction retires only the corrected recognition and
its postings; its legs stay. A leg correction also retires the old adoption,
its event and its legs.

A reconciliation claim is invalidated when the net change on one of its scope
accounts is nonzero on a date inside its cutoffs, by the claim's own date
basis, or when its evidence, decision, accounting, exclusion or
matched-allocation identities name a retired identity. A posting date that is
not a civil date counts as inside every range. A claim that is already
invalidated is skipped. The plan lists each invalidated claim in record order
after its other invalidations, so the command result's effects list it too.
The claim record is never rewritten. `DomainState::claim_invalidated` reports
true from the commit whose result invalidated the claim, and
`DomainState::claim_status` keeps reporting the record's own lifecycle status.
Replay derives the plan again and rejects a segment or correction commit whose
recorded invalidations differ. Replay rejects any other commit whose result
lists a reconciliation claim in its invalidations. Adoption, association and
import commits invalidate no claim when they commit; a reader sees their
changes in the claim's dependency digest (C09).

Organization records on legs that stay keep their targets. A replacement that
retires a leg that an organization record targets returns
`SegmentPreviewV1::NeedsOrganizationDecision`, which lists each record and
leg, and no plan. A leg correction returns
`CorrectionPreviewV1::NeedsOrganizationDecision` the same way until the
request remaps each record. An organization record that names another retired identity
refuses the change. So does a review record, forecast run, occurrence match,
allocation or association that names an account the change touches or a
retired identity. The durable correction keeps refusing any forecast run,
occurrence match, allocation or association in the dataset.

### CUR-016 segment commit checkpoint

`verify_segment_preflight(prior, trusted_tip, request, plan, frozen)` accepts
a segment replacement only when the prior state is exactly the trusted tip,
the frozen batch names that tip, `SegmentEngine::replace` derives the same
ready plan again from `request`, and `Writer::verify_segment_freeze` passes.
That writer check requires one successor segment whose only closure is the
retired opening and whose opening and backfill are the replacement entries in
order, then compares the batch with a fresh C05 freeze of the plan's intent.
It is read-only; C04 still checks the exact tip inside the write transaction.

The feature-gated `synthetic_commit_verified_segment` rebuilds the prior from
the same Writer's audited history, as the correction host path does, and
commits only after that check. An exact retry uses the prior of the earlier
commit, so it resolves to that commit; a reused command ID with other content
is refused. Encrypted tests cover stale and altered plans, cancellation before
COMMIT submission, retry, reopen, a fresh rebuild and a 2024 backfill on an
account declared in 2026. History replayed through the commit before the
replacement keeps the old records. There is still no command route or service
dispatch.

`cargo xtask ticket CUR-016` lists every CUR-016 test by package and target,
runs each target and fails when a listed test is missing, renamed, ignored or
failing, or when a target has a test the list does not name. Later CUR-016
leaves add their tests to that list.

### CUR-016 large closure checkpoint

The preview counts the native operations a replacement needs before it checks
the drafts. C05 reverses each native entry of the old opening and of each
re-emitted original, then adds one entry for the new opening, each backfill
and each re-emission. A count above `MAX_CLOSURE_OPERATIONS` (20,000) returns
`SegmentPreviewV1::TooLarge { operations, limit }` and no plan. The backfill
has no length limit of its own.

A plan also has to fit one C04 commit: its records plus the frozen manifest
may number at most `MAX_COMMIT_ITEMS` (20,000). Each single-leg backfill adds
four records (event, leg, adoption and recognition), so a backfill-only
replacement holds at most 4,998 backfills, far below the operation limit. A larger plan returns `SegmentPreviewV1::TooManyRecords { records,
limit }` and no plan. Both constants live in `currents-types`;
`currents-accounting` re-exports the first and the C04 writer checks the
second.

A plan within both limits freezes and commits through the segment commit path
unchanged. C04 applies its native batches of at most 600 entries on one
transaction, so a reader sees the prior domain and native heads until COMMIT,
and a failure or cancellation in any batch rolls back the whole closure. The
C05 limit of 64 MiB of canonical instructions still applies when the plan is
frozen.

### CUR-021 investment control roles

`ControlRoleV1` adds three internal roles. Quantity control (`QuantityControl`,
equity class) balances quantity movements and takes a security unit.
Investment carrying amount (`InvestmentCarrying`, asset class) and disposal
clearing (`DisposalClearing`, equity class) take a currency unit.
`CatalogEngine::set_complete` refuses each in any other unit kind, and refuses
every earlier role (unresolved equity, opening equity, income flow, expense
flow, clearing and tax prepayment) in a security unit. An external account may
be in a security unit.

Activity adoption refuses a leg or posting in a security unit, and a posting to
any of the three new roles. Association admission refuses a leg or posting in
a security unit. Only the C11 investment engine moves quantities. Existing
roles, records and command tags keep their bytes.

<a id="c09"></a>

## C09: Queries/reconciliation, owners CUR-017, CUR-018, CUR-020

Files: `crates/currents-domain/src/{reconciliation,position,aggregation,history,review}.rs`.

`ReconciliationRequestV1 { kind:Balance|Activity|Relationship|Holdings, scope, cutoffs:DateScope, evidence, exclusions, basis }`; `ClaimV1` is the `ReconciliationClaimV1` record; `ReconciliationEngine::evaluate(state,&request)->Result<ReconciliationClaimV1,EngineError>`; `ReconciliationEngine::confirm(state,claim:RecordId,digest:Digest)->Result<DecisionPlan,EngineError>`; `record`, `dispute` and `lifecycle` follow the CUR-017 claim lifecycle checkpoint. `SetDatasetPolicyV1 { policy_id:RecordId, prior_version, effective_scope, ownership_perimeter, source_priority, freshness_thresholds, timezone, reporting_currency }` contains typed optional changes with explicit unknowns and adopted basis; `PolicyEngine::set(state:&DomainState,request:&SetDatasetPolicyV1)->Result<DecisionPlan,EngineError>` previews material meaning changes and required invalidation without altering prior policy versions. `PositionEngine::query(state:&DomainState,basis:&ReadBasis,scope:&AccountScope)->Result<PositionResponseV1,EngineError>`; response contains assertions, movements/anchored balance, selected basis, pending, freshness/coverage, discrepancies, missing inputs and `AggregateV1 = Exact(Amount)|KnownSubtotal{known,omitted,obstructions}|Range{low,high}|Indeterminate{components,obstructions}` (see the CUR-018 household aggregate checkpoint). Ownership shares exact rational; range only when alternatives evidenced. `HistoryEngine::query(state,basis,filter:HistoryFilterV1)->Result<HistoryResponseV1,EngineError>`; `ReviewEngine::set(state,request:ReviewDispositionV1)->Result<DecisionPlan,EngineError>`. Filter names account/party/category/tag/date basis/page cursor; response includes unknown/incomplete buckets. Every paged result pins the same basis. `ExplanationV1` is a scoped graph of evidence/decisions/selected policies, not unrestricted raw records.

`ReconciliationClaimV1 { id, kind:ReconciliationKindV1, comparison_version, basis:ReadBasis, scope, unit, cutoffs, evidence_dependencies, decision_dependencies, accounting_dependencies, dependency_digest, matched_allocations, differences, exclusions, status, reasons:Vec<ClaimReasonV1>, reason }`.
`status` is `Proposed`, `PartiallySupported`, `ConfirmedForScope`, `Disputed`
or `Superseded`. `reasons` lists why a claim is partial: `UnknownOpening`,
`IncompleteInterval`, `IncompatibleBalanceKinds`, `MissingDateMapping`,
`UnexplainedResidual`, or another reason a checkpoint below defines. Storage checks references by role:
`evidence_dependencies` name evidence records, `decision_dependencies` name
decisions, and `accounting_dependencies` name recognitions, legs,
association revisions and accounting segments. `unit` names a declared unit,
and `basis` names the dataset, its unit catalog version and a commit at or
before the claim's own. No route writes claims yet.

The bounded in-process Activity comparison compares caller-selected source
and allocation lines by exact observation/facet identity, scope, selected
civil date, unit and normalized signed amount. Offsetting omissions cannot
net out; same-amount rows with distinct identities cannot substitute.
Split/repeated identities touching scope remain unsupported (callers
must supply all sibling rows, including those outside scope). An empty
difference list proves neither account-period completeness nor retained
allocation eligibility; this is not claim confirmation or service dispatch.

### CUR-017 activity coverage checkpoint

`ReconciliationEngine::evaluate(state,&request)->Result<ReconciliationClaimV1,EngineError>`
evaluates `Activity` requests, and `Balance`, `Holdings` and `Relationship`
requests as the checkpoints below describe. It refuses a request whose
`basis.through_commit` is not the state's commit, or an Activity or Balance
request whose accounting index lags the state, with `StaleBasis`. An Activity request names exactly one account and a
`cutoffs` interval with both a start and an end. Each `evidence` ID names an
import receipt or an observation version. Evaluation refuses a named
observation version that has a later explicit revision with `StaleBasis`.
Each exclusion names a named observation or a live recognized leg.

The source lines are the movement facets of the named observations. A line's
one reported date is its facet's temporal value, and the comparison uses it
under either date basis. The recognized legs are the legs of every Activity
recognition that the accounting index has not reversed. Exclusions remove the
named lines, their allocations and the named legs from the comparison. The
engine compares the lines with every live leg's allocations through
`compare_activity_allocations`, so offsetting omissions never net out.

The claim is `Proposed` only when it has no reasons. Otherwise it is
`PartiallySupported`. `ClaimReasonV1` reasons are:

- `IncompleteInterval`: no named receipt reports `Complete` coverage of the
  whole interval by the request's date basis, or a later receipt contradicts
  the one the claim uses. When several named receipts report it, the claim
  uses the one committed last. A receipt contradicts it when it comes from the
  same source (an equal `SourceIdentityV1`), was committed after it, and
  reports coverage by the request's date basis that overlaps the interval and
  is not `Complete`. Commit order decides, not a receipt's received time.
- `UnknownOpening` or `UnknownClosing`: the statement reports no single
  balance at the end of the day before the interval, or at the end of its last
  day. The opening is also unknown when the interval starts on or before the
  cutoff of the account's active opening anchor (C08): that history is
  missing from the journal, and the anchor's own evidence cannot corroborate
  it. Such a claim names the anchor's segment in `accounting_dependencies`.
  An interval that starts after the cutoff keeps its earlier result, even
  when its opening is the anchor's evidence.
- `UnexplainedResidual`: the opening plus the source lines in the interval,
  excluded lines included, is not the closing. The balances are in the
  source's convention and the lines in journal sign, so the comparison
  converts the balances to journal sign by the account's normal sign.
  `differences` holds the closing minus that sum, converted back to the
  source's convention.
- `MissingDateMapping`: a named line or balance, or a live leg on the account,
  has no civil date by the request's basis. It leaves the comparison.
- `MissingAllocation`, `UnexpectedAllocation`, `RepeatedIdentity` and
  `DifferentTerms` each name one `EvidenceLineV1 { observation, facet }`.
  They serialize as `{"MissingAllocation":{...}}`.
- `UnsupportedLeg` names a live leg on the account, with a civil date inside
  the interval, that allocates no source line, such as a leg an association
  admitted. Excluding the leg removes the reason. It serializes as
  `{"UnsupportedLeg":"<leg id>"}`; the reasons without a payload stay plain
  strings.

The claim records the request's basis, account, `cutoffs` and sorted
evidence and exclusions, the account's unit and comparison version 1.
`matched_allocations` lists every compared line in the interval that matched,
with its source amount. `accounting_dependencies` names each live leg the
claim compared: a leg on the account inside the interval or without a civil
date, or one that allocates a named observation. It names no recognition, so
an identity-only correction, which replaces a recognition and keeps its legs,
does not invalidate the claim under the CUR-016 rules.
`decision_dependencies` is empty. The claim ID is a version 8 UUID derived
from the request's kind, accounts, `cutoffs`, sorted evidence and exclusions,
and basis, so the same request at the same basis gives the same claim and
replay can derive the ID again from the claim.

`dependency_digest` is a SHA-256 over the comparison version, the account
and interval, each named observation with its latest explicit revision, each
compared leg by its stable leg ID with its account, unit, effective date,
posting date, normalized amount and evidence allocations, the account's
current catalog terms except its display name, and the receipt coverage the
claim used. When a later receipt contradicts that coverage, the digest also
covers each contradicting receipt's ID and its overlapping coverage entries;
a later receipt that reports the interval complete changes no input.
Recognition and posting IDs are not inputs, so an identity-only correction
keeps the digest. Neither organization records nor the display name enter
the digest. When an anchor makes the opening unknown, the digest also covers
the anchor's segment and opening terms; otherwise the digest is unchanged.

### CUR-017 balance agreement checkpoint

A `Balance` request names exactly one account and a `cutoffs` scope with no
start and an end, the boundary. Its `evidence` names import receipts and
observation versions that report exactly one balance facet and no movement
facet, and it has no exclusions. The journal balance is the opening amount of
the account's active anchor, converted to journal sign by the account's normal
sign, plus every live recognized leg on the account whose date by the
request's basis is on or before the boundary. It counts as
`Posted`. The claim is `Proposed` only when the source balance equals the
journal balance exactly; there is no quantization. Its reasons are:

- `UnknownOpening`: the account has no active anchor, or the boundary is
  before the anchor's cutoff.
- `CircularAnchor`: the source balance is the balance evidence the anchor's
  opening adopted, or a later revision of that facet. It cannot corroborate
  the anchor, even at the anchor's boundary.
- `IncompatibleBoundary`: the source balance's as-of date is not the
  boundary. `MissingDateMapping`: the balance or a live leg on the account has
  no civil date.
- `IncompatibleBalanceKinds`: the source balance is `Available` or
  `Statement`. `UnknownBalanceKind`: it is `Unknown`, as every balance of a
  version-1 product is.
- `IncompatibleUnits`: the source balance is not in the account's unit.
- `UnexplainedResidual`: the balances compare and differ. `differences` holds
  the source balance minus the journal balance, in the source's convention:
  the source balance converted to journal sign by the account's normal sign,
  minus the journal balance, converted back. For a liability, a positive
  difference means the source reports more owed than the journal.

Only a balance with no reason, or with `CircularAnchor` alone, is compared.
A balance that compares equal is listed in `matched_allocations`. A
difference stays visible; no entry or adjustment is proposed, and `record`
returns `AccountingIntent::NoEffect`. The claim records comparison version 1,
the account's unit, no exclusions, and in `accounting_dependencies` the
anchor's segment and every live leg on the account through the boundary or
without a civil date. Its digest covers the same inputs as an Activity
digest, with the anchor and no coverage, under its own domain tag.
`applicability` rechecks it the same way.

Applicability adds `InvalidationReasonV1::Anchor`: the anchor that decides the
claim changed, such as an opening adopted after an Activity claim whose
interval now starts before it.

### CUR-017 holdings agreement checkpoint

```text
compare_holdings(source:&[HoldingPositionV1],journal:&[HoldingPositionV1])->Result<Vec<HoldingDifferenceV1>,EngineError>
HoldingPositionV1 { quantity:Amount, basis:RecordId, boundary:Date }
HoldingDifferenceV1 = Quantity(Amount) | IncompatibleBasis(UnitId) | IncompatibleBoundary(UnitId)
```

`currents_domain::holdings::compare_holdings` is a pure comparison. A
position states a quantity in its security's unit, under one
corporate-action basis, at the end of one day; it has no price, so equal
market value never makes positions agree. Each side names a security at most
once, or the comparison refuses with `InvalidReference`. Per security, in
unit order, it reports `IncompatibleBasis` and `IncompatibleBoundary` when
the sides differ in basis or boundary, and then compares no quantity.
Otherwise it reports `Quantity` with the source quantity minus the journal
quantity when they differ. A security on one side only compares with a zero
quantity. An empty list is agreement.

A `Holdings` request names one account and a `cutoffs` scope with no start
and an end, the boundary. Its `evidence` names import receipts and current
observation versions whose facets are all `Holding` facets, at least one in
all, and it has no exclusions. The journal records no security quantities
until CUR-021 (#30), so the claim is always `PartiallySupported` with the one
reason `JournalQuantityUnavailable`, no differences, no matched allocations
and no accounting dependencies. It records comparison version 1 and the
account's unit. Its digest covers the named observations with their latest
revisions, the account and boundary, and the account's catalog terms, under
its own domain tag; no leg or anchor enters it. `applicability` rechecks it
the same way.

### CUR-017 relationship settlement checkpoint

A `Relationship` request names no account, a `cutoffs` scope with neither a
start nor an end, and no exclusions. Its one `evidence` ID names a revision
of a `Transfer` or `Payroll` association; another kind is refused with
`InvalidReference`. The claim evaluates the association family's current
revision and says nothing about the rest of any account. It is `Proposed`
only when it has no reasons. Otherwise it is `PartiallySupported`:

- `UnexplainedResidual`: the revision records a nonzero residual.
  `differences` lists each nonzero residual.
- `UnsupportedLeg`: the revision's evidenced allocations do not support one
  of its legs in full, in the leg's unit. Additive allocations must sum to
  the leg's amount; a corroborating allocation supports the leg when it
  equals that amount. `differences` lists the leg's amount minus its
  support, after the residuals.

The claim names the current revision as its only `accounting_dependencies`
entry and names no evidence, accounts or matched allocations. Its unit is
the unit of the revision's first leg, and it records comparison version 1.
Replay evaluates the request that names the revision the claim compared.
Its dependency digest covers the comparison version, the association's root
and the revision, under its own domain tag. A later revision of the
association makes `applicability` report `Relationship`. `lifecycle`
supersedes a Relationship claim only with a newer claim on the same
association.

### CUR-017 claim lifecycle checkpoint

```text
ReconciliationEngine::record(state,&request)->Result<DecisionPlan,EngineError>
ReconciliationEngine::confirm(state,claim:RecordId,digest:Digest)->Result<DecisionPlan,EngineError>
ReconciliationEngine::dispute(state,claim:RecordId,digest:Digest,reason:&str)->Result<DecisionPlan,EngineError>
ReconciliationEngine::lifecycle(state,claim:RecordId)->Result<ReconciliationStatus,EngineError>
```

`record` evaluates the request and returns a plan with the one claim, no
invalidations and `AccountingIntent::NoEffect`. Its commit holds one
`Reconciliation` decision.

An owner confirms or disputes a claim with a
`ReconciliationDispositionV1 { id, claim, dependency_digest, disposition, reason }`
record, where `disposition` is `ConfirmedForScope` or `Disputed`. The plan
holds only that record, with no invalidations and no accounting effect. Its
ID derives from the claim and the disposition, so a claim takes at most one.
Storage checks that `claim` names a `ReconciliationClaim`. The claim record
itself is never rewritten.

- `confirm` refuses with `InvalidReference` unless the claim's lifecycle
  status is `Proposed`. It refuses with `StaleBasis` unless `digest` equals
  the claim's recorded digest and the claim's applicability at `state` is
  `Current`.
- `dispute` accepts a `Proposed` or `PartiallySupported` claim with a
  nonblank reason, and refuses with `StaleBasis` unless `digest` equals the
  claim's recorded digest. It does not recompute the digest.
- `lifecycle` reports `Superseded` once the state holds a newer claim with the
  same kind, accounts, unit and `cutoffs`. A claim's commit follows its basis
  commit, so the newer claim has the later `basis.through_commit`. Otherwise it
  reports `ConfirmedForScope` or `Disputed` once the state holds the claim's
  disposition, and the claim's own recorded status before either. Each status
  holds from the commit that records the deciding record, so replay through an
  earlier commit reports the earlier status.

`DomainState::claim_status` still reports the record's own status, and
`DomainState::claim_invalidated` keeps its CUR-016 meaning. Routes for these
operations belong to later work.

### CUR-017 claim applicability checkpoint

```text
ReconciliationEngine::applicability(state,claim:RecordId)->Result<ClaimApplicabilityV1,EngineError>
ClaimApplicabilityV1 = Current | Invalidated(Vec<InvalidationReasonV1>)
InvalidationReasonV1 = ScopeActivity | EvidenceRevision | AccountMapping | Coverage | Anchor | Relationship | RetiredDependency
```

A claim stops applying from the first commit that changes an input in its
scope. `applicability` reports `Invalidated` when the dependency digest
computed at `state` differs from the claim's recorded digest, or when
`DomainState::claim_invalidated` reports true. Each reason names an input:

- `ScopeActivity`: the live legs in the claim's scope differ from the legs it
  names, such as a newly recognized backdated line on the account inside the
  interval.
- `EvidenceRevision`: a named observation has a later explicit revision.
  Evaluation refuses a revised observation, so the revision came after the
  claim.
- `Coverage`: whether the interval is complete differs from the claim's own
  `IncompleteInterval` reason, or a receipt that contradicts the coverage the
  claim used was committed after the claim's basis commit. A later import from
  the same source that reports part of the interval `Partial` is one. A
  receipt from another connection, by the other date basis, or outside the
  interval leaves the claim `Current`. Balance claims use no coverage.
- `Anchor`: the active anchor that decides the claim differs from the segment
  the claim names (balance agreement checkpoint).
- `AccountMapping`: the account's catalog terms, other than its display name,
  changed. A named receipt never changes, so when the interval's completeness
  and its contradicting receipts are unchanged, a digest difference left after comparing the legs, anchor and
  named versions is in the catalog.
- `Relationship`: the association a Relationship claim compared has a later
  revision. Activity, Balance and Holdings claims compare no relationship.
- `RetiredDependency`: a segment replacement or correction listed the claim
  in its invalidations under the CUR-016 rules.

Activity outside the claim's accounts or dates, an organization record and a
display name change leave the claim `Current`. Neither the claim record nor
its disposition changes, and the history through an earlier commit reports
the claim's applicability at that commit. `applicability` refuses a claim
with another comparison version with `UnsupportedVersion`, and an Activity
or Balance claim whose accounting index lags the state with `StaleBasis`.

### CUR-018 dataset policy revisions

```text
DatasetPolicyRevisionV1 { id, policy_id, version, prior_version:Option<u32>, effective_scope:DateScope, ownership_perimeter, source_priority, freshness_thresholds, timezone, reporting_currency }
PolicyEngine::set(state:&DomainState,request:&SetDatasetPolicyV1)->Result<DecisionPlan,EngineError>
PolicyEngine::governing(state:&DomainState,day:Date)->Result<GoverningPolicyV1,EngineError>
GoverningPolicyV1 { ownership_perimeter, source_priority, freshness_thresholds, timezone, reporting_currency }
PolicySettingV1<T> = NotSet | Set { version, value:SourceValue<T> }
```

An owner's `SetDatasetPolicyV1` becomes a new immutable
`DatasetPolicyRevisionV1`. Each revision field is the request's
`Option<PolicyChangeV1<T>>` unchanged: null when the revision does not set
the field, and otherwise the value exactly as stated, `Unknown` included.
`version` is the family's latest revision version plus one, starting at 1,
and `prior_version` is that latest version. The ID is a version 8 UUID
derived from `policy_id` and `version`, so one family version names one
record. The plan holds the one revision, no invalidations and
`AccountingIntent::NoEffect`. Its commit holds one `Policy` decision.

`set` refuses, with no plan:

- a `policy_id` that names no `DatasetPolicy` record, or a second family
  when another family already has revisions; a dataset has one revision
  family;
- a `prior_version` other than the family's latest revision version (`None`
  for the first revision);
- an effective scope whose basis is not `Effective`, or whose start is after
  its end;
- a request that changes no field;
- a change whose `basis` names another dataset or a commit after the state's;
- a known ownership perimeter that names an account no component declares,
  names one account twice, or holds a zero denominator or a numerator above
  its denominator;
- a known source priority or freshness threshold list that names a connection
  no import receipt or account alias uses, or names one connection twice, or
  a threshold of zero seconds; and
- a known reporting currency that is not a declared `Currency` unit.

`governing` gives, for each field, the value of the highest-version revision
whose effective scope contains `day` (both ends inclusive, an open end
unbounded) and which sets that field, with that revision's version. When no
such revision exists the field is `NotSet`. It never falls back to the
latest revision outside its dates, or to a bootstrap `DatasetPolicyV1` field
such as `freshness_seconds`. Replay through an earlier commit reads only the
revisions committed by then.

Replay re-admits a revision only when `PolicyEngine::set` at the prior state
yields the same record, so a stale prior version, a second revision on the
same prior version and a share above one are refused. Storage checks that
`policy_id` names a `DatasetPolicy` record. `DatasetPolicyV1` records,
`PolicyHistory` and the exact `(id, version)` policy a recognition or
adoption names are unchanged. The service route and its preview belong to
later work.

### CUR-018 position selection checkpoint

```text
PositionEngine::query(state:&DomainState,basis:&ReadBasis,scope:&AccountScope)->Result<PositionResponseV1,EngineError>
```

The query reports, for each component in scope, the source balances that
assert its position, the one it selects, and the journal's anchored balance
or known movements. It reads the state and writes nothing.

It refuses with `StaleBasis` a basis whose `through_commit` is not the
state's commit, and a state whose accounting index lags it. It refuses with
`InvalidReference` a basis whose effective scope has no end or another date
basis, a scope that names no component or account or more than 256 entries,
and an unknown component or account. A named account stands for its
component, and each component appears once. The requested date is
`basis.effective_scope.end`.

A balance facet of an observation version asserts a component's position
when the version is current (no later explicit revision, and not a
tombstone), its source identity equals the source of one of the component's
aliases, and its original IDs contain that alias's original ID. A component's
aliases are those its root and catalog revisions declare. `assertions` lists
each such balance whose boundary does not start after the requested date
ends, with the observation version's ID; a version with several balance
facets lists each. Only a `Posted` balance in the component's unit with a
known as-of time, bound to that component alone, is a candidate. Every other
listed balance stays evidence and adds its code to `missing_inputs`.

A date-only as-of is an interval and an instant is a point. All date-only
values, the requested date included, read in one unknown timezone between
UTC−12 and UTC+14, so two dates compare as civil dates. Against an instant, a
date spans from its start at UTC+14 to its end at UTC−12. One boundary is
later than another only when it starts after the other ends.

Selection keeps the candidates no other candidate is later than. When they
share one identical boundary, the governing `source_priority` for the
requested date (`PolicyEngine::governing`) keeps only the candidates from the
highest-ranked connection among them. A priority that is `NotSet`, `Unknown`
or names none of them decides nothing. Overlapping boundaries that are not
identical ignore priority. The remaining candidates are all selected when
their normalized amounts are equal; otherwise they are a conflict and none is
selected. Receipt and commit order never decide. `selected_basis` lists the
selected observation version IDs in ID order.

When the component's account has an active anchor (C08) whose cutoff is on or
before the requested date, `anchored_balance` reports, in journal sign, the
anchor's opening converted by the account's normal sign plus every live
Activity leg on the account dated after the cutoff and on or
before the requested date by its effective date. It names the opening
recognition, the requested date as `through`, and the segment and those legs
as dependencies. Its amount is `Unknown` when a live leg on the account has
no civil effective date. Without such an anchor, `movements` lists each live
leg on the account dated on or before the requested date, with the leg ID and
its allocated observations as evidence. A source balance is never rolled
forward by later movements.

`missing_inputs` lists each code that applies once, sorted:

- `ambiguous_alias`: a balance's aliases bind two or more components.
- `incompatible_balance_kind`: a balance is `Available` or `Statement`.
- `unknown_balance_kind`: a balance is `Unknown`, as every balance of a
  version-1 product is.
- `incompatible_unit`: a balance is not in the component's unit.
- `unknown_as_of`: a balance's as-of time is unknown.
- `unknown_amount`: a balance's amount is not known. It is not listed in
  `assertions`.
- `conflict`: the remaining candidates' amounts differ.
- `unknown_opening`: the component has no anchor at or before the requested
  date.
- `missing_date_mapping`: a live leg on the component's account has no civil
  effective date. It leaves the movements and makes the anchored amount
  `Unknown`.

`aggregate` follows the household aggregate checkpoint below. `pending`,
`freshness`, `coverage` and `discrepancies` are empty, and each assertion's
`coverage` is an unbounded `Unknown` entry; later CUR-018 work fills them.
There is no route or query dispatch.

### CUR-018 household aggregate checkpoint

```text
AggregateV1 = Exact(Amount)
  | KnownSubtotal { known:Amount, omitted:Vec<AggregateOmissionV1>, obstructions:Vec<String> }
  | Range { low, high }
  | Indeterminate { components:Vec<AggregateComponentV1>, obstructions:Vec<String> }
AggregateComponentV1 { component, amount:SourceValue<Amount>, evidence:Vec<RecordId> }
AggregateOmissionV1 { component, amount:SourceValue<Amount>, evidence:Vec<RecordId>, reason:String }
```

The aggregate counts each in-perimeter component in scope once, at its exact
governing ownership share, in the reporting currency. A missing opening,
rate, ownership or value never counts as zero.

A component's amount is its selected assertion's amount, with the selected
observation version IDs as evidence. Without a selection it is the anchored
balance, with the segment and legs as evidence. Without either it is
`Unknown`. Every amount is in the source's convention, so a liability's
amount is the positive amount owed (ruling G5 on #27). A selected source
balance already reports it that way. The anchored balance is in journal sign
and is converted by the account's normal sign, the same conversion C08
openings and CUR-017 balance agreement use, so a credit-normal liability's
anchored balance is negated.

The governing ownership perimeter for the requested date
(`PolicyEngine::governing`) decides each share. When it is `NotSet`, each
component's catalog ownership applies. When it is `Unknown`, every component
is omitted with `unknown_ownership`. When it is known, a component whose
account it does not list is outside the household: it is not listed anywhere
in the aggregate and contributes nothing. The query never reads the dataset
definition's bootstrap perimeter or `OwnershipPerimeter` records.

The reporting currency is the governing `reporting_currency`, or the dataset
definition's when that is `NotSet`. An `Unknown` value omits every component
with `unknown_reporting_currency`.

Each in-perimeter component is omitted with the first reason that applies:

- `unknown_ownership`: the governing perimeter is `Unknown`.
- `unknown_reporting_currency`: the governing reporting currency is
  `Unknown`.
- `unsupported_class`: the component is not an asset or a liability.
- `conflict`: its candidates conflict, even when it has an anchored balance.
- `unknown_opening`: it has no selection and no anchor at or before the
  requested date.
- `missing_date_mapping`: it has no selection, and its anchored amount is
  `Unknown`.
- `missing_rate`: its amount is in a currency other than the reporting
  currency. There is no FX conversion.
- `missing_value`: its amount is in a security or analytical unit. There is
  no valuation.
- `inexact_share`: its amount times its share has no exact decimal at a scale
  of 18 or less.

Every other component adds its amount times its share, and a liability
subtracts it. The total is normalized and in the reporting currency.

- `Exact(total)`: no in-perimeter component is omitted. A scope the known
  perimeter leaves wholly outside the household gives `Exact` zero.
- `KnownSubtotal`: some components are omitted and at least one counts.
  `known` is the total of those that count, and `omitted` lists each omitted
  component in component order with its amount in its own unit, its evidence
  and its reason. A known subtotal is not complete net worth.
- `Indeterminate`: every in-perimeter component is omitted, or the reporting
  currency is `Unknown`. `components` lists each in-perimeter component in
  component order with its amount in its own unit and its evidence.

`obstructions` lists each omission reason once, sorted. The query never
returns `Range`; transfer adjustments and brokerage overlap belong to later
CUR-018 work.

<a id="c10"></a>

## C10: Organization and deterministic rules, owner CUR-019

Files: `crates/currents-domain/src/{organization,rules}.rs`.

`OrganizationRequestV1 { economic_ids, field, value, allocation, provenance, override_action }` is the protocol `SetOrganization` payload; its bytes are unchanged and no route maps it yet. `RuleDefinitionV1 { id,version,scope,priority,predicate,allowed_operation,adopted_value }`; `RuleGrantV1 { grant_id,rule_id,version,accounts,conditions,operation_kinds,revoked }`. `OrganizationEngine::set(state,&OrganizationChangeV1)->Result<OrganizationPreviewV1,EngineError>`; `RuleEvaluator::propose(state,&RuleDefinitionV1)->Result<Vec<DecisionPlan>,EngineError>`. Only service-internal authorized evaluator can adopt with an active exact grant. AI proposals contain input field/model/version provenance and never a grant; no remote AI adapter in v1. Categories partition signed amounts; tags nonadditive; hierarchy deduplicates stable leg IDs; conflicting equal-priority rules remain conflicts.

### CUR-019 owner organization records

```text
OrganizationRecordV1 { id, version, kind, targets, parent, text, allocations, supersedes }
OrganizationFieldV1 = Party | Category | Tag
OrganizationSourceV1 = Owner | Rule(RuleSourceV1 { rule, version, grant:Option<GrantId> }) | Proposal(RecordId)
OrganizationAssignmentV1 { id, schema_version, version, supersedes:Option<RecordId>, field:OrganizationFieldV1, legs:Vec<LegId>, value:Option<RecordId>, override_action:OverrideActionV1, source:OrganizationSourceV1 }
OrganizationChangeV1 { records:Vec<OrganizationRecordV1>, values:Vec<LegValueRequestV1> }
LegValueRequestV1 { field, legs:Vec<LegId>, value:Option<RecordId>, action:OverrideActionV1 }
OrganizationPreviewV1 { plan:DecisionPlan, skipped:Vec<LegId> }
DomainState::owner_value(leg,field)->Option<OwnerValueV1 { record, version, value }>
```

Parties, aliases, categories, tags and notes are `OrganizationRecordV1`
versions; its fields and bytes are unchanged. A party, category or tag value
on legs is an `OrganizationAssignmentV1` version, schema version 1.
`AuthoritativeRecordV1` adds `OrganizationAssignment(OrganizationAssignmentV1)`.
An edit appends the next version and never changes an earlier one.

`OrganizationEngine::set` previews one owner change: the change's records
first, then one assignment per changed leg. Its plan has no invalidations and
`AccountingIntent::NoEffect`. A leg's assignment for a field is the next
version of its latest one, including a version that cleared the value, or a
first version; its ID derives from the field, the leg and the version it
supersedes. `Preserve` sets a value only on legs without one and lists the
others in `skipped`. `Replace` supersedes the current value. `Clear` releases
it with a version whose `value` is null; a leg with no value has nothing to
clear and is refused.

`set` refuses, with no plan, an empty change, a value request with no legs or
a leg twice, and a request whose `value` is null for any action but `Clear`.
It and replay both refuse:

- a record whose ID is nil, already recorded, or used twice in the commit;
- a first version other than version 1, or a revision that is not the next
  version of the version it supersedes, changes its kind or field, or forks a
  version another record already supersedes;
- a party, alias, category or tag with targets; a party with a parent; a
  category or tag whose parent is not the first version of another of its
  kind; an alias whose parent is not a party's first version; and a parent
  chain, read through each definition's latest version, that returns to where
  it started;
- a note with a parent, with no targets or a target twice, or with a target
  that is not a `FinancialLeg` record, such as a recognition or posting ID;
- an assignment on no legs, a leg twice or a missing leg; a `Clear` that names
  a value or supersedes a version without one; another action whose value is
  not the first version of a definition of the field's kind; a `Preserve` that
  supersedes a value; and a revision whose legs differ from its predecessor's;
- two latest owner assignments for one leg and field.

Replay refuses, with a version error, an organization record with allocations
or of kind `ReportingAllocation`, an assignment of another schema version, and
an assignment whose source is not `Owner` until later CUR-019 leaves define
rules and proposals. Storage checks that an assignment's legs are
`FinancialLeg` records, its value an `Organization` record and its
predecessor an `OrganizationAssignment`. It also refuses a second successor
of one assignment version, and a rule or proposal source.

`owner_value` gives the latest owner assignment that holds the leg for the
field, its version and the definition it names, or `None` when there is none
or it clears the value. Replay through an earlier commit reports that
commit's value.

Corrections and segment replacements refuse a change while an assignment
names a leg they retire, until a later CUR-019 leaf defines its remap.
`OrganizationRecordV1` successors in corrections work as before. Precedence
beyond the owner tier, alias matching, rules, grants, proposals, splits, tag
counting and the command route belong to later work.

<a id="c11"></a>

## C11: Investments/FX/manual, owners CUR-021–024

Files: `crates/currents-domain/src/investments/{quantity,basis,valuation}.rs`, `crates/currents-domain/src/{fx,loans,manual}.rs`.

`TradeRequestV1` specifies adopted security identity, cash/quantity legs, fees, dates and source references. `LotSelectionV1 = Fifo | Explicit { acquisitions, attributed_choice }`; `BasisResultV1 = Known { carrying_cost,gross_gain,fees,net_result,remainder } | Partial { known,unknown_quantity } | Unknown { reason }`. `InvestmentEngine::recognize(state,&TradeRequestV1)->Result<DecisionPlan,EngineError>`; `BasisEngine::allocate(state,disposal:LegId,selection:LotSelectionV1)->Result<BasisResultV1,EngineError>`; `ValuationEngine::value(state,basis:&ReadBasis,account:AccountId)->Result<PositionResponseV1,EngineError>`. Supported corporate-action payloads specify split numerator/denominator, effective date and resulting basis; cash-in-lieu distinct. Unknown cohorts participate in FIFO eligibility; transfers preserve lineage.

### CUR-021 securities and positions

```text
SecuritySourceIdV1 { source, identifier }
SecurityMappingV1 { identifier:SecuritySourceIdV1, reason }
SecurityV1 { id, schema_version, root, prior_revision, unit_id, source_identifiers, display_symbol, local_mappings }
SecurityDeclarationV1 { security:SecurityV1, unit:UnitDefinition, unit_catalog:RecordId }
PositionV1 { id, schema_version, security, quantity_account, carrying_account }
```

A security is identified by its source identifiers and the owner's local
mappings, never by its display symbol. `AuthoritativeRecordV1` adds
`Security(SecurityV1)` and `Position(PositionV1)`, both schema version 1.

`InvestmentEngine::declare_security(state,&SecurityDeclarationV1)` declares a
new security, whose first revision's `id` is its `root`. The security gets its
own unit: `UnitKind::Security`, exponent 0 to 18, and an ID and native code
unused by any declared unit. The plan holds a new `UnitCatalogV1` (version 1, that one
unit) and the security record, and its intent is a metadata declaration of the
unit, so it commits before any financial preview. The planner refuses a
security with no source identifier, and any identifier, source or mapped,
already held by another security.

`InvestmentEngine::revise_security(state,&SecurityV1)` replaces the head named
by `prior_revision`. A revision keeps the unit and every identifier and mapping,
may add identifiers and mappings, and has no accounting effect. A mapping joins
another source's identifier to this security; one that would join two
securities is refused, since each has its own unit. `current` returns a
security's head and refuses a forked chain. `resolve` returns the security an
identifier names, or none.

`InvestmentEngine::declare_position(state,&PositionV1)` links a security to an
external quantity account in its unit and an `InvestmentCarrying` account in a
currency. Each account belongs to at most one position. The plan holds the
record and has no accounting effect.

Replay derives each security and position commit again from the prior prefix.
A new security's unit must be declared by a unit catalog in the same commit,
and a unit catalog that declares a security unit needs that security record.
Openings, snapshots, corporate actions, transfers and valuation belong to the
later CUR-021 leaves; basis and lots to CUR-022. There is no command route.

### CUR-021 trades

```text
TradeSideV1 = Buy | Sell
TradeRequestV1 { schema_version, id, position, side, event:FinancialEventV1, cash:SourceValue<AdoptedLegV1>, quantity:SourceValue<AdoptedLegV1>, fee:Amount, terms:RecognitionTermsV1, interpretation, governing_policy }
TradeV1 { id, schema_version, position, side, event_id, cash:SourceValue<AdoptedFacetV1>, quantity:SourceValue<AdoptedFacetV1>, fee, recognition, interpretation, governing_policy }
```

`InvestmentEngine::recognize(state,&TradeRequestV1)` plans one buy or sale of
a position's security. Each side is `Known`, one leg bound to one movement
fact, or `Missing`. The cash leg moves an open external account in the
carrying account's currency; the quantity leg moves the position's quantity
account in the security's unit. Each observation keeps C07's one facet, so the
two sides bind separate rows. `terms` holds the recognition's identities,
postings and dates, with no unresolved account or opening. The planner refuses
a request with neither side known, a fact another adoption or recognition
already binds, and a movement at or before a supported opening's cutoff.

The known sides post in one recognition, so they freeze as one C05 entry and
commit together. Each unit balances on its own. A known buy posts the cash
paid to the carrying account, so its fee is capitalized. A known sale posts
the cash received, its fee once to an `ExpenseFlow` account, and its gross
proceeds to a `DisposalClearing` account; it records no gain and leaves the
carrying amount alone. A known quantity posts the quantity account against a
`QuantityControl` account. A missing side posts nothing, and a fee needs a
known cash side. The planner refuses a posting in another unit than its
account's, any other account, and an amount beyond its unit's exponent; it
never rounds. The plan holds the event, the known legs, a version 1
`TradeV1` record and the recognition. `AuthoritativeRecordV1` adds
`Trade(TradeV1)`.

`InvestmentEngine::holdings(state,position,through)` reads one position
through a date: the quantity and carrying amount that live recognitions post
on or before it, and the trades by then whose quantity or cash side is
missing. Replay restores each trade's request from its records and derives
the plan again from the prior prefix; C05 then checks the frozen entry against
the recognition, so a batch that holds only one side's postings is refused.
Every correction path and segment replacement refuses a recognition that
holds a security-unit posting, or a closure that includes one, until CUR-022
extends `ReplacedLegs` to trade quantity legs.

`FxRequestV1 { source_amount,destination_amount,quote,quote_direction,quote_time,source,fees,rounding,residual }`; `FxEngine::recognize(state,&FxRequestV1)->Result<DecisionPlan,EngineError>`. `ManualPositionV1 { component,amount,as_of,method,evidence,ownership_share }`; `LoanPaymentV1 { cash,principal,interest,fees,unknown_allocation,source }`; `ManualEngine::observe(state,&ManualPositionV1)->Result<DecisionPlan,EngineError>`; `LoanEngine::recognize(state,&LoanPaymentV1)->Result<DecisionPlan,EngineError>`. These create no institution operations.

<a id="c12"></a>

## C12: Portable planning, owners CUR-025–028

CUR025 implements the commitment/calendar subset in [portable commitments](../portable-commitments.md). `CommitmentV1` additionally retains `series_generation:u32`, `calendar_data:Box<ForecastCalendarDataV1>`, and `supersession:Option<CommitmentSupersessionV1 { prior_version:u32,prior_generation:u32,from_ordinal:CommitSeq,not_before:CivilDateV1,remaps:Vec<OccurrenceRemapV1 { old:OccurrenceKeyV1,new:Option<OccurrenceKeyV1> }> }>`. `not_before` is the immutable exact inclusive predecessor boundary: the maximum of the inherited boundary, the nominal prior cutoff date, and one civil day after each still-active pre-cutoff exception. An unrepresentable successor fails visibly. Every revision must start at or after this bound; later transitions inherit it. `recurrence_version` remains the algorithm version. Snapshot/scalar timezone fields must agree. `CalendarEngine::new(zone:&str,version:&str,retained_tzif:&[u8])` validates the bounded retained bytes; its `expand(&self,commitment,start,months)` uses only that exact evidence. `CommitmentEngine::new(calendar)` supplies the pure `set(&self,state,commitment)` and `expand_history`; `resolve_occurrence` resolves explicit retirement/remapping across immutable accepted revisions. This adds no matching allocations, forecasts or accounting authority.

Files: `crates/currents-planning/src/{commitments,calendar,matches,forecast,recurrence,compare}.rs`.

`CommitmentV1` records all design section 9 fields: ID/version, low/base/high/nominal amounts, typed account legs, due/settlement rules, recurrence/calendar/timezone versions, start/end/exceptions/provenance. `OccurrenceKeyV1 { commitment_id,series_generation,ordinal }`; `OccurrenceV1 { key,version,legs,due_date,settlement_range }`; `OccurrenceMatchV1 { occurrence,planned_leg,economic_leg,allocation_version,amount,inclusion,pending }`. `CommitmentEngine::set(state,&CommitmentV1)->Result<DecisionPlan,EngineError>`; `CalendarEngine::expand(commitment:&CommitmentV1,start:Date,months:u8)->Result<Vec<OccurrenceV1>,EngineError>`; months only1/3/12 for product forecasts. `MatchEngine::set(state,&OccurrenceMatchV1)->Result<DecisionPlan,EngineError>` conserves per-leg actual and expectation allocations.

`ForecastRequestV1 { basis,start,months,commitment_versions,assumption_versions,scenario }`; `ForecastRunV1 { id,request,expanded_inputs,calendar_data,algorithm_version,outputs }`; `ForecastEngine::run(state:&DomainState,request:&ForecastRequestV1)->Result<ForecastRunV1,EngineError>`; `ForecastEngine::save(state,&ForecastRunV1)->Result<DecisionPlan,EngineError>`; `ForecastEngine::compare(state,saved:RecordId,basis:&ReadBasis)->Result<ForecastComparisonV1,EngineError>`. Outputs include daily per-account amounts/min/date, unknowns/omissions, inclusion maps, assumptions, variants and matching conflicts. Planning has no AccountingAdapter/native-storage dependency. `RecurrenceEngine::suggest(state,basis:&ReadBasis)->Result<Vec<RecurrenceProposalV1>,EngineError>`; `VariableRateEngine::suggest(state,basis,account,category)->Result<VariableAssumptionV1,EngineError>` follow exact deterministic design section 9 thresholds. Proposals cannot mutate commitments.

<a id="c13"></a>

## C13: Archive and lifecycle, owners CUR-029, CUR-031–033

Files: `crates/currents-storage/src/{archive,generations,migration,retention,backup}.rs`, `crates/currents-service/src/{archive,lifecycle}.rs`, `crates/currents-verify/src/main.rs`, `native/CurrentsPlatform/Sources/{KeyVault,LockCoordinator}.swift`.

`ArchiveManifestV1` includes kind Full|Ingress|Acknowledgment, cipher/profile/version, dataset/cutoff, semantic definitions, reference closure, counts/digests, journal/domain hashes, coverage and key ID; no secret key. `ArchiveLimits { encrypted_bytes:64 GiB,records:10000000,chunk_bytes:1 MiB }`. `ArchiveContainer::write_full(source:&HistoryReader,destination:&Path,key:RecoveryKey,limits:ArchiveLimits)->Result<ContainerReport,EngineError>` and `ArchiveContainer::open_full(path:&Path,key:RecoveryKey)->Result<ValidatedArchiveSource,EngineError>` live in currents-storage and perform bounded cipher/schema/digest/reference-stream validation only, with no domain/service dependency. `ContainerReport` records byte/record counts and digests; `ValidatedArchiveSource` owns the read-only verified file identity and record stream. `ArchiveCoordinator::export_full(source:&HistoryReader,destination:&Path,key:RecoveryKey,limits:ArchiveLimits)->Result<ArchiveReport,EngineError>` and `ArchiveCoordinator::verify_full(path:&Path,key:RecoveryKey)->Result<VerifiedFullArchive,EngineError>` live in currents-service, invoke container operations plus ReplayCoordinator, and certify full semantic/native replay before completed publication. currents-verify invokes these shared coordinators headlessly. `TransferArchiveWriter::write(payload:TransferPayloadV1,destination:&Path,key:IngressKey)->Result<ArchiveReport,EngineError>` and `TransferArchiveReader::read(path:&Path,key:IngressKey)->Result<TransferPayloadV1,EngineError>` use distinct compile-time key wrappers and variant tags; they cannot call full restore. Here `|` denotes the named enum `TransferPayloadV1 = Bundle(IngressBundleV1) | Ack(IngressAckV1)`, not an untyped union. Paths accepted only by authorized host maintenance, never arbitrary MCP queries. Full archive success requires isolated single-file fresh-process reopen and full dataset replay. Ingress/ack success instead verifies authentication, kind/schema, bounded reference/digest closure and isolated single-file reopen; these evidence-only payloads have no dataset journal to replay. Reader rejects executable schema objects, extensions and unknown tables, and is defensive/query-only.

`VerifiedFullArchive { report:ArchiveReport, source:ValidatedArchiveSource }` is the output of verify_full with checked full kind/closure; ValidatedArchiveHandle is an opaque read-only retained source handle that preserves the verified file identity and cannot be constructed by a caller; `ActiveGenerationRef` identifies the closed audited source generation, and `MigrationTargetV1` names supported schema/readers. `GenerationInputV1 = Restore { archive:VerifiedFullArchive,new_data_key:DatasetKey } | Migrate { source:ActiveGenerationRef,target:MigrationTargetV1 } | RotateDataKey { source:ActiveGenerationRef,new_data_key:DatasetKey }`. No ingress archive can satisfy a full restore. `GenerationId`, `KeyId`, `GenerationMarkerV1 { generation,key_id,instance_id }`; `GenerationCoordinator::stage(input:GenerationInputV1)->Result<StagedGeneration,EngineError>`; `GenerationCoordinator::activate(staged:StagedGeneration)->Result<ActivationReport,EngineError>`. GenerationCoordinator and VerifiedFullArchive live in currents-service, not storage. Storage generation I/O only creates/closes files, flushes and publishes the marker; it never imports service or calls domain replay. The coordinator supplies verified staged data, orchestrates full replay/backup checks and owns the high-level activation result. Hold exclusive lock; await close; persist key first; verify and flush DB/sidecars/generation; write/flush/rename CURRENT; flush registry directory; reopen/full replay; retain old keys/generation through verified backup. `KeyVault` Swift protocol: `createDatasetKey`, `loadDatasetKey`, `storeRecoveryKeyWithConsent`, `rotateIngressKey`, `retireKey` use opaque handles, protected Keychain and LocalAuthentication; Rust receives secrets through private memory API, not wire history. `LockCoordinator.lock(reason:) async` returns fullyLocked only after admission stop/revoke/drain/close.

`BackupPolicy::evaluate(state:&ProtectionState,now:OffsetDateTime)->BackupAction` encodes 24 hours changed-data/due-on-unlock/degraded status and no pruning. `RetentionEngine::plan(request:RetentionRequestV1)->Result<RetentionPlanV1,EngineError>` supports seven-day transient cleanup, disconnect credential removal and explicitly confirmed whole dataset destruction, never selective committed erasure. Key retirement/removal is separately recorded; unreachable archives remain disclosed.

<a id="c14"></a>

## C14: FFI and Mac transport, owners CUR-034–036

Files: `crates/currents-ffi/src/{lib,handles,buffers}.rs`, `native/CurrentsPlatform/Sources/{EngineActor,XPCClient,OwnerService,AuthorizationHelper}.swift`, `include/currents.h`.

C ABI v1 uses uint64 generation-tagged handles and lengths, never raw Rust object pointers. `currents_owner_open_v1(config_bytes,len,key_bytes,key_len,out_service)->Status` (private trusted host entry point); `currents_submit_v1(session,request,len,out_request)->Status`; `currents_poll_v1(session,request,out_buffer,out_len)->Status`; `currents_copy_v1(buffer,destination,capacity,out_written)->Status`; `currents_release_v1(buffer)->Status`; `currents_cancel_v1(session,request)->Status`; `currents_command_status_v1(session,request,len,out_request)->Status`; `currents_close_v1(handle)->Status`. C callers must provide valid writable output pointers/buffers with required capacity and lifetime; arbitrary pointer validity cannot be established by Rust. The ABI checks nulls, lengths, capacities and generation tokens before use; lengths bounded before allocation; buffers have explicit one-owner lifetime. `Status` is fixed integer discriminant with golden layout. The private owner_open receives raw key bytes in separately sized memory, copies them into DatasetKey, and clears owned copies at close; host retains responsibility for its input memory lifetime/clearing. Keys are forbidden in config_bytes and all generic JSON. The host-private `currents_bind_session_v1(service,authenticated_context_bytes,len,out_session)->Status` returns a generation-tagged SessionHandle from host-verified RequestContext; it is never exported to XPC clients. `currents_revoke_session_v1(service,session)->Status` invalidates it. Submit/query (through submit), status, poll and cancel derive RequestContext from this session binding, never from caller DTO/actor/grant fields. Lock/revoke/owner close/reconnection invalidates all affected sessions; only a new authenticated bootstrap can rebind. Possession of the private ABI is inside the trusted owner process, not an external authorization mechanism. Release/copy buffer APIs enforce owning session association through the generation-tagged buffer table; untrusted client never receives ABI handles.

`EngineActor` Swift wrapper owns handle/runtime, response decoding and idempotent cleanup. The Mac installation bundles `native/Owner/currentsd`, `native/LaunchAgents/com.currents.owner.plist` with a MachServices bootstrap name, and a signed authorization helper. `SMAppService.agent(plistName:)` registers an on-demand per-user LaunchAgent only after explicit install/setup consent. `OwnerRegistration` implements status/register/unregister using ServiceManagement; denial or disabled background permission remains visible, and clients explain how to enable it rather than bypassing it. The installed app need not run for CLI/stdio clients to launch the single owner through launchd. Uninstall unregisters with approval and handles record retention separately; it never deletes a dataset as an installation side effect. XPC bootstrap sends no private payload until UID/code-signing requirements verified on live anonymous endpoint; reconnection invalidates trust and requires new auth. `CapabilityV1 { client,dataset,instance,session,accounts,fields,operations,expiry }` is ephemeral. Trusted approval helper binds challenge/digest/session/instance, five-minute one-use token and fresh LocalAuthentication. Scope filtering applies before rows/aggregate/errors/counts/explanation construction, not just response redaction.

<a id="c15"></a>

## C15: Mobile ingress, owners CUR-044–046

Files: `native/CurrentsCapture/Sources/{CaptureQueue,FinanceKitAdapter,Pairing,BundleTransfer}.swift`, `crates/currents-acquisition/src/mobile.rs`.

`PairingV1 { pairing_id,device_id,epoch,source_scope,revoked }`; `IngressBundleV1 { pairing_id,epoch,batch_number,previous_digest,capture_metadata,evidence }`; `IngressAckV1 { pairing_id,epoch,batch_number,bundle_digest,commit }`. All in C13 SQLCipher transfer envelopes under unique ingress key, not data/recovery keys. `MobileIngress::accept(state,&IngressBundleV1)->Result<DecisionPlan,EngineError>` deduplicates batch/digest and preserves gaps; acknowledgment emitted after durable receipt. `CaptureQueue` retains until authenticated matching ack, encrypted/device-protected; one-time owner-verified QR pairing, system file handoff; no automatic cloud target. Rotation waits for ack or immediate revocation invalidates old epoch, then repackages retained evidence with original capture metadata.

<a id="c16"></a>

## C16: Production gates, owners CUR-004, CUR-042, CUR-044, CUR-047, CUR-048, CUR-053, CUR-054, CUR-056, CUR-057, CUR-058

`QualificationReportV1 { gate,inputs,platform,versions,tests,evidence,status,limitations }` is a non-financial build artifact. `ProductionReadinessV1` is a local checklist with account inventory/ownership/timezone/reporting currency; adopted source precedence/freshness/openings/withholding; provider scope/coverage/cost/consent; Apple route and explicit owner-accepted limitations; independent backup/offline recovery proof; signed-client and optional agent scope proof. Gate statuses are `Unassessed|Blocked|Passed|OwnerAcceptedLimitation`; limitations cannot waive constitutional invariants. No due dates, owners' identities or private financial details appear in public reports. Every ticket has one accountable role; the role is assigned to a person at implementation kickoff, not an invented external assignee.

<a id="c17"></a>

## C17: Transfer-only iOS library, owner CUR-058

Files: `crates/currents-transfer/src/{lib,archive,ffi}.rs`, `include/currents_transfer.h`, `native/CurrentsCapture/Sources/TransferActor.swift`. Build staticlib/XCFramework for aarch64-apple-ios and aarch64-apple-ios-sim. The locked libsqlite3-sys 0.37.0 source selects Apple crypto when host and target contain apple; qualify that exact provider and prevent OpenSSL override leakage. Source support is not device qualification.

`TransferArchive::encode(payload:TransferPayloadV1,key:IngressKey,destination:&Path)->Result<ContainerReport,EngineError>` and `TransferArchive::decode(path:&Path,key:IngressKey)->Result<TransferPayloadV1,EngineError>` share the documented bounded ingress/ack schema and canonical bytes with the Mac adapter. Container verification includes profile/key, kind/schema, references/digests, gap metadata and self-contained finalization, never full domain replay. The crate cannot reference domain/accounting/service or accept DatasetKey/RecoveryKey. It uses private currents-cipher connection initialization and explicit awaited shutdown, not the production owner-service ABI.

Private bounded C functions are `currents_transfer_encode_v1(payload,len,key,key_len,destination,destination_len,out_operation)->Status`, `currents_transfer_decode_v1(archive_bytes,len,key,key_len,out_operation)->Status`, `currents_transfer_poll_v1(operation,out_buffer,out_len)->Status`, `currents_transfer_copy_v1(buffer,destination,capacity,out_written)->Status`, `currents_transfer_release_v1(buffer)->Status` and `currents_transfer_close_v1(operation)->Status`. Paths are supplied only by the trusted capture app's protected staging directory, never by source contents. Opaque operation/buffer handles are generation-tagged; explicit input/key lifetime and valid C memory obligations, limits, panic containment and actor cleanup match C14 without its owner/session authority. Copy/release handles belong to their operation. No financial command,full restore or owner-key/session API is linked/exported.

`TransferActor` keeps work off the main actor and controls device-protected temporary files and key memory. Wrong key/profile/kind fails closed; all final files/acks reopen without sidecars. Actual device and simulator tests include canaries, interrupted closure, duplicate/late buffers, debug/release memory harness and Mac↔iOS cross-process canonical interoperability. Desktop crypto proof is not substituted.

### CUR-011 implementation clarification

The synthetic headless verifier may construct host-private `DatasetKey` and
`CipherProfileV1` through an explicit dependency on `currents-cipher`; it still
cannot use SQLx/sys, native accounting, network or Apple APIs. The documented
public fixture key is never private-data encryption. C06 integration tests may
use dev-only storage→service/domain edges; production storage remains service-free.

C06 `DomainState.domain_digest` records exact accepted prefix continuity beside
its nonauthoritative latest-ID index. Full comparison hashes ordered authority,
not that lossy index. `Service::recover_into(destination)` requires a fresh
host-supplied encrypted shadow, holds serial exclusion, verifies complete
correspondence and unchanged source, and only then publishes the verified prefix.
The old `recover()` returns `IncompleteBasis/Recover`; it cannot certify replay.
A separately rebuilt destination does not repair or grant write authority to a
damaged source. Active file/key generation switching remains C13/CUR-032.

C08 replay reconstructs exact DatasetPolicy history atomically. New catalog/adoption
records are checked against the exact prior command-admission prefix and their
complete emitted record set; a future policy cannot repair an earlier reference.
C06 then performs frozen native correspondence and the C08 semantic audit before
activating the projection. Replay re-admits a replacement segment only when the
plan derived again from the prior prefix matches it exactly. Unsupported
association, general allocation and occurrence semantics remain fail-closed. S1 fixtures are explicitly
synthetic typed-history evidence, not provider export grammars.
