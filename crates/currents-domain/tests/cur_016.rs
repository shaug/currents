//! CUR-016 segment replacement through the public preview, replay and C05 adapter.
//! Synthetic account A (id 10) is anchored at 3,300 USD at the end of 2026-11-01.
use currents_accounting::AccountingAdapter;
use currents_domain::{
    ReplayEngine,
    associations::IdentityEngine,
    catalog::CatalogEngine,
    dependents::OrganizationQuestionV1,
    recognition::RecognitionEngine,
    segments::{SegmentEngine, SegmentPreviewV1, SegmentReplacementV1, TransferJoinV1},
};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;
#[path = "support/associations.rs"]
#[allow(dead_code)]
mod association_fixtures;
#[path = "support/transfers.rs"]
mod transfers;

const OLD_SEGMENT: u32 = 707;
const OLD_OPENING: u32 = 703;
const EQUITY: u32 = 50;
const EXPENSE: u32 = 30;

fn account(n: u32) -> AccountId {
    AccountId(id(n).0)
}
fn recognition(n: u32) -> RecognitionId {
    RecognitionId(id(n).0)
}

/// One proposed commit with the exact frozen operations its manifest names
/// and the invalidations its result records.
#[derive(Clone)]
struct Candidate {
    commit: DomainCommitV1,
    bytes: Vec<u8>,
    batch: AccountingBatchV1,
}
fn candidate(
    state: &DomainState,
    mut records: Vec<AuthoritativeRecordV1>,
    invalidations: &[RecordId],
    batch: AccountingBatchV1,
) -> Candidate {
    let seq = CommitSeq(state.through.0 + 1);
    let manifest = AccountingAdapter::manifest(id(5000 + seq.0 as u32), &batch).unwrap();
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(manifest));
    let mut commit = DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records,
        decisions: vec![],
        native_head_after: NativeHead {
            semantics_version: 1,
            checkpoint: Digest([seq.0 as u8; 32]),
        },
        command_result: CommandResultV1 {
            command_id: CommandId(id(6000 + seq.0 as u32).0),
            digest: Digest([9; 32]),
            state: CommandState::Committed,
            commit: Some(seq),
            effects: EffectsV1 {
                records: vec![],
                accounts: vec![],
                amounts: vec![],
                invalidations: invalidations.to_vec(),
            },
            health: vec![],
        },
        digest: Digest([0; 32]),
    };
    commit.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&commit).unwrap()).into());
    let bytes = AccountingAdapter::instruction_bytes(&batch).unwrap();
    Candidate {
        commit,
        bytes,
        batch,
    }
}
fn planned(state: &DomainState, plan: &DecisionPlan) -> Candidate {
    let batch = AccountingAdapter::freeze(&plan.accounting, state).unwrap();
    candidate(state, plan.records.clone(), &plan.invalidations, batch)
}
/// Replay one commit as the service does: domain re-admission, the native
/// index and its correspondence, then the C08 audit of the whole projection.
fn admit(state: &DomainState, c: &Candidate) -> Result<DomainState, EngineError> {
    let mut projection = state.clone();
    ReplayEngine::apply(&mut projection, &c.commit)?;
    let mut native = state.clone();
    AccountingAdapter::advance_index(&mut native, &c.commit, &c.bytes)?;
    let index = native.accounting.ok_or_else(EngineError::invalid)?;
    AccountingAdapter::verify_correspondence(&c.commit, Some(&c.batch), &index)?;
    projection.accounting = Some(index);
    RecognitionEngine::audit(&projection)?;
    IdentityEngine::audit(&projection)?;
    Ok(projection)
}
type Snapshot = (
    CommitSeq,
    Digest,
    BTreeMap<RecordId, AuthoritativeRecordV1>,
    BTreeMap<RecognitionId, Vec<FrozenEntryV1>>,
    BTreeSet<RecognitionId>,
);
fn snapshot(state: &DomainState) -> Snapshot {
    let index = state.accounting.as_ref().unwrap();
    (
        state.through,
        state.domain_digest,
        state.records.clone(),
        index.recognitions.clone(),
        index.reversed.clone(),
    )
}
/// Native balances: every frozen posting of every recognition still active.
fn balances(state: &DomainState) -> BTreeMap<AccountId, Amount> {
    let index = state.accounting.as_ref().unwrap();
    let mut totals = BTreeMap::new();
    for (id, entries) in &index.recognitions {
        if index.reversed.contains(id) {
            continue;
        }
        for posting in entries.iter().flat_map(|e| &e.postings) {
            let total = totals
                .entry(posting.original.account_id)
                .or_insert(amount(0));
            *total = total.checked_add(&posting.original.amount).unwrap();
        }
    }
    totals
}
/// A batch with changed operations, its counts and digests bound again so
/// only the operations themselves differ.
fn rebound(mut batch: AccountingBatchV1) -> AccountingBatchV1 {
    let count = CommitSeq(batch.operations.len() as u64);
    batch.result_expectations = AccountingExpectationsV1 {
        native_operation_count: count,
        financial_entry_count: count,
    };
    batch.decision_digest = AccountingAdapter::prebinding_digest(&batch).unwrap();
    for op in &mut batch.operations {
        if let FrozenNativeOperationV1::Entry(e) = op {
            e.generated_document = batch.decision_digest;
        }
    }
    batch
}
fn refused(state: &DomainState, c: &Candidate) {
    let before = snapshot(state);
    assert!(admit(state, c).is_err());
    assert_eq!(snapshot(state), before);
}
/// The domain replay itself refuses the records and leaves the prior head.
fn replay_refused(state: &DomainState, c: &Candidate) {
    let mut copy = state.clone();
    assert!(ReplayEngine::apply(&mut copy, &c.commit).is_err());
    assert_eq!(snapshot(&copy), snapshot(state));
}

/// The contract fixture record of `kind`.
fn fixture_record(kind: &str) -> AuthoritativeRecordV1 {
    let fixtures: Vec<AuthoritativeRecordV1> =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    fixtures
        .into_iter()
        .find(|r| serde_json::to_value(r).unwrap()["kind"] == kind)
        .unwrap()
}
fn civil(day: &str) -> Date {
    let TemporalValue::Date { civil, .. } = date(day) else {
        panic!("civil date")
    };
    civil
}
/// Claim `n`, Matched on account `on` from `start` through `end` by
/// effective date, with no explicit dependencies.
fn claim(n: u32, on: u32, start: &str, end: &str) -> ReconciliationClaimV1 {
    let AuthoritativeRecordV1::ReconciliationClaim(mut claim) =
        fixture_record("ReconciliationClaim")
    else {
        panic!("claim fixture")
    };
    claim.id = id(n);
    claim.scope = vec![account(on)];
    claim.cutoffs = DateScope {
        start: Some(civil(start)),
        end: Some(civil(end)),
        basis: DateBasis::Effective,
    };
    claim.status = ReconciliationStatus::Matched;
    claim
}
/// Note `n` on `target`.
fn note(n: u32, target: RecordId) -> AuthoritativeRecordV1 {
    let AuthoritativeRecordV1::Organization(mut note) = fixture_record("Organization") else {
        panic!("organization fixture")
    };
    note.id = id(n);
    note.kind = OrganizationKind::Note;
    note.targets = vec![target];
    AuthoritativeRecordV1::Organization(note)
}
/// Commit `records` with no accounting effect, as their owners will.
fn recorded(state: &DomainState, records: Vec<AuthoritativeRecordV1>) -> DomainState {
    let reason = "Synthetic dependent records".into();
    let batch =
        AccountingAdapter::freeze(&AccountingIntent::NoEffect(NoEffectV1 { reason }), state)
            .unwrap();
    let mut c = candidate(state, records, &[], batch);
    c.commit.native_head_after = state
        .accounting
        .as_ref()
        .unwrap()
        .prefix
        .native_head
        .clone();
    c.commit.digest = Digest([0; 32]);
    c.commit.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&c.commit).unwrap()).into());
    admit(state, &c).unwrap()
}

fn anchored() -> DomainState {
    anchored_with(&[])
}
/// Account A anchored as above, its opening including the movement facts
/// `(evidence, amount, date)`.
fn anchored_with(included: &[(u32, i128, &str)]) -> DomainState {
    let mut state = monetary_state();
    for mapping in state.accounting.as_mut().unwrap().accounts.values_mut() {
        mapping.path = format!("Currents:A{}", mapping.account_id.0.simple());
    }
    let mut opening = supported_opening(&mut state);
    let facts = included
        .iter()
        .map(|&(n, value, day)| dated(&mut state, n, value, day, ObservationKind::Movement))
        .collect();
    let terms = opening.financial.as_mut().unwrap();
    terms.opening.as_mut().unwrap().included_activity = facts;
    let plan = RecognitionEngine::adopt(&state, &opening).unwrap();
    let state = admit(&state, &planned(&state, &plan)).unwrap();
    assert_eq!(balances(&state)[&account(10)], amount(3300));
    state
}
fn dated(
    state: &mut DomainState,
    n: u32,
    value: i128,
    day: &str,
    kind: ObservationKind,
) -> SourceFacetBindingV1 {
    let binding = evidence(state, n, value, None);
    let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
        state.records.get_mut(&id(n))
    else {
        panic!("observation missing")
    };
    observation.temporal = date(day);
    observation.semantic_kind = kind;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get_mut(&id(n + 1))
    else {
        panic!("parse product missing")
    };
    product.product.facets[0].temporal = date(day);
    product.product.facets[0].kind = kind;
    binding
}
fn at(request: &mut ActivityRequestV1, day: &str) {
    for leg in &mut request.legs {
        leg.leg.effective = date(day);
        leg.leg.posting = date(day);
    }
    let terms = request.financial.as_mut().unwrap();
    terms.bookkeeping_date = date(day);
    for posting in &mut terms.postings {
        posting.effective = date(day);
        posting.posting = date(day);
    }
}
/// A replacement with fresh identities from `base`: the opening draft, its
/// successor segment (`base + 17`) and each backfill against the expense account.
fn replacement_from(
    state: &mut DomainState,
    base: u32,
    opening: i128,
    cutoff: &str,
    backfill: &[(i128, &str)],
) -> SegmentReplacementV1 {
    let balance = dated(state, base, opening, cutoff, ObservationKind::Balance);
    let mut draft = posted(balance.clone(), base + 10, EQUITY);
    at(&mut draft, cutoff);
    draft.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: id(base + 17),
        component_id: id(11),
        amount: balance.amount,
        cutoff: date(cutoff),
        evidence: balance,
        included_activity: vec![],
    });
    let backfill: Vec<_> = backfill
        .iter()
        .zip(0u32..)
        .map(|(&(value, day), i)| {
            let n = base + 100 + 20 * i;
            let source = dated(state, n, value, day, ObservationKind::Movement);
            let mut movement = posted(source, n + 10, EXPENSE);
            at(&mut movement, day);
            movement
        })
        .collect();
    let mut approved_accounts = vec![account(10), account(EQUITY)];
    if !backfill.is_empty() {
        approved_accounts.push(account(EXPENSE));
    }
    SegmentReplacementV1 {
        prior_segment: id(OLD_SEGMENT),
        opening: draft,
        backfill,
        approved_accounts,
        transfers: vec![],
        reemissions: vec![],
    }
}
fn replacement(
    state: &mut DomainState,
    opening: i128,
    cutoff: &str,
    backfill: &[(i128, &str)],
) -> SegmentReplacementV1 {
    replacement_from(state, 3000, opening, cutoff, backfill)
}
fn ready(state: &DomainState, request: &SegmentReplacementV1) -> DecisionPlan {
    match SegmentEngine::replace(state, request) {
        Ok(SegmentPreviewV1::Ready(plan)) => *plan,
        other => panic!("expected a ready plan: {other:?}"),
    }
}
fn on_account(entry: &FrozenEntryV1, n: u32) -> Amount {
    entry
        .postings
        .iter()
        .find(|p| p.original.account_id == account(n))
        .unwrap()
        .original
        .amount
}
fn successor(plan: &DecisionPlan) -> &AccountingSegmentV1 {
    let mut segments = plan.records.iter().filter_map(|r| match r {
        AuthoritativeRecordV1::AccountingSegment(s) => Some(s),
        _ => None,
    });
    let segment = segments.next().unwrap();
    assert!(segments.next().is_none());
    segment
}

#[test]
fn cur_016_1_replacement_proves_old_boundary() {
    let mut state = anchored();
    let request = replacement(&mut state, 3400, "2026-10-30", &[(-100, "2026-10-31")]);
    let plan = ready(&state, &request);
    let AccountingIntent::Correction(correction) = &plan.accounting else {
        panic!("a replacement retires the old opening")
    };
    assert_eq!(correction.active_recognition, recognition(OLD_OPENING));
    let batch = AccountingAdapter::freeze(&plan.accounting, &state).unwrap();
    let entries: Vec<_> = batch
        .operations
        .iter()
        .map(|op| match op {
            FrozenNativeOperationV1::Entry(e) => e,
            other => panic!("unexpected native operation {other:?}"),
        })
        .collect();
    let [reversal, opening, backfill] = entries.as_slice() else {
        panic!("one reversal, one opening and one backfill: {entries:?}")
    };
    assert_eq!(reversal.recognition_id, recognition(OLD_OPENING));
    assert!(reversal.reverses.is_some());
    assert_eq!(reversal.booking_date, "2026-11-01");
    assert_eq!(reversal.value_date, "2026-11-01");
    assert_eq!(on_account(reversal, 10), amount(-3300));
    assert_eq!(on_account(reversal, EQUITY), amount(3300));
    assert_eq!(opening.recognition_id, recognition(3013));
    assert!(opening.reverses.is_none());
    assert_eq!(
        (opening.booking_date.as_str(), opening.value_date.as_str()),
        ("2026-10-30", "2026-10-30")
    );
    assert_eq!(on_account(opening, 10), amount(3400));
    assert_eq!(backfill.recognition_id, recognition(3113));
    assert_eq!(
        (backfill.booking_date.as_str(), backfill.value_date.as_str()),
        ("2026-10-31", "2026-10-31")
    );
    assert_eq!(on_account(backfill, 10), amount(-100));
    assert_eq!(on_account(backfill, EXPENSE), amount(100));
    let segment = successor(&plan);
    assert_eq!(segment.id, id(3017));
    assert_eq!(segment.prior_segment, Some(id(OLD_SEGMENT)));
    assert_eq!(segment.opening, recognition(3013));
    assert_eq!(segment.included_activity, [recognition(3113)]);
    assert_eq!(segment.replacement_closure, [recognition(OLD_OPENING)]);
    assert_eq!(
        segment
            .approved_accounts
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([account(10), account(EQUITY), account(EXPENSE)])
    );
}

#[test]
fn cur_016_1_unresolved_residual_blocks_without_plug() {
    let mut state = anchored();
    let request = replacement(&mut state, 3400, "2026-10-30", &[(-90, "2026-10-31")]);
    let before = snapshot(&state);
    let balances_before = balances(&state);
    assert_eq!(
        SegmentEngine::replace(&state, &request).unwrap(),
        SegmentPreviewV1::UnresolvedResidual(amount(10))
    );
    assert_eq!(snapshot(&state), before);
    assert_eq!(balances(&state), balances_before);
    assert_eq!(balances(&state)[&account(EQUITY)], amount(-3300));
}

#[test]
fn cur_016_anchor_correction_at_same_boundary() {
    let mut state = anchored();
    let request = replacement(&mut state, 3300, "2026-11-01", &[]);
    let plan = ready(&state, &request);
    assert!(successor(&plan).included_activity.is_empty());
    let replaced = admit(&state, &planned(&state, &plan)).unwrap();
    assert_eq!(balances(&replaced)[&account(10)], amount(3300));

    let mut state = anchored();
    let request = replacement(&mut state, 3250, "2026-11-01", &[]);
    assert_eq!(
        SegmentEngine::replace(&state, &request).unwrap(),
        SegmentPreviewV1::UnresolvedResidual(amount(-50))
    );
}

#[test]
fn cur_016_2_overlapping_active_segments_rejected() {
    let mut state = anchored();
    let request = replacement(&mut state, 3400, "2026-10-30", &[(-100, "2026-10-31")]);
    let other = replacement_from(
        &mut state,
        4000,
        3500,
        "2026-10-29",
        &[(-200, "2026-10-30")],
    );
    let plan = ready(&state, &request);
    let other_plan = ready(&state, &other);
    let first = planned(&state, &plan);
    let replaced = admit(&state, &first).unwrap();

    // The replaced segment is no longer active: a second replacement is refused.
    let before = snapshot(&replaced);
    assert!(SegmentEngine::replace(&replaced, &other).is_err());
    assert_eq!(snapshot(&replaced), before);
    // History cannot add a second successor to the same prior segment.
    let fork = candidate(
        &replaced,
        other_plan.records.clone(),
        &other_plan.invalidations,
        AccountingAdapter::freeze(&other_plan.accounting, &state).unwrap(),
    );
    replay_refused(&replaced, &fork);

    // A successor that names itself as its prior segment.
    let mut own = request.clone();
    own.opening
        .financial
        .as_mut()
        .unwrap()
        .opening
        .as_mut()
        .unwrap()
        .segment_id = id(OLD_SEGMENT);
    let before = snapshot(&state);
    assert!(SegmentEngine::replace(&state, &own).is_err());
    assert_eq!(snapshot(&state), before);
    let mut records = plan.records.clone();
    for record in &mut records {
        if let AuthoritativeRecordV1::AccountingSegment(s) = record {
            s.prior_segment = Some(s.id);
        }
    }
    replay_refused(
        &state,
        &candidate(&state, records, &plan.invalidations, first.batch.clone()),
    );

    // A commit that would leave two active segments for account A.
    let mut records = plan.records.clone();
    let mut second = successor(&plan).clone();
    second.id = id(3999);
    second.prior_segment = None;
    records.push(AuthoritativeRecordV1::AccountingSegment(second));
    replay_refused(
        &state,
        &candidate(&state, records, &plan.invalidations, first.batch.clone()),
    );
    assert_eq!(snapshot(&state), before);
}

#[test]
fn cur_016_replacement_rejects_bad_drafts() {
    let refuse = |change: &dyn Fn(&mut DomainState, &mut SegmentReplacementV1)| {
        let mut state = anchored();
        let mut request = replacement(&mut state, 3400, "2026-10-30", &[(-100, "2026-10-31")]);
        ready(&state, &request);
        change(&mut state, &mut request);
        let before = snapshot(&state);
        let result = SegmentEngine::replace(&state, &request);
        assert!(result.is_err(), "expected a refusal: {result:?}");
        assert_eq!(snapshot(&state), before);
    };
    // Backfill dated after the old cutoff, or at the new one.
    for day in ["2026-11-02", "2026-10-30"] {
        refuse(&|state, request| {
            *request = replacement(state, 3400, "2026-10-30", &[(-100, day)]);
        });
    }
    // Moving the opening to a later cutoff is outside this leaf.
    refuse(&|state, request| {
        *request = replacement(state, 3200, "2026-11-02", &[]);
    });
    // Backfill in another unit.
    refuse(&|_, request| {
        let other = UnitId(id(2).0);
        let draft = &mut request.backfill[0];
        let leg = &mut draft.legs[0];
        leg.leg.movement.unit_id = other;
        leg.leg.allocations[0].amount.unit_id = other;
        leg.sources[0].amount.unit_id = other;
        for posting in &mut draft.financial.as_mut().unwrap().postings {
            posting.amount.unit_id = other;
        }
    });
    // Backfill whose movement evidence is not retained.
    refuse(&|state, _| {
        state.records.remove(&id(3100));
        state.records.remove(&id(3101));
    });
    // Approved accounts must be exactly the accounts the plan touches.
    refuse(&|_, request| {
        request.approved_accounts.retain(|a| *a != account(EXPENSE));
    });
    refuse(&|_, request| request.approved_accounts.push(account(40)));
}

#[test]
fn cur_016_replay_readmits_only_the_exact_plan() {
    let mut state = anchored();
    let request = replacement(&mut state, 3400, "2026-10-30", &[(-100, "2026-10-31")]);
    let plan = ready(&state, &request);
    let good = planned(&state, &plan);
    let replaced = admit(&state, &good).unwrap();
    for kept in [id(OLD_SEGMENT), id(OLD_OPENING), id(700)] {
        assert_eq!(
            serde_json::to_vec(&replaced.records[&kept]).unwrap(),
            serde_json::to_vec(&state.records[&kept]).unwrap()
        );
    }
    let index = replaced.accounting.as_ref().unwrap();
    assert!(index.reversed.contains(&recognition(OLD_OPENING)));
    let totals = balances(&replaced);
    assert_eq!(totals[&account(10)], amount(3300));
    assert_eq!(totals[&account(EQUITY)], amount(-3400));
    assert_eq!(totals[&account(EXPENSE)], amount(100));

    // A changed amount in the typed records.
    let mut records = plan.records.clone();
    for record in &mut records {
        if let AuthoritativeRecordV1::Recognition(r) = record
            && r.id == recognition(3113)
        {
            r.postings[0].amount.coefficient -= 1;
            r.postings[1].amount.coefficient += 1;
        }
    }
    replay_refused(
        &state,
        &candidate(&state, records, &plan.invalidations, good.batch.clone()),
    );

    // Native operations that differ from the plan derived again. The adapter
    // itself refuses each one, and so does the whole replay.
    let native = |batch: AccountingBatchV1| {
        let batch = rebound(batch);
        assert!(
            AccountingAdapter::verify_correspondence(&good.commit, Some(&batch), index).is_err()
        );
        refused(
            &state,
            &candidate(&state, plan.records.clone(), &plan.invalidations, batch),
        );
    };
    let mut missing = good.batch.clone();
    missing.operations.remove(0);
    native(missing);
    let mut extra = good.batch.clone();
    let Some(FrozenNativeOperationV1::Entry(mut copy)) = extra.operations.last().cloned() else {
        panic!("backfill entry missing")
    };
    copy.key.push_str("/extra");
    extra.operations.push(FrozenNativeOperationV1::Entry(copy));
    native(extra);
    let mut reordered = good.batch.clone();
    reordered.operations.swap(1, 2);
    native(reordered);
}

#[test]
fn cur_016_replacement_refuses_cross_account_and_dependents() {
    // A backfill that is a transfer to a second external account.
    let mut state = anchored();
    let mut external = catalog();
    external.revision_id = id(62);
    external.catalog.account_id = account(60);
    external.catalog.component_id = id(61);
    external.ownership.account_id = account(60);
    let plan = CatalogEngine::set_complete(&state, &external).unwrap();
    apply(&mut state, &plan);
    let index = state.accounting.as_mut().unwrap();
    let mut mapping = index.accounts[&account(10)].clone();
    mapping.account_id = account(60);
    mapping.path = format!("Currents:A{}", id(60).0.simple());
    index.accounts.insert(account(60), mapping);
    let mut request = replacement(&mut state, 3400, "2026-10-30", &[(-100, "2026-10-31")]);
    let source = dated(
        &mut state,
        3300,
        100,
        "2026-10-31",
        ObservationKind::Movement,
    );
    let draft = &mut request.backfill[0];
    let mut leg = draft.legs[0].clone();
    leg.leg.id = LegId(id(3310).0);
    leg.leg.account_id = account(60);
    leg.leg.movement = amount(100);
    leg.leg.allocations = vec![EvidenceAllocationV1 {
        observation: source.observation,
        facet: source.facet,
        amount: amount(100),
    }];
    leg.sources = vec![source];
    draft.event.legs.push(leg.leg.id);
    draft.legs.push(leg);
    draft.financial.as_mut().unwrap().postings[1].account_id = account(60);
    request.approved_accounts = vec![account(10), account(EQUITY), account(60)];
    let before = snapshot(&state);
    assert!(SegmentEngine::replace(&state, &request).is_err());
    assert_eq!(snapshot(&state), before);

    // An organization record, review record or forecast run that names the
    // account, the old opening or one of its legs.
    let TemporalValue::Date { civil, .. } = date("2026-11-02") else {
        panic!("civil date")
    };
    let mut dependents = vec![];
    if let AuthoritativeRecordV1::Organization(mut organization) = fixture_record("Organization") {
        organization.id = id(3402);
        organization.targets = vec![id(OLD_OPENING)];
        dependents.push(AuthoritativeRecordV1::Organization(organization));
    }
    if let AuthoritativeRecordV1::Review(mut review) = fixture_record("Review") {
        review.id = id(3403);
        review.targets = vec![id(701)];
        dependents.push(AuthoritativeRecordV1::Review(review));
    }
    if let AuthoritativeRecordV1::ForecastRun(mut run) = fixture_record("ForecastRun") {
        run.id = id(3404);
        run.expanded_inputs
            .starting_positions
            .push(ForecastStartingPositionV1 {
                account: account(10),
                as_of: CivilDateV1(civil),
                amount: SourceValue::Absent,
                selected_basis: vec![],
                inclusion_map: vec![],
            });
        dependents.push(AuthoritativeRecordV1::ForecastRun(run));
    }
    assert_eq!(dependents.len(), 3);
    for dependent in dependents {
        let mut state = anchored();
        let request = replacement(&mut state, 3400, "2026-10-30", &[(-100, "2026-10-31")]);
        ready(&state, &request);
        let key = serde_json::from_value(
            serde_json::to_value(&dependent).unwrap()["value"]["id"].clone(),
        )
        .unwrap();
        state.records.insert(key, dependent);
        let before = snapshot(&state);
        assert!(SegmentEngine::replace(&state, &request).is_err());
        assert_eq!(snapshot(&state), before);
    }
}

// Cross-account closures. Account C (id 60) is external, account 80 is the
// clearing account and account 70 is a second external account. A sent 1,000
// USD to C on 2026-10-31; C's receipt on 2026-11-02 is recognized in transfer
// family 1650 with clearing open, because association admission refuses A's
// leg at or before A's cutoff.
const C: u32 = 60;
const OTHER: u32 = 70;
const CLEARING: u32 = 80;
const FAMILY: u32 = 1650;
const RECEIPT: u32 = 1610;

fn declare(state: &mut DomainState, n: u32, role: Option<ControlRoleV1>) {
    declare_as(state, n, AccountClass::Asset, NormalSign::Debit, role);
}
/// Declare account `n` of `class`, internal with `role` when there is one.
fn declare_as(
    state: &mut DomainState,
    n: u32,
    class: AccountClass,
    sign: NormalSign,
    role: Option<ControlRoleV1>,
) {
    let mut request = catalog();
    request.revision_id = id(n + 2);
    request.catalog.component_id = id(n + 1);
    request.catalog.account_id = account(n);
    request.catalog.account_class = class;
    request.catalog.normal_sign = sign;
    request.ownership.account_id = account(n);
    if let Some(role) = role {
        request.origin = AccountOriginV1::Internal(InternalAccountV1 {
            role,
            created_at: date("2026-01-01"),
        });
    }
    let plan = CatalogEngine::set_complete(state, &request).unwrap();
    apply(state, &plan);
    state.accounting.as_mut().unwrap().accounts.insert(
        account(n),
        NativeAccountMappingV1 {
            account_id: account(n),
            unit_id: UnitId(id(1).0),
            account_class: class,
            path: format!("Currents:A{}", id(n).0.simple()),
        },
    );
}
fn associated(state: &DomainState, request: &CompleteAssociationRequestV1) -> DomainState {
    let plan = IdentityEngine::associate(state, request).unwrap();
    admit(state, &planned(state, &plan)).unwrap()
}
fn with_transfer() -> DomainState {
    let mut state = anchored();
    for (n, role) in [
        (C, None),
        (OTHER, None),
        (CLEARING, Some(ControlRoleV1::Clearing)),
    ] {
        declare(&mut state, n, role);
    }
    let rows = transfers::cross_account_rows();
    let capture = association_fixtures::retained_relationships(&mut state, 1500, rows.clone());
    let both = transfers::clearing_family(capture, &rows, 1600, &[(10, 0), (C, 1)]);
    assert!(IdentityEngine::associate(&state, &both).is_err());
    associated(
        &state,
        &transfers::clearing_family(capture, &rows, 1600, &[(C, 1)]),
    )
}
/// A's earlier opening of `opening` USD at 2026-10-30 and its 2026-10-31
/// send of 1,000 USD to clearing, which joins C's transfer family.
fn cross_account(state: &mut DomainState, opening: i128) -> SegmentReplacementV1 {
    let mut request = replacement(state, opening, "2026-10-30", &[(-1000, "2026-10-31")]);
    request.backfill[0].financial.as_mut().unwrap().postings[1].account_id = account(CLEARING);
    request.approved_accounts = vec![account(10), account(EQUITY), account(CLEARING), account(C)];
    request.transfers = vec![TransferJoinV1 {
        revision: id(3500),
        prior_revision: id(FAMILY),
        leg: LegId(id(3111).0),
        source: transfers::row_fact(id(1501), "send-a"),
    }];
    request.reemissions = vec![transfers::reemission(state, RECEIPT, 3600)];
    request
}
/// Native postings on account `n` by value date, from active recognitions.
fn dated_postings(state: &DomainState, n: u32) -> Vec<(String, Amount)> {
    let index = state.accounting.as_ref().unwrap();
    let mut postings: Vec<_> = index
        .recognitions
        .iter()
        .filter(|(id, _)| !index.reversed.contains(id))
        .flat_map(|(_, entries)| entries)
        .flat_map(|e| {
            e.postings
                .iter()
                .filter(|p| p.original.account_id == account(n))
                .map(|p| (e.value_date.clone(), p.original.amount))
        })
        .collect();
    postings.sort_by(|a, b| a.0.cmp(&b.0));
    postings
}
fn revision(plan: &DecisionPlan) -> &AssociationRevisionV1 {
    let mut revisions = plan.records.iter().filter_map(|r| match r {
        AuthoritativeRecordV1::AssociationRevision(v) => Some(v),
        _ => None,
    });
    let revision = revisions.next().unwrap();
    assert!(revisions.next().is_none());
    revision
}

#[test]
fn cur_016_1_cross_account_backfill() {
    let mut state = with_transfer();
    let request = cross_account(&mut state, 4300);
    let plan = ready(&state, &request);
    let joined = revision(&plan);
    assert_eq!(joined.root, id(FAMILY));
    assert_eq!(joined.prior_revision, Some(id(FAMILY)));
    assert_eq!(joined.legs, [LegId(id(1601).0), LegId(id(3111).0)]);
    assert_eq!(joined.retired_recognitions, [recognition(RECEIPT)]);
    assert_eq!(joined.recognitions, [recognition(3610), recognition(3113)]);
    assert_eq!(joined.residuals, [amount(0)]);
    let edge = joined.captured_allocations.last().unwrap();
    assert_eq!(edge.source, transfers::row_fact(id(1501), "send-a"));
    assert_eq!(
        (edge.leg, edge.allocated),
        (LegId(id(3111).0), amount(-1000))
    );
    let segment = successor(&plan);
    assert_eq!(
        segment.replacement_closure,
        [recognition(OLD_OPENING), recognition(RECEIPT)]
    );
    assert!(segment.approved_accounts.contains(&account(C)));

    // The receipt is reversed once and issued again, after the replacement.
    let batch = AccountingAdapter::freeze(&plan.accounting, &state).unwrap();
    let entries: Vec<_> = batch
        .operations
        .iter()
        .map(|op| match op {
            FrozenNativeOperationV1::Entry(e) => (e.recognition_id, e.reverses.is_some()),
            other => panic!("unexpected native operation {other:?}"),
        })
        .collect();
    assert_eq!(
        entries,
        [
            (recognition(OLD_OPENING), true),
            (recognition(3013), false),
            (recognition(3113), false),
            (recognition(RECEIPT), true),
            (recognition(3610), false),
        ]
    );
    let replaced = admit(&state, &planned(&state, &plan)).unwrap();
    let index = replaced.accounting.as_ref().unwrap();
    let [original] =
        state.accounting.as_ref().unwrap().recognitions[&recognition(RECEIPT)].as_slice()
    else {
        panic!("one native receipt entry")
    };
    let [again] = index.recognitions[&recognition(3610)].as_slice() else {
        panic!("one native re-emission")
    };
    assert_eq!(
        (&again.booking_date, &again.value_date),
        (&original.booking_date, &original.value_date)
    );
    let economics = |e: &FrozenEntryV1| {
        e.postings
            .iter()
            .map(|p| {
                let mut posting = p.original.clone();
                posting.id = id(0);
                (posting, p.native_coefficient.clone(), p.path.clone())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(economics(again), economics(original));
    assert_eq!(dated_postings(&replaced, C), dated_postings(&state, C));
    assert_eq!(balances(&replaced)[&account(10)], amount(3300));
    assert_eq!(balances(&state)[&account(CLEARING)], amount(-1000));
    assert_eq!(balances(&replaced)[&account(CLEARING)], amount(0));
}

#[test]
fn cur_016_1_post_cutoff_activity_untouched() {
    let mut state = with_transfer();
    let source = dated(
        &mut state,
        1800,
        -50,
        "2026-11-03",
        ObservationKind::Movement,
    );
    let mut purchase = posted(source, 1810, EXPENSE);
    at(&mut purchase, "2026-11-03");
    let plan = RecognitionEngine::adopt(&state, &purchase).unwrap();
    let state = admit(&state, &planned(&state, &plan)).unwrap();
    let later = transfers::clearing_family(
        id(1501),
        &transfers::cross_account_rows(),
        1700,
        &[(10, 2), (C, 3)],
    );
    let mut state = associated(&state, &later);
    let kept = [recognition(1813), recognition(1710), recognition(1711)];

    let request = cross_account(&mut state, 4300);
    let replaced = admit(&state, &planned(&state, &ready(&state, &request))).unwrap();
    let index = replaced.accounting.as_ref().unwrap();
    for id in kept {
        assert!(index.recognitions.contains_key(&id) && !index.reversed.contains(&id));
        assert_eq!(
            replaced.records[&RecordId(id.0)],
            state.records[&RecordId(id.0)]
        );
    }

    // A closure that names the purchase or the later transfer is refused.
    for (original, base, extra) in [
        (1813, 3700, Some(EXPENSE)),
        (1710, 3800, None),
        (1711, 3900, None),
    ] {
        let mut named = request.clone();
        named
            .reemissions
            .push(transfers::reemission(&state, original, base));
        named.approved_accounts.extend(extra.map(account));
        let before = snapshot(&state);
        assert!(SegmentEngine::replace(&state, &named).is_err());
        assert_eq!(snapshot(&state), before);
    }
    // So is joining the backfilled send to the later transfer instead.
    let mut later_family = request.clone();
    later_family.transfers[0].prior_revision = id(1750);
    later_family.reemissions = vec![
        transfers::reemission(&state, 1710, 3700),
        transfers::reemission(&state, 1711, 3800),
    ];
    assert!(SegmentEngine::replace(&state, &later_family).is_err());
}

/// The movement binding that `dated` made for evidence `n`.
fn binding_of(state: &DomainState, n: u32) -> SourceFacetBindingV1 {
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) = state.records.get(&id(n + 1))
    else {
        panic!("parse product missing")
    };
    let SourceValue::Known(amount) = product.product.facets[0].amount else {
        panic!("amount missing")
    };
    SourceFacetBindingV1 {
        observation: id(n),
        parse_product: id(n + 1),
        facet: product.product.facets[0].lineage_key,
        amount,
    }
}
/// A replacement at 2026-10-30 whose new opening includes `included` and
/// whose backfill takes each of `backfill` on 2026-10-31.
fn moving(
    state: &mut DomainState,
    included: Vec<SourceFacetBindingV1>,
    backfill: &[&SourceFacetBindingV1],
) -> SegmentReplacementV1 {
    let mut request = replacement(state, 3360, "2026-10-30", &[]);
    let terms = request.opening.financial.as_mut().unwrap();
    terms.opening.as_mut().unwrap().included_activity = included;
    request.backfill = (0u32..)
        .zip(backfill)
        .map(|(i, source)| {
            let mut movement = posted((*source).clone(), 3110 + 20 * i, EXPENSE);
            at(&mut movement, "2026-10-31");
            movement
        })
        .collect();
    request.approved_accounts.push(account(EXPENSE));
    request
}

#[test]
fn cur_016_1_old_inclusion_map_moves() {
    // The old opening included a 40 USD fact on 2026-10-29 and a 60 USD fact
    // on 2026-10-31. A new opening at 2026-10-30 includes the first; the
    // second becomes a backfill.
    let mut state = anchored_with(&[(1900, -40, "2026-10-29"), (1920, -60, "2026-10-31")]);
    let early = binding_of(&state, 1900);
    let late = binding_of(&state, 1920);
    let request = moving(&mut state, vec![early.clone()], &[&late]);
    let plan = ready(&state, &request);
    let replaced = admit(&state, &planned(&state, &plan)).unwrap();
    assert_eq!(balances(&replaced)[&account(10)], amount(3300));

    let fresh = dated(
        &mut state,
        1940,
        -60,
        "2026-10-31",
        ObservationKind::Movement,
    );
    let cases = [
        ("drops the early fact", moving(&mut state, vec![], &[&late])),
        (
            "drops the late fact",
            moving(&mut state, vec![early.clone()], &[&fresh]),
        ),
        (
            "repeats the early fact",
            moving(&mut state, vec![early.clone(), early.clone()], &[&late]),
        ),
        (
            "repeats the late fact",
            moving(&mut state, vec![early.clone()], &[&late, &late]),
        ),
    ];
    for (name, request) in cases {
        let before = snapshot(&state);
        let result = SegmentEngine::replace(&state, &request);
        assert!(result.is_err(), "{name}: {result:?}");
        assert_eq!(snapshot(&state), before);
    }
}

#[test]
fn cur_016_1_partial_or_changed_reemission_refused() {
    let refuse = |name: &str, change: &dyn Fn(&DomainState, &mut SegmentReplacementV1)| {
        let mut state = with_transfer();
        let mut request = cross_account(&mut state, 4300);
        ready(&state, &request);
        change(&state, &mut request);
        let before = snapshot(&state);
        let result = SegmentEngine::replace(&state, &request);
        assert!(result.is_err(), "{name}: {result:?}");
        assert_eq!(snapshot(&state), before);
    };
    fn again(request: &mut SegmentReplacementV1) -> &mut RecognitionV1 {
        &mut request.reemissions[0].replacement
    }
    refuse("changed amount", &|_, request| {
        let postings = &mut again(request).postings;
        postings[0].amount.coefficient += 1;
        postings[1].amount.coefficient -= 1;
    });
    refuse("changed date", &|_, request| {
        for posting in &mut again(request).postings {
            posting.effective = date("2026-11-03");
        }
    });
    refuse("changed account", &|_, request| {
        again(request).postings[0].account_id = account(OTHER);
        request.approved_accounts.push(account(OTHER));
    });
    refuse("only some postings", &|_, request| {
        again(request).postings.pop();
    });
    refuse("approved accounts without C", &|_, request| {
        request.approved_accounts.retain(|a| *a != account(C));
    });
    refuse("the transfer named twice", &|state, request| {
        request
            .reemissions
            .push(transfers::reemission(state, RECEIPT, 3700));
    });
    refuse("the transfer left out", &|_, request| {
        request.reemissions.clear();
        request.approved_accounts.retain(|a| *a != account(C));
    });
    // The send's relationship evidence, not the request, places it in a family.
    refuse("evidence from another transfer", &|_, request| {
        request.transfers[0].source = transfers::row_fact(id(1501), "later-send-a");
    });
    refuse("the counterpart's own evidence", &|_, request| {
        request.transfers[0].source = transfers::row_fact(id(1501), "receipt-c");
    });

    // A family that holds only C's later receipt is not the send's family,
    // even though it uses the same clearing account.
    let state = with_transfer();
    let rows = transfers::cross_account_rows();
    let unrelated = transfers::clearing_family(id(1501), &rows, 1700, &[(C, 3)]);
    let mut state = associated(&state, &unrelated);
    let mut request = cross_account(&mut state, 4300);
    request.transfers[0].prior_revision = id(1750);
    request.reemissions = vec![transfers::reemission(&state, 1710, 3700)];
    let before = snapshot(&state);
    let result = SegmentEngine::replace(&state, &request);
    assert!(result.is_err(), "unrelated family: {result:?}");
    assert_eq!(snapshot(&state), before);
}

#[test]
fn cur_016_1_cross_account_residual_blocks() {
    let mut state = with_transfer();
    let request = cross_account(&mut state, 4290);
    let before = snapshot(&state);
    let balances_before = balances(&state);
    assert_eq!(
        SegmentEngine::replace(&state, &request).unwrap(),
        SegmentPreviewV1::UnresolvedResidual(amount(-10))
    );
    assert_eq!(snapshot(&state), before);
    assert_eq!(balances(&state), balances_before);
    assert_eq!(balances(&state)[&account(10)], amount(3300));
    assert_eq!(balances(&state)[&account(C)], amount(1000));
}

#[test]
fn cur_016_1_cross_account_replay() {
    let mut state = with_transfer();
    let request = cross_account(&mut state, 4300);
    let plan = ready(&state, &request);
    let good = planned(&state, &plan);
    let family_bytes = serde_json::to_vec(&state.records[&id(FAMILY)]).unwrap();
    let replaced = admit(&state, &good).unwrap();
    assert_eq!(
        serde_json::to_vec(&replaced.records[&id(FAMILY)]).unwrap(),
        family_bytes
    );
    IdentityEngine::audit(&replaced).unwrap();

    // Native history that reverses the receipt twice.
    let mut twice = good.batch.clone();
    let position = twice
        .operations
        .iter()
        .position(|op| {
            matches!(op, FrozenNativeOperationV1::Entry(e)
                if e.recognition_id == recognition(RECEIPT) && e.reverses.is_some())
        })
        .unwrap();
    let FrozenNativeOperationV1::Entry(mut repeat) = twice.operations[position].clone() else {
        panic!("reversal entry")
    };
    repeat.key.push_str("/again");
    twice
        .operations
        .insert(position + 1, FrozenNativeOperationV1::Entry(repeat));
    refused(
        &state,
        &candidate(
            &state,
            plan.records.clone(),
            &plan.invalidations,
            rebound(twice),
        ),
    );

    // Native history that issues the receipt to another counterpart.
    let mut other = good.batch.clone();
    for op in &mut other.operations {
        if let FrozenNativeOperationV1::Entry(e) = op
            && e.recognition_id == recognition(3610)
        {
            let posting = &mut e.postings[0];
            assert_eq!(posting.original.account_id, account(C));
            posting.original.account_id = account(OTHER);
            posting.path = format!("Currents:A{}", id(OTHER).0.simple());
        }
    }
    let other = rebound(other);
    let index = replaced.accounting.as_ref().unwrap();
    assert!(AccountingAdapter::verify_correspondence(&good.commit, Some(&other), index).is_err());
    refused(
        &state,
        &candidate(&state, plan.records.clone(), &plan.invalidations, other),
    );

    // Domain history whose new revision or re-emission link differs.
    let mut residual = plan.records.clone();
    for record in &mut residual {
        if let AuthoritativeRecordV1::AssociationRevision(r) = record {
            r.residuals = vec![amount(-1000)];
        }
    }
    let mut unlinked = plan.records.clone();
    unlinked.retain(|r| !matches!(r, AuthoritativeRecordV1::SegmentReemission(_)));
    for records in [residual, unlinked] {
        replay_refused(
            &state,
            &candidate(&state, records, &plan.invalidations, good.batch.clone()),
        );
    }
}

// Dependent records. On account A, claim K1 covers 2026-10-01 to 2026-10-31
// and K2 covers 2026-11-05 to 2026-11-30. On C, K3 covers November. Note N1
// targets the transfer's leg on C; note N2 targets A's old opening leg.
const K1: u32 = 4001;
const K2: u32 = 4002;
const K3: u32 = 4003;
const N1: u32 = 4011;
const N2: u32 = 4012;
const OLD_LEG: u32 = 701;
const C_LEG: u32 = 1601;

fn claims() -> Vec<AuthoritativeRecordV1> {
    [
        claim(K1, 10, "2026-10-01", "2026-10-31"),
        claim(K2, 10, "2026-11-05", "2026-11-30"),
        claim(K3, C, "2026-11-01", "2026-11-30"),
    ]
    .into_iter()
    .map(AuthoritativeRecordV1::ReconciliationClaim)
    .collect()
}

#[test]
fn cur_016_claims_outside_change_stay_current() {
    let mut state = with_transfer();
    let request = cross_account(&mut state, 4300);
    let state = recorded(&state, claims());
    let plan = ready(&state, &request);
    assert!(plan.invalidations.contains(&id(K1)));
    assert!(!plan.invalidations.contains(&id(K2)));
    assert!(!plan.invalidations.contains(&id(K3)));
    let replaced = admit(&state, &planned(&state, &plan)).unwrap();
    assert_eq!(
        replaced.claim_status(id(K1)),
        Some(ReconciliationStatus::Invalidated)
    );
    for kept in [K2, K3] {
        assert_eq!(
            replaced.claim_status(id(kept)),
            Some(ReconciliationStatus::Matched)
        );
        assert_eq!(replaced.records[&id(kept)], state.records[&id(kept)]);
    }
}

/// Account A without an anchor, so activity may sit in October.
fn unanchored() -> DomainState {
    let mut state = monetary_state();
    for mapping in state.accounting.as_mut().unwrap().accounts.values_mut() {
        mapping.path = format!("Currents:A{}", mapping.account_id.0.simple());
    }
    state
}
/// A standalone -25 USD purchase on A on `day`, recognition `n + 13`.
fn purchase(state: &DomainState, n: u32, day: &str) -> DomainState {
    let mut state = state.clone();
    let source = dated(&mut state, n, -25, day, ObservationKind::Movement);
    let mut request = posted(source, n + 10, EXPENSE);
    at(&mut request, day);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    admit(&state, &planned(&state, &plan)).unwrap()
}
/// Recognition `original` again with fresh identities from `base`.
fn identity_correction(
    state: &DomainState,
    original: u32,
    base: u32,
) -> currents_domain::correction::CorrectionRequestV1 {
    let again = transfers::reemission(state, original, base);
    currents_domain::correction::CorrectionRequestV1 {
        active_recognition: again.original,
        replacement: vec![again.replacement],
        remapping: vec![],
        reason: "Correct the posting identities only".into(),
    }
}

#[test]
fn cur_016_identity_correction_and_claims() {
    use currents_domain::correction::CorrectionEngine;
    let state = purchase(&unanchored(), 4100, "2026-10-15");
    let bought = recognition(4113);
    let mut named = claim(4004, EXPENSE, "2026-12-01", "2026-12-31");
    named.decision_dependencies = vec![RecordId(bought.0)];
    let state = recorded(
        &state,
        vec![
            AuthoritativeRecordV1::ReconciliationClaim(claim(K1, 10, "2026-10-01", "2026-10-31")),
            AuthoritativeRecordV1::ReconciliationClaim(named),
        ],
    );
    let request = identity_correction(&state, 4113, 4200);
    let plan = CorrectionEngine::correct_durable(&state, id(4300), &request).unwrap();
    assert!(!plan.invalidations.contains(&id(K1)));
    assert_eq!(plan.invalidations.last(), Some(&id(4004)));
    let good = planned(&state, &plan);
    let corrected = admit(&state, &good).unwrap();
    // Replay rejects the correction when its result omits or adds a claim.
    let omitted = &plan.invalidations[..plan.invalidations.len() - 1];
    let added: Vec<_> = plan.invalidations.iter().copied().chain([id(K1)]).collect();
    for invalidations in [omitted, &added] {
        let records = plan.records.clone();
        replay_refused(
            &state,
            &candidate(&state, records, invalidations, good.batch.clone()),
        );
    }
    assert!(
        corrected
            .accounting
            .as_ref()
            .unwrap()
            .reversed
            .contains(&bought)
    );
    assert_eq!(
        corrected.claim_status(id(K1)),
        Some(ReconciliationStatus::Matched)
    );
    assert_eq!(
        corrected.claim_status(id(4004)),
        Some(ReconciliationStatus::Invalidated)
    );
    assert_eq!(corrected.records[&id(4004)], state.records[&id(4004)]);
}

#[test]
fn cur_016_organization_on_stable_leg_survives() {
    let mut state = with_transfer();
    let request = cross_account(&mut state, 4300);
    let state = recorded(&state, vec![note(N1, id(C_LEG))]);
    let plan = ready(&state, &request);
    assert!(!plan.invalidations.contains(&id(N1)));
    let replaced = admit(&state, &planned(&state, &plan)).unwrap();
    assert!(!replaced.invalidated.contains(&id(N1)));
    assert_eq!(
        serde_json::to_vec(&replaced.records[&id(N1)]).unwrap(),
        serde_json::to_vec(&state.records[&id(N1)]).unwrap()
    );
    // The leg N1 targets is in the family's new revision.
    assert!(revision(&plan).legs.contains(&LegId(id(C_LEG).0)));
}

#[test]
fn cur_016_organization_on_retired_leg_asks_owner() {
    let mut state = with_transfer();
    let request = cross_account(&mut state, 4300);
    let state = recorded(&state, vec![note(N1, id(C_LEG)), note(N2, id(OLD_LEG))]);
    let before = snapshot(&state);
    assert_eq!(
        SegmentEngine::replace(&state, &request).unwrap(),
        SegmentPreviewV1::NeedsOrganizationDecision(vec![OrganizationQuestionV1 {
            record: id(N2),
            leg: LegId(id(OLD_LEG).0),
        }])
    );
    assert_eq!(snapshot(&state), before);
}

#[test]
fn cur_016_unsupported_dependents_still_refuse() {
    use currents_domain::correction::CorrectionEngine;
    let mut state = anchored();
    let request = replacement(&mut state, 3400, "2026-10-30", &[(-100, "2026-10-31")]);
    let state = purchase(&state, 4100, "2026-11-10");
    let correction = identity_correction(&state, 4113, 4200);
    ready(&state, &request);
    CorrectionEngine::correct_durable(&state, id(4300), &correction).unwrap();

    let AuthoritativeRecordV1::Review(mut review) = fixture_record("Review") else {
        panic!("review fixture")
    };
    review.id = id(4021);
    review.targets = vec![id(10)];
    let AuthoritativeRecordV1::ForecastRun(mut run) = fixture_record("ForecastRun") else {
        panic!("forecast fixture")
    };
    run.id = id(4022);
    run.expanded_inputs
        .starting_positions
        .push(ForecastStartingPositionV1 {
            account: account(10),
            as_of: CivilDateV1(civil("2026-11-30")),
            amount: SourceValue::Absent,
            selected_basis: vec![],
            inclusion_map: vec![],
        });
    for dependent in [
        AuthoritativeRecordV1::Review(review),
        AuthoritativeRecordV1::ForecastRun(run),
    ] {
        let state = recorded(&state, vec![dependent]);
        let before = snapshot(&state);
        assert!(SegmentEngine::replace(&state, &request).is_err());
        assert!(CorrectionEngine::correct_durable(&state, id(4300), &correction).is_err());
        assert_eq!(snapshot(&state), before);
    }
}

// Reinterpretation: the legs stay and only the internal postings change.
const UNRESOLVED: u32 = 40;
const WAGES: u32 = 45;
const EXPENSE_B: u32 = 35;
const CARD: u32 = 90;
const DEPOSIT: u32 = 8000;
const CARD_PURCHASE: u32 = 8050;

fn governing() -> PolicyReferenceV1 {
    PolicyReferenceV1 {
        id: id(104),
        version: 1,
    }
}
/// Unanchored checking (A) with wages income, a second expense account (B),
/// an external card account and a clearing account.
fn reinterpretable() -> DomainState {
    let mut state = unanchored();
    let income = Some(ControlRoleV1::IncomeFlow);
    declare_as(
        &mut state,
        WAGES,
        AccountClass::Income,
        NormalSign::Credit,
        income,
    );
    let expense = Some(ControlRoleV1::ExpenseFlow);
    declare_as(
        &mut state,
        EXPENSE_B,
        AccountClass::Expense,
        NormalSign::Debit,
        expense,
    );
    declare_as(
        &mut state,
        CARD,
        AccountClass::Liability,
        NormalSign::Credit,
        None,
    );
    declare(&mut state, CLEARING, Some(ControlRoleV1::Clearing));
    state
}
/// A 3,300 USD deposit to checking on 2026-11-02, adopted as debit checking,
/// credit unresolved equity. Its leg is `DEPOSIT + 11` and its recognition
/// `DEPOSIT + 13`.
fn deposited() -> DomainState {
    let mut state = reinterpretable();
    let source = dated(
        &mut state,
        DEPOSIT,
        3300,
        "2026-11-02",
        ObservationKind::Movement,
    );
    let mut request = posted(source, DEPOSIT + 10, UNRESOLVED);
    at(&mut request, "2026-11-02");
    request.financial.as_mut().unwrap().unresolved_account = Some(account(UNRESOLVED));
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    admit(&state, &planned(&state, &plan)).unwrap()
}
/// A 96 USD card purchase on 2026-11-03, adopted to expense account A.
/// Its recognition is `CARD_PURCHASE + 13`.
fn card_purchase() -> DomainState {
    let mut state = reinterpretable();
    let source = dated(
        &mut state,
        CARD_PURCHASE,
        -96,
        "2026-11-03",
        ObservationKind::Movement,
    );
    let mut request = posted(source, CARD_PURCHASE + 10, EXPENSE);
    at(&mut request, "2026-11-03");
    request.legs[0].leg.account_id = account(CARD);
    request.financial.as_mut().unwrap().postings[0].account_id = account(CARD);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    admit(&state, &planned(&state, &plan)).unwrap()
}
/// Recognition `original` again with fresh identities from `base`: its first
/// (external) posting as it was, then one posting per `(account, amount)`.
fn reinterpretation(
    state: &DomainState,
    original: u32,
    base: u32,
    counters: &[(u32, i128)],
) -> currents_domain::correction::CorrectionRequestV1 {
    let AuthoritativeRecordV1::Recognition(prior) = &state.records[&id(original)] else {
        panic!("recognition {original} missing")
    };
    let mut replacement = prior.clone();
    replacement.id = recognition(base);
    let template = replacement.postings[1].clone();
    replacement.postings.truncate(1);
    replacement
        .postings
        .extend(counters.iter().map(|&(on, value)| PostingV1 {
            account_id: account(on),
            amount: amount(value),
            ..template.clone()
        }));
    for (posting, n) in replacement.postings.iter_mut().zip(base + 1..) {
        posting.id = id(n);
    }
    currents_domain::correction::CorrectionRequestV1 {
        active_recognition: prior.id,
        replacement: vec![replacement],
        remapping: vec![],
        reason: "Reinterpret the activity; its legs stay".into(),
    }
}
fn reinterpret(
    state: &DomainState,
    revision: u32,
    request: &currents_domain::correction::CorrectionRequestV1,
    unresolved: Option<u32>,
) -> Result<DecisionPlan, EngineError> {
    currents_domain::correction::CorrectionEngine::reinterpret(
        state,
        id(revision),
        request,
        &governing(),
        unresolved.map(account),
    )
}
fn correction_revision(plan: &DecisionPlan) -> &RecognitionCorrectionRevisionV1 {
    let Some(AuthoritativeRecordV1::RecognitionCorrectionRevision(revision)) = plan.records.first()
    else {
        panic!("the plan starts with its revision")
    };
    revision
}
/// The native balance of account `n`, zero when nothing posts to it.
fn balance(state: &DomainState, n: u32) -> Amount {
    balances(state)
        .get(&account(n))
        .copied()
        .unwrap_or(amount(0))
        .normalized()
        .unwrap()
}

#[test]
fn cur_016_reinterpret_deposit_as_income() {
    let state = deposited();
    let original = recognition(DEPOSIT + 13);
    let request = reinterpretation(&state, DEPOSIT + 13, 8100, &[(WAGES, -3300)]);
    let plan = reinterpret(&state, 8200, &request, None).unwrap();
    assert_eq!(
        correction_revision(&plan),
        &RecognitionCorrectionRevisionV1 {
            id: id(8200),
            schema_version: 2,
            prior_recognition: original,
            replacement_recognition: recognition(8100),
            reason: request.reason.clone(),
            remap_disposition: RecognitionCorrectionRemapV1::ReinterpretedStableLegs(
                StableLegsReinterpretationV1 {
                    governing_policy: governing(),
                    unresolved_account: None,
                    changed_accounts: vec![account(UNRESOLVED), account(WAGES)],
                }
            ),
        }
    );
    let good = planned(&state, &plan);
    let [
        FrozenNativeOperationV1::Entry(reversal),
        FrozenNativeOperationV1::Entry(added),
    ] = good.batch.operations.as_slice()
    else {
        panic!("one reversal, then one replacement")
    };
    let [frozen] = state.accounting.as_ref().unwrap().recognitions[&original].as_slice() else {
        panic!("one native entry")
    };
    assert_eq!(reversal.recognition_id, original);
    assert_eq!(reversal.reverses.as_ref(), Some(&frozen.key));
    for entry in [reversal, added] {
        assert_eq!(entry.value_date, "2026-11-02");
        assert_eq!(entry.booking_date, "2026-11-02");
    }
    let ids = |entry: &FrozenEntryV1| -> Vec<RecordId> {
        entry.postings.iter().map(|p| p.original.id).collect()
    };
    assert_eq!(ids(reversal), ids(frozen));
    let moves = |entry: &FrozenEntryV1| -> Vec<(AccountId, Amount)> {
        entry
            .postings
            .iter()
            .map(|p| (p.original.account_id, p.original.amount))
            .collect()
    };
    assert_eq!(
        moves(added),
        vec![(account(10), amount(3300)), (account(WAGES), amount(-3300))]
    );

    let corrected = admit(&state, &good).unwrap();
    assert_eq!(dated_postings(&corrected, 10), dated_postings(&state, 10));
    // Credit balances: unresolved equity falls by 3,300 and wages rises by 3,300.
    assert_eq!(balance(&state, UNRESOLVED), amount(-3300));
    assert_eq!(balance(&corrected, UNRESOLVED), amount(0));
    assert_eq!(balance(&state, WAGES), amount(0));
    assert_eq!(balance(&corrected, WAGES), amount(-3300));
    // The earlier prefix still shows the unresolved-equity recognition as
    // active, and the old adoption, leg and recognition stay byte-identical.
    assert!(
        !state
            .accounting
            .as_ref()
            .unwrap()
            .reversed
            .contains(&original)
    );
    assert!(
        corrected
            .accounting
            .as_ref()
            .unwrap()
            .reversed
            .contains(&original)
    );
    for key in [id(DEPOSIT + 10), id(DEPOSIT + 11), id(DEPOSIT + 13)] {
        assert_eq!(
            serde_json::to_vec(&corrected.records[&key]).unwrap(),
            serde_json::to_vec(&state.records[&key]).unwrap()
        );
    }

    // Replay derives the revision again and refuses one that differs.
    let forged = |change: fn(&mut StableLegsReinterpretationV1)| {
        let mut records = plan.records.clone();
        let AuthoritativeRecordV1::RecognitionCorrectionRevision(revision) = &mut records[0] else {
            panic!("revision first")
        };
        let RecognitionCorrectionRemapV1::ReinterpretedStableLegs(interpretation) =
            &mut revision.remap_disposition
        else {
            panic!("reinterpreted")
        };
        change(interpretation);
        candidate(&state, records, &plan.invalidations, good.batch.clone())
    };
    replay_refused(&state, &forged(|i| i.changed_accounts.truncate(1)));
    replay_refused(
        &state,
        &forged(|i| i.unresolved_account = Some(account(UNRESOLVED))),
    );
    replay_refused(&state, &forged(|i| i.governing_policy.version = 2));
}

#[test]
fn cur_016_reinterpret_split_purchase() {
    let state = card_purchase();
    let request = reinterpretation(
        &state,
        CARD_PURCHASE + 13,
        8300,
        &[(EXPENSE, 60), (EXPENSE_B, 36)],
    );
    let plan = reinterpret(&state, 8400, &request, None).unwrap();
    let RecognitionCorrectionRemapV1::ReinterpretedStableLegs(interpretation) =
        &correction_revision(&plan).remap_disposition
    else {
        panic!("reinterpreted")
    };
    assert_eq!(
        interpretation.changed_accounts,
        vec![account(EXPENSE), account(EXPENSE_B)]
    );
    let corrected = admit(&state, &planned(&state, &plan)).unwrap();
    assert_eq!(balance(&corrected, CARD), amount(-96));
    assert_eq!(balance(&corrected, EXPENSE), amount(60));
    assert_eq!(balance(&corrected, EXPENSE_B), amount(36));
}

#[test]
fn cur_016_reinterpret_refuses_leg_changes() {
    let state = card_purchase();
    let request = reinterpretation(
        &state,
        CARD_PURCHASE + 13,
        8300,
        &[(EXPENSE, 60), (EXPENSE_B, 36)],
    );
    reinterpret(&state, 8400, &request, None).unwrap();
    let before = snapshot(&state);
    type Change = Box<dyn Fn(&mut RecognitionV1)>;
    let changes: [(&str, Change); 8] = [
        (
            "card amount",
            Box::new(|r| {
                r.postings[0].amount = amount(-90);
                r.postings[1].amount = amount(54);
            }),
        ),
        (
            "card account",
            Box::new(|r| r.postings[0].account_id = account(10)),
        ),
        (
            "effective date",
            Box::new(|r| {
                for p in &mut r.postings {
                    p.effective = date("2026-11-04");
                }
                r.scope.start = Some(civil("2026-11-04"));
                r.scope.end = Some(civil("2026-11-04"));
            }),
        ),
        (
            "posting date",
            Box::new(|r| {
                for p in &mut r.postings {
                    p.posting = date("2026-11-04");
                }
            }),
        ),
        (
            "unbalanced",
            Box::new(|r| r.postings[2].amount = amount(35)),
        ),
        (
            "opening equity",
            Box::new(|r| r.postings[2].account_id = account(EQUITY)),
        ),
        (
            "clearing",
            Box::new(|r| r.postings[2].account_id = account(CLEARING)),
        ),
        (
            "unnamed unresolved equity",
            Box::new(|r| r.postings[2].account_id = account(UNRESOLVED)),
        ),
    ];
    for (name, change) in &changes {
        let mut changed = request.clone();
        change(&mut changed.replacement[0]);
        assert!(
            reinterpret(&state, 8400, &changed, None).is_err(),
            "{name} must be refused"
        );
    }
    // Naming the unresolved account it posts to is a valid interpretation.
    let mut unresolved = request.clone();
    (changes[7].1)(&mut unresolved.replacement[0]);
    reinterpret(&state, 8400, &unresolved, Some(UNRESOLVED)).unwrap();
    // The governing policy must be an exact retained version.
    for version in [0, 2] {
        let policy = PolicyReferenceV1 {
            id: id(104),
            version,
        };
        assert!(
            currents_domain::correction::CorrectionEngine::reinterpret(
                &state,
                id(8400),
                &request,
                &policy,
                None
            )
            .is_err()
        );
    }
    assert_eq!(snapshot(&state), before);
}

#[test]
fn cur_016_reinterpretation_invalidates_claims_in_scope() {
    let claims = [
        claim(8501, WAGES, "2026-11-01", "2026-11-30"),
        claim(8502, 10, "2026-11-01", "2026-11-30"),
    ];
    let state = recorded(
        &deposited(),
        claims
            .into_iter()
            .map(AuthoritativeRecordV1::ReconciliationClaim)
            .collect(),
    );
    let request = reinterpretation(&state, DEPOSIT + 13, 8100, &[(WAGES, -3300)]);
    let plan = reinterpret(&state, 8200, &request, None).unwrap();
    assert!(plan.invalidations.contains(&id(8501)));
    assert!(!plan.invalidations.contains(&id(8502)));
    let corrected = admit(&state, &planned(&state, &plan)).unwrap();
    assert_eq!(
        corrected.claim_status(id(8501)),
        Some(ReconciliationStatus::Invalidated)
    );
    assert_eq!(
        corrected.claim_status(id(8502)),
        Some(ReconciliationStatus::Matched)
    );
}

#[test]
fn cur_016_reinterpretation_keeps_organization() {
    let leg = id(DEPOSIT + 11);
    let state = recorded(&deposited(), vec![note(8601, leg)]);
    let request = reinterpretation(&state, DEPOSIT + 13, 8100, &[(WAGES, -3300)]);
    let plan = reinterpret(&state, 8200, &request, None).unwrap();
    assert!(!plan.invalidations.contains(&id(8601)));
    let corrected = admit(&state, &planned(&state, &plan)).unwrap();
    assert!(!corrected.invalidated.contains(&id(8601)));
    for key in [id(8601), leg] {
        assert_eq!(
            serde_json::to_vec(&corrected.records[&key]).unwrap(),
            serde_json::to_vec(&state.records[&key]).unwrap()
        );
    }
}

/// A replacement of the anchored opening with `count` backfill movements of
/// -1 USD on 2026-10-31 and a new opening at 2026-10-30 that reconciles them.
fn many_backfills(state: &mut DomainState, count: usize) -> SegmentReplacementV1 {
    let opening = 3300 + i128::try_from(count).unwrap();
    replacement_from(
        state,
        100_000,
        opening,
        "2026-10-30",
        &vec![(-1, "2026-10-31"); count],
    )
}

#[test]
fn cur_016_4_20001_operations_fail_visibly() {
    // The old opening's reversal, the new opening and 19,999 backfills.
    let mut state = anchored();
    let request = many_backfills(&mut state, 19_999);
    let before = snapshot(&state);
    let heads = state.accounting.as_ref().unwrap().prefix.clone();
    assert_eq!(
        SegmentEngine::replace(&state, &request).unwrap(),
        SegmentPreviewV1::TooLarge {
            operations: 20_001,
            limit: 20_000,
        }
    );
    assert_eq!(snapshot(&state), before);
    assert_eq!(state.accounting.as_ref().unwrap().prefix, heads);

    // One movement fewer fits the closure limit, so the preview goes on to
    // check the drafts and refuses the first one, moved to another account.
    let mut fits = request;
    fits.backfill.pop();
    fits.backfill[0].legs[0].leg.account_id = account(EQUITY);
    assert_eq!(
        SegmentEngine::replace(&state, &fits).unwrap_err().class,
        ErrorClass::UnsupportedVersion
    );
}
