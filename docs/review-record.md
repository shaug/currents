# Currents design and implementation review record

Date: 2026-11-09 UTC. Scope: documentation and source-backed feasibility review, not application implementation or production qualification.

## Technical design

Reviewed artifact: [technical-design.md](technical-design.md).

Converged SHA-256: `03cd2988bac8e02bd5757f45196aac6efae5f57a56806ce3d33857892c68e5b4`.

Two independent adversarial review tracks examined accounting/domain semantics and runtime/storage/security. Each identified concrete counterexamples, those defects were corrected, and fresh closure passes examined the same final document. The closure criterion was no remaining reasonably addressable material finding within those scopes, not a fixed number of review passes.

| Finding class | Resolution in the design |
|---|---|
| One source movement recognized through multiple new leg IDs | Source-fact facet allocation ownership across revision/parser lineage, in addition to economic-leg recognition ownership. |
| Backfill blocked by native period/account/unit effective limits | Explicit full supported date range, one unsealed native period, fixed native policy and domain-owned lifecycle. |
| Variable wire scale confused with native store precision | One native scale of 18; exact rescaling, per-unit divisibility, overflow rejection and stable compatible unit codes. |
| Missing native booking/value-date mapping and reversal behavior | Recorded bookkeeping policy, retained nullable source dates, explicit original dates for reversals and domain-date reports. |
| Native batch and declaration limits | Metadata-only precommits; separate qualification of multi-operation composition in one transaction; bounded atomic closures and visible refusal beyond limits. |
| Unknown or future inventory used as known basis | Unknown/undated cohorts participate in eligibility; date-constrained allocation; unknown basis remains unavailable. |
| Sale fees counted twice | Acquisition fees capitalized; gross disposal gain and selling expense recorded separately; net result deducts fees once. |
| Digest self-reference and replay entropy | Explicit preimage exclusions, disabled native memos, stable frozen operations and outer domain/native-head mapping. |
| COMMIT error misreported as rollback | Preserve uncertain outcomes; reopen/audit/status resolution before a definitive conclusion. |
| Filesystem/key-store activation treated as one atomic mutation | Flush-ordered generation selection; install new key first; retain prior generation/key until verified reopen and backup. |
| Single-file backup depended on WAL sidecars | Separate archive connection profile, full close/flush, no-clobber publication and isolated single-file reopen. |
| Reauthorization broke identical retry | Stable command body separated from ephemeral authorization envelope. |
| Cancellation or absent result row misclassified an in-flight write | Explicit pending/unknown/committed/definitely-not-committed state machine and writer-settlement barrier. |

The design also retains the earlier reviewed contracts for complete household scope, asynchronous positions, reconciliation, category/merchant meaning, per-leg forecasting, Apple Card acquisition, private financial state and independent recovery. The constitution and its provenance remain unchanged.

### Residual uncertainty and implementation gates

No production Rust workspace or application was built in this documentation task. The historical spikes proved bounded feasibility, not production readiness. In particular, the following remain mandatory implementation acceptance gates:

- SQLCipher linking, secure connection initialization, key-safe diagnostics, explicit shutdown and encrypted recovery on each supported platform.
- The new multi-operation same-transaction seam, with unchanged native validation/canonical encoding and full failure/replay conformance.
- A unified persistent encrypted Rust service and hardened Swift boundary, not an assumed combination of the separate prototypes.
- Platform authentication, signing, minimum-OS behavior, independent loss-of-device restore, migrations and realistic performance.
- Disk-full, I/O and applicable filesystem durability behavior. Process-kill tests do not establish physical power-loss tolerance.
- Owner account inventory, read-only provider eligibility/consent, Apple Card route, backup destination and production release setup.

Review convergence does not waive these gates. Any failed gate that changes the selected contract requires a documented design revision and renewed review.

## Historical implementation-plan review (2026-11-09)

Reviewed artifacts: the implementation plan (retired from the current tree; retained in Git history), [shared contracts](planning/contracts.md), [dependency decisions](planning/dependencies.md), and the [issue manifest](planning/manifest.json).

Converged implementation-plan SHA-256: `11c2fe49ae78fcb993c7919c068ae1d2f0f971ea508917e067769769aaa50669`.

Converged manifest SHA-256: `1f717201abc45371be5352b6829158b4b80efdd11993479424b7063127aa3bf9`.

The separate plan underwent two independent adversarial tracks: implementation contracts/Rust/runtime quality, and coverage/sequencing/ticket quality. Eight material contract findings and three material coverage findings were corrected, then fresh closure passes inspected the same final artifacts. No remaining reasonably addressable material finding was identified within those scopes.

| Finding class | Resolution in the plan |
|---|---|
| Domain depended on storage or replay coordination created a crate cycle | Pure domain replay/evidence types; service-level replay, archive certification and generation coordination; bounded storage streams. |
| Test paths and package commands disagreed | Explicit suite ownership and static package/path consistency checks. |
| Caller input could substitute for trusted native identity | Private host session binding, revocation and generation-tagged handles; spoofed-context tests. |
| Verification dispatcher called itself | Concrete suite commands and native handlers; recursive-alias validation. |
| Archive verification returned no usable restore proof or required replay for evidence-only bundles | Service-owned verified full-archive capability; separate bounded ingress/ack verification. |
| Final acceptance required itself to have already passed | Explicit prerequisite-only gate check, followed by acceptance persistence and final audit. |
| CLI worked only while the GUI had started an owner service | Consented per-user SMAppService LaunchAgent, cold CLI launch and concurrent-client tests. |
| Authorization reused session-start time | Fresh trusted authorization time at admission and queued execution, separate from pinned financial read time. |
| Private activation could precede signed host qualification | Separate early signed-synthetic host gate, with implemented CLI/MCP predecessors, before private activation. |
| Apple real-account coverage preceded the protected device implementation | Route research, synthetic capture, iOS-only encrypted transfer qualification and protected live coverage are distinct tickets/gates. |
| Native setup, manual/loan entry and history/report journeys were missing | Typed durable catalog/policy/manual/loan commands and named native view files and acceptance tests. |

The iOS transfer-only Rust/Swift component has explicit device/simulator builds, encrypted-container interoperability and restricted authority. Its chosen provisioning, entitlement, real-device installation/update and distribution route are separate from Mac notarization. These are future qualification obligations, not passed platform results.

### Validation actually run at review closure (2026-11-09)

- 58 implementation/qualification tickets, 9 epic issues, 7 milestones and 14 label definitions.
- 100 mapped requirement records, 222 valid acyclic hard-dependency edges and exactly one epic membership per ticket.
- 67 complete proposed issue bodies, with 255 acceptance assertions across the leaf tickets.
- 498 local document links/anchors, package/test-path consistency, matching design hash and public-material marker scan passed.
- Independent comparison of generated bodies with manifest scope, completion criteria, commands and assertions passed.
- Deterministic regeneration of all 69 generated outputs passed.
- Constitution and provenance remain byte-for-byte unchanged. The original preliminary proposal is retained with a supersession banner.

Run `python3 docs/planning/validate.py` to repeat the structural checks. They do not replace semantic review or future application tests.

### Completion boundary at review closure

At review closure, the local design, separate implementation plan, reviews and complete proposed GitHub payloads were ready. No application implementation, dependency installation, live financial access, signing, production qualification or application release was performed. At review closure, GitHub publication was pending confirmation of the repository target, visibility and publication scope. Publication was subsequently authorized and verified on 2026-11-09 at 12:19:31 UTC against source commit `980cb30687c91df13ab9027a28d9d3a7c9ef6ae2`: 67 complete issue bodies, 58 native parent-child links and 222 native blocked-by dependencies. The dated receipt and mapping are retained in Git history. That publication does not change the reviewed design/plan hashes or pass an implementation gate.

## Documentation consolidation (2026-11-09)

The implementation plan, publication receipt and redundant mapping were retired from the current tree. Their hashes and review facts above describe historical artifacts, not current document dependencies. Durable cross-ticket guidance now lives in the [engineering guide](engineering-guide.md). [GitHub issues](https://github.com/shaug/currents/issues), parents, dependencies and milestones own current work state. The [reviewed source package](planning/README.md), exact shared contracts, dependency decisions, requirement coverage and all acceptance assertions remain available. No application or production qualification gate is passed by this consolidation.
