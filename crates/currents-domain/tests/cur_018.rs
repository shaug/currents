//! CUR-018 effective-dated dataset policy revisions, using synthetic
//! identities and exact public interfaces.
use currents_domain::{ReplayEngine, catalog::CatalogEngine, policy::PolicyEngine};
use currents_types::*;
use serde_json::json;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;

/// Policy family P, and a second family Q.
const P: u32 = 700;
const Q: u32 = 701;
/// Connections S1 and S2.
const S1: u32 = 801;
const S2: u32 = 802;
/// Accounts A (checking) and B (savings), each in USD.
const A: u32 = 810;
const B: u32 = 820;
/// A declared analytical unit, which is not a currency.
const POINTS: u32 = 731;

fn account(n: u32) -> AccountId {
    AccountId(id(n).0)
}

fn day(civil: &str) -> Date {
    scope(Some(civil), None).start.unwrap()
}

fn scope(start: Option<&str>, end: Option<&str>) -> DateScope {
    serde_json::from_value(json!({"start":start,"end":end,"basis":"Effective"})).unwrap()
}

fn basis(state: &DomainState) -> ReadBasis {
    serde_json::from_value(json!({
        "dataset_id": id(100),
        "through_commit": state.through,
        "effective_scope": {"start":null,"end":null,"basis":"Effective"},
        "evaluation_instant": "2026-11-27T00:00:00Z",
        "query_semantics_version": 1,
        "selection_policy_version": 1,
        "unit_catalog_version": 1
    }))
    .unwrap()
}

fn change<T>(state: &DomainState, value: SourceValue<T>) -> Option<PolicyChangeV1<T>> {
    Some(PolicyChangeV1 {
        value,
        basis: basis(state),
    })
}

/// A request for family P that changes no field.
fn request(prior_version: Option<u32>, effective_scope: DateScope) -> SetDatasetPolicyV1 {
    SetDatasetPolicyV1 {
        policy_id: id(P),
        prior_version,
        effective_scope,
        ownership_perimeter: None,
        source_priority: None,
        freshness_thresholds: None,
        timezone: None,
        reporting_currency: None,
    }
}

fn priority(
    state: &DomainState,
    prior: Option<u32>,
    s: DateScope,
    order: [u32; 2],
) -> SetDatasetPolicyV1 {
    let mut r = request(prior, s);
    r.source_priority = change(state, SourceValue::Known(order.map(id).to_vec()));
    r
}

fn share(n: u32, numerator: u64, denominator: u64) -> OwnershipV1 {
    OwnershipV1 {
        account_id: account(n),
        numerator: CommitSeq(numerator),
        denominator: CommitSeq(denominator),
    }
}

fn threshold(source: u32, seconds: u64) -> FreshnessThresholdV1 {
    FreshnessThresholdV1 {
        source: id(source),
        seconds: CommitSeq(seconds),
    }
}

/// The committed commit after `state` that holds `records` and one owner
/// `Policy` decision.
fn commit(state: &DomainState, records: Vec<AuthoritativeRecordV1>) -> DomainCommitV1 {
    let seq = CommitSeq(state.through.0 + 1);
    let n = u32::try_from(seq.0).unwrap();
    let command_id = CommandId(id(9000 + n).0);
    let command_digest = Digest([88; 32]);
    let adopted: Vec<RecordId> = records
        .iter()
        .filter_map(|record| match record {
            AuthoritativeRecordV1::DatasetPolicyRevision(r) => Some(r.id),
            _ => None,
        })
        .collect();
    let decisions = if adopted.is_empty() {
        vec![]
    } else {
        vec![DecisionEnvelopeV1 {
            id: id(9100 + n),
            commit: seq,
            item_position: 0,
            decision_type: DecisionKind::Policy,
            schema_version: 1,
            actor: id(9200),
            authority: AuthorityKind::Owner,
            grant_id: None,
            command_id,
            command_digest,
            references: adopted.clone(),
            recorded_time: serde_json::from_value(json!("2026-11-27T00:00:00Z")).unwrap(),
            adopted_result: adopted,
            policy: PolicyReferenceV1 {
                id: id(P),
                version: 1,
            },
            corrections: vec![],
            supersedes: vec![],
        }]
    };
    DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records,
        decisions,
        native_head_after: NativeHead {
            semantics_version: 1,
            checkpoint: Digest([0; 32]),
        },
        digest: Digest([u8::try_from(seq.0).unwrap(); 32]),
        command_result: CommandResultV1 {
            command_id,
            digest: command_digest,
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
    }
}

fn replay(state: &mut DomainState, records: Vec<AuthoritativeRecordV1>) {
    let next = commit(state, records);
    ReplayEngine::apply(state, &next).unwrap();
}

/// Replay refuses `records` after `state` and keeps the prior head.
fn refused(state: &DomainState, records: Vec<AuthoritativeRecordV1>) {
    let mut copy = state.clone();
    assert!(ReplayEngine::apply(&mut copy, &commit(state, records)).is_err());
    assert_eq!(copy.through, state.through);
    assert_eq!(copy.domain_digest, state.domain_digest);
    assert_eq!(copy.records, state.records);
    assert_eq!(copy.dataset_policies, state.dataset_policies);
}

/// The engine refuses `request` with no plan, and `state` keeps its head.
fn refuses(state: &DomainState, request: &SetDatasetPolicyV1) {
    let before = (state.through, state.domain_digest, state.records.clone());
    assert!(
        PolicyEngine::set(state, request).is_err(),
        "accepted {request:?}"
    );
    assert_eq!(
        (state.through, state.domain_digest, state.records.clone()),
        before
    );
}

fn revision(plan: &DecisionPlan) -> DatasetPolicyRevisionV1 {
    let [AuthoritativeRecordV1::DatasetPolicyRevision(revision)] = plan.records.as_slice() else {
        panic!("one policy revision")
    };
    revision.clone()
}

fn not_set() -> GoverningPolicyV1 {
    GoverningPolicyV1 {
        ownership_perimeter: PolicySettingV1::NotSet,
        source_priority: PolicySettingV1::NotSet,
        freshness_thresholds: PolicySettingV1::NotSet,
        timezone: PolicySettingV1::NotSet,
        reporting_currency: PolicySettingV1::NotSet,
    }
}

fn policy(n: u32) -> AuthoritativeRecordV1 {
    let mut policy = serde_json::from_str::<Vec<AuthoritativeRecordV1>>(include_str!(
        "../../../fixtures/contracts/v1/records.json"
    ))
    .unwrap()
    .into_iter()
    .find_map(|r| match r {
        AuthoritativeRecordV1::DatasetPolicy(p) => Some(p),
        _ => None,
    })
    .unwrap();
    policy.id = id(n);
    policy.opening_policy = id(n);
    policy.withholding_policy = id(n);
    AuthoritativeRecordV1::DatasetPolicy(policy)
}

fn checking(n: u32, connection: u32, name: &str) -> CatalogRequestV1 {
    let mut request = catalog();
    request.revision_id = id(n + 2);
    request.catalog.account_id = account(n);
    request.catalog.component_id = id(n + 1);
    request.catalog.display_name = name.into();
    request.catalog.aliases = vec![SourceAliasV1 {
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(connection),
            device_id: None,
        },
        original_id: format!("{name}-1"),
    }];
    request.ownership.account_id = account(n);
    request
}

/// Families P and Q, a points unit, and accounts A and B whose aliases use
/// connections S1 and S2, each replayed.
fn world() -> DomainState {
    let mut state = state();
    replay(
        &mut state,
        vec![
            policy(P),
            policy(Q),
            AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
                id: id(730),
                version: 1,
                units: vec![UnitDefinition {
                    id: UnitId(id(POINTS).0),
                    exponent: 0,
                    native_code: "PTS".into(),
                    kind: UnitKind::Analytical,
                }],
            }),
        ],
    );
    for request in [checking(A, S1, "A"), checking(B, S2, "B")] {
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        replay(&mut state, plan.records);
    }
    state
}

#[test]
fn cur_018_policy_revision_round_trip() {
    let mut state = world();
    let policies = state.dataset_policies.clone();
    let p = state.records.get(&id(P)).cloned();
    let ownership = vec![share(A, 1, 1), share(B, 1, 2)];
    let order = vec![id(S2), id(S1)];
    let thresholds = vec![threshold(S1, 86_400)];
    let mut r = request(None, scope(Some("2026-11-01"), None));
    r.ownership_perimeter = change(&state, SourceValue::Known(ownership.clone()));
    r.source_priority = change(&state, SourceValue::Known(order.clone()));
    r.freshness_thresholds = change(&state, SourceValue::Known(thresholds.clone()));
    let plan = PolicyEngine::set(&state, &r).unwrap();
    let recorded = revision(&plan);
    assert_eq!(recorded.policy_id, id(P));
    assert_eq!(recorded.version, 1);
    assert_eq!(recorded.prior_version, None);
    assert_eq!(recorded.effective_scope, r.effective_scope);
    assert_eq!(recorded.ownership_perimeter, r.ownership_perimeter);
    assert_eq!(recorded.source_priority, r.source_priority);
    assert_eq!(recorded.freshness_thresholds, r.freshness_thresholds);
    assert_eq!(recorded.timezone, None);
    assert_eq!(recorded.reporting_currency, None);
    assert!(plan.invalidations.is_empty());
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));

    replay(&mut state, plan.records.clone());
    assert_eq!(
        state.records.get(&recorded.id),
        Some(&AuthoritativeRecordV1::DatasetPolicyRevision(recorded))
    );
    // The bootstrap policy record and its history index are unchanged.
    assert_eq!(state.dataset_policies, policies);
    assert_eq!(state.records.get(&id(P)).cloned(), p);

    let governing = PolicyEngine::governing(&state, day("2026-11-05")).unwrap();
    assert_eq!(
        governing,
        GoverningPolicyV1 {
            ownership_perimeter: PolicySettingV1::Set {
                version: 1,
                value: SourceValue::Known(ownership),
            },
            source_priority: PolicySettingV1::Set {
                version: 1,
                value: SourceValue::Known(order),
            },
            freshness_thresholds: PolicySettingV1::Set {
                version: 1,
                value: SourceValue::Known(thresholds),
            },
            timezone: PolicySettingV1::NotSet,
            reporting_currency: PolicySettingV1::NotSet,
        }
    );
    assert_eq!(
        PolicyEngine::governing(&state, day("2026-10-31")).unwrap(),
        not_set()
    );
}

#[test]
fn cur_018_policy_unknown_stays_unknown() {
    let mut state = world();
    let mut r = request(None, scope(Some("2026-11-01"), None));
    r.timezone = change(&state, SourceValue::Unknown);
    r.reporting_currency = change(&state, SourceValue::Unknown);
    let plan = PolicyEngine::set(&state, &r).unwrap();
    let recorded = revision(&plan);
    assert_eq!(recorded.timezone.unwrap().value, SourceValue::Unknown);
    assert_eq!(
        recorded.reporting_currency.unwrap().value,
        SourceValue::Unknown
    );
    replay(&mut state, plan.records);
    let governing = PolicyEngine::governing(&state, day("2026-11-05")).unwrap();
    assert_eq!(
        governing.timezone,
        PolicySettingV1::Set {
            version: 1,
            value: SourceValue::Unknown,
        }
    );
    assert_eq!(
        governing.reporting_currency,
        PolicySettingV1::Set {
            version: 1,
            value: SourceValue::Unknown,
        }
    );
}

#[test]
fn cur_018_policy_later_revision_wins_inside_its_dates() {
    let mut state = world();
    let v1 = PolicyEngine::set(
        &state,
        &priority(&state, None, scope(Some("2026-11-01"), None), [S1, S2]),
    )
    .unwrap();
    replay(&mut state, v1.records);
    let through_v1 = state.clone();
    let v2 = PolicyEngine::set(
        &state,
        &priority(
            &state,
            Some(1),
            scope(Some("2026-11-10"), Some("2026-11-20")),
            [S2, S1],
        ),
    )
    .unwrap();
    assert_eq!(revision(&v2).version, 2);
    assert_eq!(revision(&v2).prior_version, Some(1));
    replay(&mut state, v2.records);

    let order = |state: &DomainState, civil: &str| {
        PolicyEngine::governing(state, day(civil))
            .unwrap()
            .source_priority
    };
    let set = |version: u32, order: [u32; 2]| PolicySettingV1::Set {
        version,
        value: SourceValue::Known(order.map(id).to_vec()),
    };
    assert_eq!(order(&state, "2026-11-15"), set(2, [S2, S1]));
    assert_eq!(order(&state, "2026-11-25"), set(1, [S1, S2]));
    assert_eq!(order(&through_v1, "2026-11-15"), set(1, [S1, S2]));
}

#[test]
fn cur_018_policy_rejects_bad_requests() {
    let mut state = world();
    let from = scope(Some("2026-11-01"), None);
    let valid = priority(&state, None, from.clone(), [S1, S2]);
    let mutated = |edit: &dyn Fn(&mut SetDatasetPolicyV1)| {
        let mut r = valid.clone();
        edit(&mut r);
        r
    };
    let known = |state: &DomainState| basis(state);
    let s = state.clone();
    for bad in [
        // A family that names no DatasetPolicy record.
        mutated(&|r| r.policy_id = id(999)),
        mutated(&|r| r.policy_id = id(100)),
        // A prior version for the first revision.
        mutated(&|r| r.prior_version = Some(1)),
        // An effective scope on another basis, or ending before it starts.
        mutated(&|r| r.effective_scope.basis = DateBasis::Posting),
        mutated(&|r| r.effective_scope = scope(Some("2026-11-20"), Some("2026-11-10"))),
        // No changed field.
        mutated(&|r| r.source_priority = None),
        // A basis for another dataset, or after the state's commit.
        mutated(&|r| r.source_priority.as_mut().unwrap().basis.dataset_id = DatasetId(id(999).0)),
        mutated(&|r| {
            r.source_priority.as_mut().unwrap().basis.through_commit =
                CommitSeq(known(&s).through_commit.0 + 1)
        }),
        // Ownership of an undeclared account, a zero denominator, a share
        // above one, and an account listed twice.
        mutated(&|r| {
            r.ownership_perimeter = change(&s, SourceValue::Known(vec![share(999, 1, 1)]))
        }),
        mutated(&|r| r.ownership_perimeter = change(&s, SourceValue::Known(vec![share(A, 0, 0)]))),
        mutated(&|r| r.ownership_perimeter = change(&s, SourceValue::Known(vec![share(B, 3, 2)]))),
        mutated(&|r| {
            r.ownership_perimeter =
                change(&s, SourceValue::Known(vec![share(A, 1, 1), share(A, 1, 2)]))
        }),
        // A priority that names an unused connection or one twice.
        mutated(&|r| r.source_priority = change(&s, SourceValue::Known(vec![id(S1), id(999)]))),
        mutated(&|r| r.source_priority = change(&s, SourceValue::Known(vec![id(S1), id(S1)]))),
        // A threshold for an unused connection, one twice, or zero seconds.
        mutated(&|r| {
            r.freshness_thresholds = change(&s, SourceValue::Known(vec![threshold(999, 60)]))
        }),
        mutated(&|r| {
            r.freshness_thresholds = change(
                &s,
                SourceValue::Known(vec![threshold(S1, 60), threshold(S1, 120)]),
            )
        }),
        mutated(&|r| {
            r.freshness_thresholds = change(&s, SourceValue::Known(vec![threshold(S2, 0)]))
        }),
        // A reporting currency that is undeclared or not a currency.
        mutated(&|r| r.reporting_currency = change(&s, SourceValue::Known(UnitId(id(999).0)))),
        mutated(&|r| r.reporting_currency = change(&s, SourceValue::Known(UnitId(id(POINTS).0)))),
    ] {
        refuses(&state, &bad);
    }
    // Each valid field value is accepted on its own.
    for good in [
        mutated(&|r| r.ownership_perimeter = change(&s, SourceValue::Known(vec![share(B, 0, 1)]))),
        mutated(&|r| {
            r.freshness_thresholds = change(&s, SourceValue::Known(vec![threshold(S2, 1)]))
        }),
        mutated(&|r| r.reporting_currency = change(&s, SourceValue::Known(UnitId(id(1).0)))),
    ] {
        assert!(PolicyEngine::set(&state, &good).is_ok(), "refused {good:?}");
    }

    let plan = PolicyEngine::set(&state, &valid).unwrap();
    replay(&mut state, plan.records);
    // After version 1, the prior version must be 1.
    for prior in [None, Some(0), Some(2)] {
        refuses(&state, &priority(&state, prior, from.clone(), [S2, S1]));
    }
    // A dataset has one revision family.
    let mut other = priority(&state, None, from.clone(), [S2, S1]);
    other.policy_id = id(Q);
    refuses(&state, &other);
    assert!(PolicyEngine::set(&state, &priority(&state, Some(1), from, [S2, S1])).is_ok());
}

#[test]
fn cur_018_policy_replay_rejects_forged_revision() {
    let mut state = world();
    let from = scope(Some("2026-11-01"), None);
    let v1 = PolicyEngine::set(&state, &priority(&state, None, from.clone(), [S1, S2])).unwrap();
    // A second revision on the same prior version, planned before version 1.
    let rival = PolicyEngine::set(&state, &priority(&state, None, from.clone(), [S2, S1])).unwrap();
    replay(&mut state, v1.records);
    refused(&state, rival.records);

    // Two revisions on the same prior version in one commit.
    let a = PolicyEngine::set(&state, &priority(&state, Some(1), from.clone(), [S2, S1])).unwrap();
    let mut late = request(Some(1), from.clone());
    late.freshness_thresholds = change(&state, SourceValue::Known(vec![threshold(S1, 60)]));
    let b = PolicyEngine::set(&state, &late).unwrap();
    refused(
        &state,
        a.records.iter().chain(&b.records).cloned().collect(),
    );

    // A stale prior version: version 2 is committed, then a revision that
    // still names version 1.
    replay(&mut state, a.records);
    refused(&state, b.records);

    // An ownership share of 3/2 the engine would refuse.
    let mut owned = request(Some(2), from);
    owned.ownership_perimeter = change(&state, SourceValue::Known(vec![share(B, 1, 2)]));
    let plan = PolicyEngine::set(&state, &owned).unwrap();
    let mut forged = revision(&plan);
    forged.ownership_perimeter.as_mut().unwrap().value = SourceValue::Known(vec![share(B, 3, 2)]);
    refused(
        &state,
        vec![AuthoritativeRecordV1::DatasetPolicyRevision(forged)],
    );
    // A revision under another ID than the one its family and version derive.
    let mut renamed = revision(&plan);
    renamed.id = id(999);
    refused(
        &state,
        vec![AuthoritativeRecordV1::DatasetPolicyRevision(renamed)],
    );
    // The engine's own plan is admitted.
    replay(&mut state, plan.records);
}
