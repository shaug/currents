//! Reconciliation claims committed as their owner records them: one evaluated
//! claim and one `Reconciliation` decision per commit, with no accounting
//! effect. Replay derives each claim again from the prior history.
use super::{fixture, support};
use currents_domain::reconciliation::ReconciliationEngine;
use currents_storage::{CommitControl, PreparedCommit};
use currents_types::*;
use std::sync::Arc;

/// The Activity request on `account` from `start` through `end` by
/// effective date, read at `state`, naming no evidence.
pub(super) fn request(
    state: &DomainState,
    account: AccountId,
    start: &str,
    end: &str,
) -> ReconciliationRequestV1 {
    let civil = |day: &str| {
        let TemporalValue::Date { civil, .. } = fixture::date(day) else {
            panic!("civil date")
        };
        Some(civil)
    };
    let fixtures: Vec<AuthoritativeRecordV1> = serde_json::from_str(include_str!(
        "../../../../fixtures/contracts/v1/records.json"
    ))
    .unwrap();
    let Some(AuthoritativeRecordV1::ReconciliationClaim(claim)) = fixtures
        .into_iter()
        .find(|r| matches!(r, AuthoritativeRecordV1::ReconciliationClaim(_)))
    else {
        panic!("claim fixture missing");
    };
    let mut basis = claim.basis;
    basis.dataset_id = DatasetId(support::id(1000).0);
    basis.through_commit = state.through;
    ReconciliationRequestV1 {
        kind: ReconciliationKindV1::Activity,
        scope: AccountScope {
            accounts: vec![account],
            components: vec![],
        },
        cutoffs: DateScope {
            start: civil(start),
            end: civil(end),
            basis: DateBasis::Effective,
        },
        evidence: vec![],
        exclusions: vec![],
        basis,
    }
}

/// The claim `request` evaluates at `state` and the commit input that
/// records it under command `command`.
pub(super) fn recorded(
    state: &DomainState,
    request: &ReconciliationRequestV1,
    command: u32,
) -> (ReconciliationClaimV1, PreparedCommit) {
    let plan = ReconciliationEngine::record(state, request).unwrap();
    let [AuthoritativeRecordV1::ReconciliationClaim(claim)] = plan.records.as_slice() else {
        panic!("one claim");
    };
    let claim = claim.clone();
    let command_id = CommandId(fixture::id(command).0);
    let digest = Digest([71; 32]);
    let seq = CommitSeq(state.through.0 + 1);
    let policy = state.dataset_policies.values().next().unwrap();
    let decision = DecisionEnvelopeV1 {
        id: fixture::id(command + 2),
        commit: seq,
        // After the claim and the frozen manifest.
        item_position: 2,
        decision_type: DecisionKind::Reconciliation,
        schema_version: 1,
        actor: fixture::id(command + 3),
        authority: AuthorityKind::Owner,
        grant_id: None,
        command_id,
        command_digest: digest,
        references: vec![],
        recorded_time: serde_json::from_str("\"2026-12-01T00:00:00Z\"").unwrap(),
        adopted_result: vec![claim.id],
        policy: PolicyReferenceV1 {
            id: policy.id,
            version: policy.version,
        },
        corrections: vec![],
        supersedes: vec![],
    };
    let prepared = PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id,
            digest,
            state: CommandState::Committed,
            commit: None,
            effects: EffectsV1 {
                records: vec![claim.id],
                accounts: vec![],
                amounts: vec![],
                invalidations: vec![],
            },
            health: vec![],
        },
        frozen_batch: currents_accounting::AccountingAdapter::freeze(&plan.accounting, state)
            .unwrap(),
        decision_plan: plan,
        prior_head: state.through,
        decisions: vec![decision],
        manifest_id: fixture::id(command + 1),
        control: Arc::new(CommitControl::default()),
    };
    (claim, prepared)
}
