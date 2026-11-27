//! CUR-017 reconciliation claims through the encrypted writer and replay.
use currents_accounting::AccountingAdapter;
use currents_domain::correction::{CorrectionEngine, CorrectionRequestV1};
use currents_protocol::OperationV1;
use currents_service as service_api;
use currents_storage::{CommitControl, CommitFailure, PreparedCommit};
use currents_types::*;
use std::path::Path;
use std::sync::Arc;

#[path = "../../currents-domain/tests/support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "support/lifecycle.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/financial_service.rs"]
#[allow(dead_code)]
mod support_financial;
use support_financial::*;

const ACCOUNT_A: u32 = 10;
const EXPENSE: u32 = 30;
const DIGEST: Digest = Digest([71; 32]);

/// Account A holds a 96 USD purchase recognized on 2026-10-15 as
/// recognition R, which this returns.
async fn purchase_on_a(path: &Path) -> (support::Fixture, DomainState, RecognitionId) {
    let (mut f, _) = bootstrap(path).await;
    declare_template(&mut f, &fixture::monetary_state()).await;
    publish_legacy(&mut f, 200, "1", "2026-10-15").await;
    let mut request = fixture::posted(legacy_binding(200), 300, EXPENSE);
    let day = fixture::date("2026-10-15");
    for item in &mut request.legs {
        item.leg.effective = day.clone();
        item.leg.posting = day.clone();
    }
    let terms = request.financial.as_mut().unwrap();
    terms.bookkeeping_date = day.clone();
    for posting in &mut terms.postings {
        posting.effective = day.clone();
        posting.posting = day.clone();
    }
    let recognition = terms.recognition_id;
    submit(&mut f, 55_001, OperationV1::AdoptActivity(request)).await;
    let state = refresh_financial(&mut f, None).await;
    (f, state, recognition)
}

/// Claim `n`, `Proposed` on A over October by effective date, read at
/// `state`, with empty dependency lists.
fn claim(state: &DomainState, n: u32) -> ReconciliationClaimV1 {
    let fixtures: Vec<AuthoritativeRecordV1> =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    let Some(AuthoritativeRecordV1::ReconciliationClaim(mut claim)) = fixtures
        .into_iter()
        .find(|r| matches!(r, AuthoritativeRecordV1::ReconciliationClaim(_)))
    else {
        panic!("claim fixture missing");
    };
    let civil = |day: &str| {
        let TemporalValue::Date { civil, .. } = fixture::date(day) else {
            panic!("civil date")
        };
        Some(civil)
    };
    claim.id = fixture::id(n);
    claim.kind = ReconciliationKindV1::Activity;
    claim.basis.dataset_id = DatasetId(support::id(1000).0);
    claim.basis.through_commit = state.through;
    claim.scope = vec![AccountId(fixture::id(ACCOUNT_A).0)];
    claim.cutoffs = DateScope {
        start: civil("2026-10-01"),
        end: civil("2026-10-31"),
        basis: DateBasis::Effective,
    };
    claim.status = ReconciliationStatus::Proposed;
    claim
}

/// The plan that records `claims` with no accounting effect, as their owner
/// will.
fn claims_plan(claims: Vec<ReconciliationClaimV1>) -> DecisionPlan {
    DecisionPlan {
        records: claims
            .into_iter()
            .map(AuthoritativeRecordV1::ReconciliationClaim)
            .collect(),
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "Synthetic reconciliation claims".into(),
        }),
    }
}

/// The commit input for `decision_plan` at `state`.
fn prepared(state: &DomainState, decision_plan: DecisionPlan, command: u32) -> PreparedCommit {
    let records = decision_plan
        .records
        .iter()
        .map(|r| {
            serde_json::from_value(serde_json::to_value(r).unwrap()["value"]["id"].clone()).unwrap()
        })
        .collect();
    PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id: CommandId(fixture::id(command).0),
            digest: DIGEST,
            state: CommandState::Committed,
            commit: None,
            effects: EffectsV1 {
                records,
                accounts: vec![],
                amounts: vec![],
                invalidations: decision_plan.invalidations.clone(),
            },
            health: vec![],
        },
        frozen_batch: AccountingAdapter::freeze(&decision_plan.accounting, state).unwrap(),
        decision_plan,
        prior_head: state.through,
        decisions: vec![],
        manifest_id: fixture::id(command + 1),
        control: Arc::new(CommitControl::default()),
    }
}

#[tokio::test]
async fn cur_017_storage_checks_dependency_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dependency-kinds.db");
    let (f, state, recognition) = purchase_on_a(&path).await;
    f.service.close().await.unwrap();
    let r = RecordId(recognition.0);
    let observation = legacy_binding(200).observation;

    let mut as_evidence = claim(&state, 4001);
    as_evidence.evidence_dependencies = vec![r];
    let mut as_decision = claim(&state, 4002);
    as_decision.decision_dependencies = vec![r];
    let mut observation_as_accounting = claim(&state, 4003);
    observation_as_accounting.accounting_dependencies = vec![observation];
    for (n, refused) in [
        (52_000, as_evidence),
        (52_010, as_decision),
        (52_020, observation_as_accounting),
    ] {
        let store = open(&path).await;
        let before = store.writer().audited_prefix().await.unwrap();
        let outcome = store
            .writer()
            .commit(prepared(&state, claims_plan(vec![refused]), n))
            .await;
        assert!(matches!(
            outcome,
            Err(CommitFailure::DefinitelyRolledBack(_))
        ));
        assert_eq!(store.writer().audited_prefix().await.unwrap(), before);
        assert_eq!(before.through, state.through);
        store.close().await.unwrap();
    }

    let mut named = claim(&state, 4004);
    named.accounting_dependencies = vec![r];
    let store = open(&path).await;
    store
        .writer()
        .commit(prepared(&state, claims_plan(vec![named]), 52_030))
        .await
        .expect("a claim that names R in its accounting dependencies commits");
    let after = store.writer().audited_prefix().await.unwrap();
    assert_eq!(after.through, CommitSeq(state.through.0 + 1));
    store.close().await.unwrap();
}

/// Replays `source` through `through` into the empty `destination`.
async fn replay_through(
    source: &currents_storage::HistoryReader,
    through: CommitSeq,
    destination: &currents_storage::EncryptedStore,
) -> DomainState {
    let empty = destination.writer().audited_prefix().await.unwrap();
    let mut state = DomainState {
        accounting: Some(AccountingIndexV1::empty(empty.native_head)),
        ..DomainState::default()
    };
    let mut stream = source.stream_prefix(through).await.unwrap();
    while let Some(commit) = stream.next().await.unwrap() {
        let mut next = state.clone();
        currents_domain::ReplayEngine::apply(&mut next, &commit).unwrap();
        next.accounting = Some(
            source
                .rebuild_commit(destination, &state, &commit)
                .await
                .unwrap(),
        );
        state = next;
    }
    assert_eq!(state.through, through);
    state
}

#[tokio::test]
async fn cur_017_correction_invalidates_named_recognition() {
    use currents_service::synthetic_commit_verified_correction;
    const NAMED: u32 = 4001;
    const DATED: u32 = 4002;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("named-recognition.db");
    let (mut f, state, recognition) = purchase_on_a(&path).await;
    f.service.close().await.unwrap();

    let mut named = claim(&state, NAMED);
    named.accounting_dependencies = vec![RecordId(recognition.0)];
    let store = open(&path).await;
    store
        .writer()
        .commit(prepared(
            &state,
            claims_plan(vec![named, claim(&state, DATED)]),
            52_000,
        ))
        .await
        .unwrap();
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let claimed = refresh_financial(&mut f, None).await;
    f.service.close().await.unwrap();

    // An identity-only correction of R: the same purchase with fresh IDs.
    let AuthoritativeRecordV1::Recognition(original) = &claimed.records[&RecordId(recognition.0)]
    else {
        panic!("R is retained");
    };
    let mut replacement = original.clone();
    replacement.id = RecognitionId(fixture::id(9400).0);
    for (ordinal, posting) in replacement.postings.iter_mut().enumerate() {
        posting.id = fixture::id(9401 + u32::try_from(ordinal).unwrap());
    }
    let request = CorrectionRequestV1 {
        active_recognition: recognition,
        replacement: vec![replacement],
        remapping: vec![],
        reason: "Correct the posting identities only".into(),
    };
    let revision = fixture::id(55_010);
    let plan = CorrectionEngine::correct_durable(&claimed, revision, &request).unwrap();
    assert_eq!(plan.invalidations.last(), Some(&fixture::id(NAMED)));
    assert!(!plan.invalidations.contains(&fixture::id(DATED)));
    let store = open(&path).await;
    let receipt =
        synthetic_commit_verified_correction(store.writer(), prepared(&claimed, plan, 55_011))
            .await
            .expect("the identity-only correction commits");

    let witness = open(&dir.path().join("replayed.db")).await;
    let replayed = replay_through(&store.history(), receipt.commit, &witness).await;
    witness.close().await.unwrap();
    store.close().await.unwrap();
    assert!(replayed.claim_invalidated(fixture::id(NAMED)));
    assert!(!replayed.claim_invalidated(fixture::id(DATED)));
    for n in [NAMED, DATED] {
        let key = fixture::id(n);
        assert!(!claimed.claim_invalidated(key));
        assert_eq!(
            replayed.claim_status(key),
            Some(ReconciliationStatus::Proposed)
        );
        assert_eq!(
            serde_json::to_vec(&replayed.records[&key]).unwrap(),
            serde_json::to_vec(&claimed.records[&key]).unwrap()
        );
    }
}
