# Portable commitments (CUR025)

C12 owns civil recurrence and durable planning revisions. The planning crate has
no accounting, storage, native, clock or filesystem capability. CommitmentEngine
returns the existing typed DecisionPlan with NoEffect accounting and explicit
commitment invalidation. C03's trusted owner persists this plan using the accepted
serialized service; passing time never calls recognition. Production remains
disabled. Matching allocations and forecasts remain with their respective owners.

## Explicit calendar evidence

CalendarEngine::new(zone, timezone_data_version, retained_tzif_bytes) checks size
before parsing and fingerprints the supplied bytes. New snapshots must exactly match
the pinned bundled IANA release. from_saved(calendar_data, retained_tzif_bytes)
restores an already accepted snapshot by exact digest/length/name/version even
after the bundled release changes. calendar_data() produces the
existing ForecastCalendarDataV1 snapshot with content digest, byte count and
application/tzif media type. The trusted owner retains these exact bytes through
accepted content history before accepting a commitment that references them.
The planning crate does not retrieve or stage content. A missing content object
fails the storage reference audit. A caller must supply retained bytes and their
recorded name/version, never substitute whatever tzdb happens to be installed.
Expansion compares the entire snapshot; legacy timezone, timezone_data_version
and calendar_version scalar fields must match it exactly. A changed digest,
length, zone or release label fails visibly. A newer library/database does not
rewrite an existing snapshot or expanded result.

The current calendar and recurrence algorithm versions are 1. Unsupported
versions fail rather than falling back. Civil dates remain dates. Only an
explicit local datetime conversion takes a ConversionPolicy: Reject, Earlier or
Later. Both nonexistent and repeated local times require that explicit choice.
No implicit midnight or process timezone is used.

Recurrences use the proleptic Gregorian calendar, anchored to the supplied due
date. Ordinals are zero-based and independent of the requested forecast window.
Daily and weekly intervals are whole positive integers. Monthly day rules clamp
without drifting the anchor; LastDay stays distinct from Day(30). Annual February
29 clamps in nonleap years. One, three and twelve calendar-month windows include
the clamped ending date. Start/end and explicit per-ordinal exceptions filter the
expansion. Default settlement is the due date, including weekends. Explicit next
or previous weekday rules are supported. Explicit ranges preserve the supplied
relative offset to the anchor. Addressable holiday-calendar definitions are not
yet supported by C02 and fail UnsupportedVersion; no bank calendar is inferred.

## Revision and supersession contract

CommitmentV1 adds series_generation, calendar_data and optional supersession.
The calendar_data field is boxed in Rust to keep the authoritative record enum
bounded; its JSON representation is the existing unwrapped calendar snapshot.
The existing recurrence_version remains the recurrence algorithm version, not a
series counter. New commitments start at commitment version 1 and generation 1.
Each edit advances commitment version exactly once. Amount/date edits retain the
same generation and occurrence keys; occurrence.version follows commitment.version.

A changed recurrence pattern must advance generation exactly once and name the
exact prior commitment version and generation in CommitmentSupersessionV1. Its
from_ordinal retires every prior-generation key at or after that ordinal unless
an explicit one-to-one old→new mapping is supplied. An explicit absent target
also records retirement. A mapping cannot name a different commitment, an
incorrect generation, a nonexistent ordinal, an ordinal before the cutoff, or a
duplicate target. The new series starts no earlier than the old cutoff date.
Old revisions are retained, not overwritten in authoritative history. Later
same-generation revisions preserve their generation transition unchanged.

resolve_occurrence consumes ordered, contiguous accepted revisions and resolves
old keys through every successive transition. A retired key and its attached
matches are inactive. A remapped key identifies the single target expectation;
it never contributes a second expectation. expand_history uses the latest
revision of each series and excludes retired/remapped source expectations.
These APIs define retirement only, not actual/pending allocation or matching
algorithms. Saved snapshots and results retain their original interpretation.

## Synthetic qualification

cargo test --locked -p currents-planning --test cur_025 covers the four named
portable assertions. cargo test --locked -p currents-service --test cur_025
covers actual synthetic C03 persistence, retained TZif content and reopen without
financial postings. cargo xtask ticket CUR-025 requires both suites and reports
the executed assertions/output digests. Schema fixtures remain structural
synthetic vectors, not usable household commitments or production qualification.
