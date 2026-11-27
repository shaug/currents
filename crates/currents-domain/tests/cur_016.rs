//! CUR-016 segment replacement through the public preview, replay and C05 adapter.
//! Synthetic account A (id 10) is anchored at 3,300 USD at the end of 2026-11-01.
use currents_accounting::AccountingAdapter;
use currents_domain::{
    ReplayEngine,
    associations::IdentityEngine,
    catalog::CatalogEngine,
    recognition::RecognitionEngine,
    segments::{SegmentEngine, SegmentPreviewV1, SegmentReplacementV1},
};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;

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

/// One proposed commit with the exact frozen operations its manifest names.
#[derive(Clone)]
struct Candidate {
    commit: DomainCommitV1,
    bytes: Vec<u8>,
    batch: AccountingBatchV1,
}
fn candidate(
    state: &DomainState,
    mut records: Vec<AuthoritativeRecordV1>,
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
                invalidations: vec![],
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
    candidate(state, plan.records.clone(), batch)
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

fn anchored() -> DomainState {
    let mut state = monetary_state();
    for mapping in state.accounting.as_mut().unwrap().accounts.values_mut() {
        mapping.path = format!("Currents:A{}", mapping.account_id.0.simple());
    }
    let mut opening = supported_opening(&mut state);
    let terms = opening.financial.as_mut().unwrap();
    terms.opening.as_mut().unwrap().included_activity.clear();
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
    replay_refused(&state, &candidate(&state, records, first.batch.clone()));

    // A commit that would leave two active segments for account A.
    let mut records = plan.records.clone();
    let mut second = successor(&plan).clone();
    second.id = id(3999);
    second.prior_segment = None;
    records.push(AuthoritativeRecordV1::AccountingSegment(second));
    replay_refused(&state, &candidate(&state, records, first.batch.clone()));
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
    replay_refused(&state, &candidate(&state, records, good.batch.clone()));

    // Native operations that differ from the plan derived again. The adapter
    // itself refuses each one, and so does the whole replay.
    let native = |mut batch: AccountingBatchV1| {
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
        assert!(
            AccountingAdapter::verify_correspondence(&good.commit, Some(&batch), index).is_err()
        );
        refused(&state, &candidate(&state, plan.records.clone(), batch));
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

    // A claim, organization record, review record or forecast run that names
    // the account, the old opening or one of its legs.
    let fixtures: Vec<AuthoritativeRecordV1> =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    let find = |kind: &str| {
        fixtures
            .iter()
            .find(|r| serde_json::to_value(r).unwrap()["kind"] == kind)
            .unwrap()
            .clone()
    };
    let TemporalValue::Date { civil, .. } = date("2026-11-02") else {
        panic!("civil date")
    };
    let mut dependents = vec![];
    if let AuthoritativeRecordV1::ReconciliationClaim(mut claim) = find("ReconciliationClaim") {
        claim.id = id(3401);
        claim.scope = vec![account(10)];
        dependents.push(AuthoritativeRecordV1::ReconciliationClaim(claim));
    }
    if let AuthoritativeRecordV1::Organization(mut organization) = find("Organization") {
        organization.id = id(3402);
        organization.targets = vec![id(OLD_OPENING)];
        dependents.push(AuthoritativeRecordV1::Organization(organization));
    }
    if let AuthoritativeRecordV1::Review(mut review) = find("Review") {
        review.id = id(3403);
        review.targets = vec![id(701)];
        dependents.push(AuthoritativeRecordV1::Review(review));
    }
    if let AuthoritativeRecordV1::ForecastRun(mut run) = find("ForecastRun") {
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
    assert_eq!(dependents.len(), 4);
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
