//! CUR-018 effective-dated dataset policy revisions and position selection,
//! using synthetic identities and exact public interfaces.
use currents_domain::{
    ReplayEngine, catalog::CatalogEngine, policy::PolicyEngine, position::PositionEngine,
    recognition::RecognitionEngine, reconciliation::ReconciliationEngine,
};
use currents_types::*;
use serde_json::json;
use std::collections::BTreeMap;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;
#[path = "support/associations.rs"]
#[allow(dead_code)]
mod association_fixtures;

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

// Position selection. Component A is the fixture's USD checking account,
// with an alias for connection S1 and another for S2. Savings account B has
// an alias for S1 only.

/// The fixture's checking account (A) and a savings account (B).
const CHECKING: u32 = 10;
const SAVINGS: u32 = 60;
const EXPENSE: u32 = 30;
const EUR: u32 = 740;

fn precedence() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/journeys/position/source-precedence.json"
    ))
    .unwrap()
}

fn connection(name: &str) -> u32 {
    match name {
        "S1" => S1,
        "S2" => S2,
        _ => panic!("unknown source {name}"),
    }
}

fn from(connection: u32) -> SourceIdentityV1 {
    SourceIdentityV1 {
        source: "synthetic".into(),
        connection_id: id(connection),
        device_id: None,
    }
}

fn alias(connection: u32, original_id: &str) -> SourceAliasV1 {
    SourceAliasV1 {
        source: from(connection),
        original_id: original_id.into(),
    }
}

/// A date for a civil `YYYY-MM-DD` value, an instant for an RFC 3339 value.
fn as_of(value: &str) -> TemporalValue {
    if value.contains('T') {
        serde_json::from_value(json!({
            "kind": "Instant",
            "utc": value,
            "original_offset": null,
            "original_zone": null,
            "precision": "Second",
            "original_text": value
        }))
        .unwrap()
    } else {
        date(value)
    }
}

/// Publish `records` as the next commit. The accounting index follows it,
/// and each receipt records this commit, as replay does.
fn publish(state: &mut DomainState, records: Vec<AuthoritativeRecordV1>) {
    let plan = DecisionPlan {
        records,
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "synthetic".into(),
        }),
    };
    apply(state, &plan);
    for record in &plan.records {
        if let AuthoritativeRecordV1::ImportReceipt(receipt) = record {
            state
                .receipt_commits
                .entry(receipt.id)
                .or_insert(state.through);
        }
    }
}

/// Receipt `n` from `connection`, which reports `coverage`.
fn receipt(n: u32, connection: u32, coverage: Vec<CoverageV1>) -> AuthoritativeRecordV1 {
    AuthoritativeRecordV1::ImportReceipt(
        serde_json::from_value(json!({
            "id": id(n),
            "source": from(connection),
            "adapter_version": "synthetic/1",
            "parser_version": "1",
            "requested_coverage": [],
            "reported_coverage": coverage,
            "received_time": "2026-11-07T00:00:00Z",
            "result": "Complete",
            "cursor": {"state": "Absent"},
            "truncated": false,
            "complete": true,
            "contents": []
        }))
        .unwrap(),
    )
}

/// One source balance: observation `n`, its product `n + 1`, its facet
/// `n + 2` and its receipt `n + 3`.
struct Report {
    n: u32,
    connection: u32,
    original_ids: Vec<&'static str>,
    value: i128,
    unit: u32,
    as_of: TemporalValue,
    kind: BalanceKindV1,
    coverage: Vec<CoverageV1>,
}

impl Report {
    /// A posted USD balance for A from `connection`.
    fn posted(n: u32, connection: u32, value: i128, day: &str) -> Self {
        Self {
            n,
            connection,
            original_ids: vec![if connection == S1 { "A-1" } else { "A-2" }],
            value,
            unit: 1,
            as_of: as_of(day),
            kind: BalanceKindV1::Posted,
            coverage: vec![],
        }
    }

    fn records(&self) -> Vec<AuthoritativeRecordV1> {
        let source = from(self.connection);
        let facet = id(self.n + 2);
        vec![
            receipt(self.n + 3, self.connection, self.coverage.clone()),
            AuthoritativeRecordV1::ObservationVersion(ObservationVersionV1 {
                id: id(self.n),
                source,
                original_ids: self.original_ids.iter().map(|s| s.to_string()).collect(),
                sanitized_representation: "synthetic".into(),
                reported_fields: vec![],
                temporal: self.as_of.clone(),
                semantic_kind: ObservationKind::Balance,
                prior_version: None,
                tombstone_of: None,
                content_digest: Digest([0; 32]),
            }),
            AuthoritativeRecordV1::AddressedParseProduct(AddressedParseProductV1 {
                id: id(self.n + 1),
                schema_version: 2,
                product: ParseProductV1 {
                    source_version_id: id(self.n),
                    parser_id: "synthetic".into(),
                    parser_version: "1".into(),
                    facets: vec![MovementFacetV1 {
                        lineage_key: facet,
                        kind: ObservationKind::Balance,
                        amount: SourceValue::Known(
                            Amount::new(self.value, 0, UnitId(id(self.unit).0)).unwrap(),
                        ),
                        relationships: vec![],
                        temporal: self.as_of.clone(),
                    }],
                },
                balance_kinds: vec![BalanceFacetKindV1 {
                    facet,
                    kind: self.kind,
                }],
            }),
        ]
    }
}

/// A with aliases for S1 and S2, B owned 1/2 with an alias for S1, and a
/// EUR unit.
fn positions() -> DomainState {
    let mut state = monetary_state();
    let current = CatalogEngine::current(&state, id(CHECKING + 1)).unwrap();
    let mut revision = catalog();
    revision.revision_id = id(CHECKING + 3);
    revision.prior_revision = Some(current.id);
    revision.catalog = current.catalog;
    revision.catalog.aliases = vec![alias(S1, "A-1"), alias(S2, "A-2")];
    let plan = CatalogEngine::set_complete(&state, &revision).unwrap();
    apply(&mut state, &plan);
    let mut savings = catalog();
    savings.revision_id = id(SAVINGS + 2);
    savings.catalog.component_id = id(SAVINGS + 1);
    savings.catalog.account_id = account(SAVINGS);
    savings.ownership.account_id = account(SAVINGS);
    savings.ownership.numerator = CommitSeq(1);
    savings.ownership.denominator = CommitSeq(2);
    savings.catalog.display_name = "Synthetic savings".into();
    savings.catalog.aliases = vec![alias(S1, "B-1")];
    let plan = CatalogEngine::set_complete(&state, &savings).unwrap();
    apply(&mut state, &plan);
    publish(
        &mut state,
        vec![AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
            id: id(EUR + 1),
            version: 1,
            units: vec![UnitDefinition {
                id: UnitId(id(EUR).0),
                exponent: 2,
                native_code: "EUR".into(),
                kind: UnitKind::Currency,
            }],
        })],
    );
    state
}

fn read_basis(state: &DomainState, requested: &str) -> ReadBasis {
    let mut read = basis(state);
    read.effective_scope = scope(None, Some(requested));
    read
}

fn only(component: u32) -> AccountScope {
    AccountScope {
        accounts: vec![],
        components: vec![id(component + 1)],
    }
}

fn query(state: &DomainState, requested: &str) -> PositionResponseV1 {
    PositionEngine::query(state, &read_basis(state, requested), &only(CHECKING)).unwrap()
}

fn codes(response: &PositionResponseV1) -> Vec<&str> {
    response.missing_inputs.iter().map(String::as_str).collect()
}

fn listed(response: &PositionResponseV1) -> Vec<RecordId> {
    let mut ids: Vec<RecordId> = response.assertions.iter().map(|a| a.id).collect();
    ids.sort();
    ids
}

/// Commit the owner's source priority `order` from `start` on.
fn adopt_priority(state: &mut DomainState, order: &[u32], start: &str) {
    let mut r = request(None, scope(Some(start), None));
    r.policy_id = id(104);
    r.source_priority = change(
        state,
        SourceValue::Known(order.iter().map(|c| id(*c)).collect()),
    );
    let plan = PolicyEngine::set(state, &r).unwrap();
    apply(state, &plan);
}

/// Commit the fixture case `name`'s reports, each in its own receipt, in
/// fixture order or reversed, then check each query against the fixture.
fn precedence_case(name: &str, reversed: bool) -> Vec<PositionResponseV1> {
    let fixture = precedence();
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap();
    let mut state = positions();
    let mut observations = BTreeMap::new();
    let mut reports: Vec<_> = case["reports"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .collect();
    if reversed {
        reports.reverse();
    }
    for (i, report) in reports {
        let n = 2000 + 10 * u32::try_from(i).unwrap();
        let posted = Report::posted(
            n,
            connection(report["source"].as_str().unwrap()),
            report["amount"].as_i64().unwrap().into(),
            report["as_of"].as_str().unwrap(),
        );
        publish(&mut state, posted.records());
        observations.insert(report["name"].as_str().unwrap().to_string(), id(n));
    }
    let named = |names: &serde_json::Value| {
        let mut ids: Vec<RecordId> = names
            .as_array()
            .unwrap()
            .iter()
            .map(|n| observations[n.as_str().unwrap()])
            .collect();
        ids.sort();
        ids
    };
    let mut responses = Vec::new();
    for q in case["queries"].as_array().unwrap() {
        if let Some(priority) = q.get("priority") {
            let order: Vec<u32> = priority["order"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| connection(s.as_str().unwrap()))
                .collect();
            adopt_priority(&mut state, &order, priority["from"].as_str().unwrap());
        }
        let response = query(&state, q["date"].as_str().unwrap());
        assert_eq!(listed(&response), named(&q["listed"]), "{name} {q}");
        assert_eq!(response.selected_basis, named(&q["selected"]), "{name} {q}");
        assert_eq!(
            codes(&response).contains(&"conflict"),
            q["conflict"].as_bool().unwrap(),
            "{name} {q}"
        );
        // No anchor, so the journal reports movements only.
        assert!(codes(&response).contains(&"unknown_opening"));
        assert!(response.anchored_balance.is_empty());
        // A, owned 1/1, counts its selected amount. A conflict or no
        // selection leaves the aggregate without a total.
        let selected = response
            .assertions
            .iter()
            .find(|a| response.selected_basis.contains(&a.id));
        let expected = match (q["conflict"].as_bool().unwrap(), selected) {
            (true, _) => indeterminate(&response, "conflict"),
            (false, Some(assertion)) => AggregateV1::Exact(assertion.amount),
            (false, None) => indeterminate(&response, "unknown_opening"),
        };
        assert_eq!(response.aggregate, expected, "{name} {q}");
        responses.push(response);
    }
    responses
}

/// The aggregate of a response whose only component, A, is omitted for
/// `reason`, with its selected amount or `Unknown`.
fn indeterminate(response: &PositionResponseV1, reason: &str) -> AggregateV1 {
    let selected: Vec<&PositionAssertionV1> = response
        .assertions
        .iter()
        .filter(|a| response.selected_basis.contains(&a.id))
        .collect();
    AggregateV1::Indeterminate {
        components: vec![AggregateComponentV1 {
            component: id(CHECKING + 1),
            amount: selected
                .first()
                .map_or(SourceValue::Unknown, |a| SourceValue::Known(a.amount)),
            evidence: selected.iter().map(|a| a.id).collect(),
        }],
        obstructions: vec![reason.into()],
    }
}

#[test]
fn cur_018_1_older_as_of_received_later_does_not_replace_newer() {
    let [response] = precedence_case("older_as_of_received_later", false)
        .try_into()
        .unwrap();
    let [newer, older] = [id(2000), id(2010)];
    // The 1,000 report arrived later but is as of an earlier date.
    let amounts: Vec<_> = response
        .assertions
        .iter()
        .map(|a| (a.id, a.amount, a.as_of.clone()))
        .collect();
    assert_eq!(
        amounts,
        vec![
            (newer, amount(1200), date("2026-11-05")),
            (older, amount(1000), date("2026-11-03")),
        ]
    );
    assert_eq!(response.selected_basis, vec![newer]);
}

#[test]
fn cur_018_1_requested_date_limits_candidates() {
    precedence_case("requested_date_limits_candidates", false);
}

#[test]
fn cur_018_1_overlapping_boundaries_conflict() {
    precedence_case("overlapping_boundaries_conflict", false);
    precedence_case("overlapping_boundaries_corroborate", false);
}

#[test]
fn cur_018_1_equal_boundary_needs_adopted_priority() {
    let forward = precedence_case("equal_boundary_needs_adopted_priority", false);
    let reversed = precedence_case("equal_boundary_needs_adopted_priority", true);
    // Receipt order is not a selection key: committing S2's receipt first
    // gives the same responses.
    assert_eq!(forward, reversed);
}

/// Point a fixture request for A at `account`: its leg and its first
/// posting.
fn on_account(request: &mut ActivityRequestV1, account: AccountId) {
    for item in &mut request.legs {
        item.leg.account_id = account;
    }
    request.financial.as_mut().unwrap().postings[0].account_id = account;
}

/// Commit a posted activity on A of `value` on `day` against expense,
/// recognized from movement observation `n`.
fn movement(state: &mut DomainState, n: u32, value: i128, day: &str) -> LegId {
    movement_on(state, account(CHECKING), n, value, day)
}

/// Commit a posted activity on `account` of `value` on `day` against
/// expense, recognized from movement observation `n`.
fn movement_on(
    state: &mut DomainState,
    account: AccountId,
    n: u32,
    value: i128,
    day: &str,
) -> LegId {
    let binding = SourceFacetBindingV1 {
        observation: id(n),
        parse_product: id(n + 1),
        facet: id(n + 2),
        amount: amount(value),
    };
    publish(
        state,
        vec![
            AuthoritativeRecordV1::ObservationVersion(ObservationVersionV1 {
                id: id(n),
                source: from(102),
                original_ids: vec![format!("row:{n}")],
                sanitized_representation: "synthetic".into(),
                reported_fields: vec![],
                temporal: date(day),
                semantic_kind: ObservationKind::Movement,
                prior_version: None,
                tombstone_of: None,
                content_digest: Digest([0; 32]),
            }),
            AuthoritativeRecordV1::AddressedParseProduct(AddressedParseProductV1 {
                id: id(n + 1),
                schema_version: 1,
                product: ParseProductV1 {
                    source_version_id: id(n),
                    parser_id: "synthetic".into(),
                    parser_version: "1".into(),
                    facets: vec![MovementFacetV1 {
                        lineage_key: binding.facet,
                        kind: ObservationKind::Movement,
                        amount: SourceValue::Known(amount(value)),
                        relationships: vec![],
                        temporal: date(day),
                    }],
                },
                balance_kinds: vec![],
            }),
        ],
    );
    let mut request = posted(binding, n + 10, EXPENSE);
    on_account(&mut request, account);
    for item in &mut request.legs {
        item.leg.effective = date(day);
        item.leg.posting = date(day);
    }
    let terms = request.financial.as_mut().unwrap();
    terms.bookkeeping_date = date(day);
    for posting in &mut terms.postings {
        posting.effective = date(day);
        posting.posting = date(day);
    }
    let leg = request.legs[0].leg.id;
    let plan = RecognitionEngine::adopt(state, &request).unwrap();
    apply(state, &plan);
    leg
}

#[test]
fn cur_018_unknown_opening_returns_movements_only() {
    let mut state = positions();
    let leg = movement(&mut state, 3000, -40, "2026-11-03");
    let response = query(&state, "2026-11-06");
    assert!(response.anchored_balance.is_empty());
    assert!(response.selected_basis.is_empty());
    assert!(response.assertions.is_empty());
    let [moved] = response.movements.as_slice() else {
        panic!("one movement")
    };
    assert_eq!(moved.account, account(CHECKING));
    assert_eq!(moved.component, id(CHECKING + 1));
    assert_eq!(moved.amount, amount(-40));
    assert_eq!(moved.effective, date("2026-11-03"));
    assert_eq!(moved.evidence, vec![RecordId(leg.0), id(3000)]);
    assert_eq!(codes(&response), vec!["unknown_opening"]);
    // Known movements are not a position: without an opening, A has no
    // amount to count.
    assert_eq!(
        response.aggregate,
        indeterminate(&response, "unknown_opening")
    );
    // A movement after the requested date is not listed.
    movement(&mut state, 3100, -10, "2026-11-08");
    assert_eq!(query(&state, "2026-11-06").movements.len(), 1);
}

#[test]
fn cur_018_anchored_balance_is_not_rolled_forward() {
    let mut state = positions();
    // A's opening of 3,300 at the end of 2026-11-01.
    let opening = supported_opening(&mut state);
    let plan = RecognitionEngine::adopt(&state, &opening).unwrap();
    apply(&mut state, &plan);
    let first = movement(&mut state, 3000, -100, "2026-11-03");

    let response = query(&state, "2026-11-06");
    let [anchored] = response.anchored_balance.as_slice() else {
        panic!("one anchored balance")
    };
    assert_eq!(anchored.component, id(CHECKING + 1));
    assert_eq!(
        anchored.opening,
        RecordId(opening.financial.as_ref().unwrap().recognition_id.0)
    );
    assert_eq!(anchored.amount, SourceValue::Known(amount(3200)));
    assert_eq!(anchored.through, date("2026-11-06"));
    assert_eq!(anchored.dependencies, vec![id(707), RecordId(first.0)]);
    assert!(response.movements.is_empty());
    assert!(response.selected_basis.is_empty());
    assert!(response.missing_inputs.is_empty());
    // Without a selected assertion, A counts its anchored balance.
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(3200)));
    // Before the anchor's cutoff, the opening is unknown.
    let before = query(&state, "2026-10-31");
    assert!(before.anchored_balance.is_empty());
    assert_eq!(codes(&before), vec!["unknown_opening"]);

    // S1 reports 3,250 as of 2026-11-03, and a -50 leg follows.
    publish(
        &mut state,
        Report::posted(2000, S1, 3250, "2026-11-03").records(),
    );
    movement(&mut state, 3100, -50, "2026-11-04");
    let response = query(&state, "2026-11-06");
    assert_eq!(response.selected_basis, vec![id(2000)]);
    let [assertion] = response.assertions.as_slice() else {
        panic!("one assertion")
    };
    assert_eq!(assertion.amount, amount(3250));
    assert_eq!(assertion.as_of, date("2026-11-03"));
    assert_eq!(assertion.source, from(S1));
    assert_eq!(
        response.anchored_balance[0].amount,
        SourceValue::Known(amount(3150))
    );
    // A selected assertion is the component's amount, not the journal's.
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(3250)));
}

#[test]
fn cur_018_incompatible_candidates_never_selected() {
    type Edit = fn(&mut Report);
    let cases: [(&str, Edit); 5] = [
        ("incompatible_balance_kind", |r| {
            r.kind = BalanceKindV1::Available
        }),
        ("unknown_balance_kind", |r| r.kind = BalanceKindV1::Unknown),
        ("incompatible_unit", |r| r.unit = EUR),
        ("unknown_as_of", |r| {
            r.as_of = serde_json::from_value(json!({"kind":"Unknown","original_text":"recently"}))
                .unwrap()
        }),
        ("ambiguous_alias", |r| r.original_ids = vec!["A-1", "B-1"]),
    ];
    for (code, edit) in cases {
        // A posted 1,000 as of 2026-11-03, and a newer balance that cannot
        // be selected.
        let mut state = positions();
        publish(
            &mut state,
            Report::posted(2000, S2, 1000, "2026-11-03").records(),
        );
        let mut newer = Report::posted(2010, S1, 1200, "2026-11-05");
        edit(&mut newer);
        publish(&mut state, newer.records());
        let response = query(&state, "2026-11-06");
        assert_eq!(listed(&response), vec![id(2000), id(2010)], "{code}");
        assert_eq!(response.selected_basis, vec![id(2000)], "{code}");
        assert_eq!(codes(&response), vec![code, "unknown_opening"], "{code}");
    }
}

#[test]
fn cur_018_query_refuses_stale_basis() {
    let mut state = positions();
    publish(
        &mut state,
        Report::posted(2000, S1, 1200, "2026-11-05").records(),
    );
    let stale = |result: Result<PositionResponseV1, EngineError>| {
        assert_eq!(result.unwrap_err().class, ErrorClass::StaleBasis);
    };
    // A basis that names an earlier commit.
    let mut earlier = read_basis(&state, "2026-11-06");
    earlier.through_commit = CommitSeq(state.through.0 - 1);
    stale(PositionEngine::query(&state, &earlier, &only(CHECKING)));
    // A state whose accounting index lags it.
    let mut lagging = state.clone();
    lagging.accounting.as_mut().unwrap().prefix.through = CommitSeq(state.through.0 - 1);
    stale(PositionEngine::query(
        &lagging,
        &read_basis(&lagging, "2026-11-06"),
        &only(CHECKING),
    ));
}

#[test]
fn cur_018_query_refuses_bad_scope() {
    let state = positions();
    let valid = read_basis(&state, "2026-11-06");
    let refused = |read: &ReadBasis, scope: &AccountScope| {
        let error = PositionEngine::query(&state, read, scope).unwrap_err();
        assert_eq!(error.class, ErrorClass::InvalidReference, "{scope:?}");
    };
    let named = |accounts: Vec<AccountId>, components: Vec<RecordId>| AccountScope {
        accounts,
        components,
    };
    // An empty scope, an unknown component or account, and 257 components.
    refused(&valid, &named(vec![], vec![]));
    refused(&valid, &only(999));
    refused(&valid, &named(vec![account(999)], vec![]));
    refused(&valid, &named(vec![], vec![id(CHECKING + 1); 257]));
    // An effective scope with no end, or on the posting basis.
    let mut open = valid.clone();
    open.effective_scope = scope(Some("2026-11-01"), None);
    refused(&open, &only(CHECKING));
    let mut posting = valid.clone();
    posting.effective_scope.basis = DateBasis::Posting;
    refused(&posting, &only(CHECKING));
    // A named account reads its component, once.
    let both = PositionEngine::query(
        &state,
        &valid,
        &named(
            vec![account(CHECKING), account(SAVINGS)],
            vec![id(CHECKING + 1)],
        ),
    )
    .unwrap();
    let AggregateV1::Indeterminate { components, .. } = both.aggregate else {
        panic!("indeterminate")
    };
    let ids: Vec<RecordId> = components.iter().map(|c| c.component).collect();
    assert_eq!(ids, vec![id(CHECKING + 1), id(SAVINGS + 1)]);
}

// Household aggregation (C09). Reporting currency USD, requested date
// 2026-11-15. Checking A reports 1,000.00 and is owned 1/1; savings B reports
// 500.00 and is owned 1/2. Each added account declares one S1 alias.

/// Account C, which has no assertion and no anchor.
const CASH: u32 = 4100;
/// A EUR account.
const EURO: u32 = 4200;
/// A holdings account in a security unit.
const HOLDINGS: u32 = 4300;
/// A card, which is a liability.
const CARD: u32 = 4400;
/// A declared security unit.
const SHARES: u32 = 750;
const REQUESTED: &str = "2026-11-15";

/// Declare external account `n` of `class` in `unit`, owned 1/1, with the
/// S1 alias `"{name}-1"`.
fn add_account(state: &mut DomainState, n: u32, class: AccountClass, unit: u32, name: &str) {
    let mut request = catalog();
    request.revision_id = id(n + 2);
    request.catalog.component_id = id(n + 1);
    request.catalog.account_id = account(n);
    request.ownership.account_id = account(n);
    request.catalog.account_class = class;
    if class == AccountClass::Liability {
        request.catalog.normal_sign = NormalSign::Credit;
    }
    request.catalog.unit_id = UnitId(id(unit).0);
    request.catalog.display_name = name.into();
    request.catalog.aliases = vec![alias(S1, &format!("{name}-1"))];
    let plan = CatalogEngine::set_complete(state, &request).unwrap();
    apply(state, &plan);
}

/// A posted S1 balance of `value` in `unit` as of 2026-11-10 for the account
/// whose alias is `"{alias}"`.
fn report(n: u32, alias: &'static str, value: i128, unit: u32) -> Vec<AuthoritativeRecordV1> {
    let mut report = Report::posted(n, S1, value, "2026-11-10");
    report.original_ids = vec![alias];
    report.unit = unit;
    report.records()
}

/// A reports 1,000.00 and B reports `savings`.
fn household(savings: i128) -> DomainState {
    let mut state = positions();
    publish(&mut state, report(2000, "A-1", 1000, 1));
    publish(&mut state, report(2010, "B-1", savings, 1));
    state
}

/// The aggregate for `accounts` on `requested`.
fn total(state: &DomainState, requested: &str, accounts: &[u32]) -> AggregateV1 {
    let scope = AccountScope {
        accounts: vec![],
        components: accounts.iter().map(|n| id(n + 1)).collect(),
    };
    PositionEngine::query(state, &read_basis(state, requested), &scope)
        .unwrap()
        .aggregate
}

/// The known amount and each omission's component and reason, after
/// checking that the obstructions are the distinct reasons in order.
fn subtotal(aggregate: &AggregateV1) -> (Amount, Vec<(RecordId, &str)>) {
    let AggregateV1::KnownSubtotal {
        known,
        omitted,
        obstructions,
    } = aggregate
    else {
        panic!("known subtotal, not {aggregate:?}")
    };
    let mut reasons: Vec<&str> = omitted.iter().map(|o| o.reason.as_str()).collect();
    reasons.sort_unstable();
    reasons.dedup();
    assert_eq!(obstructions, &reasons);
    (
        *known,
        omitted
            .iter()
            .map(|o| (o.component, o.reason.as_str()))
            .collect(),
    )
}

/// Commit family 104's next revision, effective from `start`, which sets
/// the ownership perimeter to `perimeter`.
fn adopt_perimeter(
    state: &mut DomainState,
    prior: Option<u32>,
    start: Option<&str>,
    perimeter: SourceValue<Vec<OwnershipV1>>,
) {
    let mut r = request(prior, scope(start, None));
    r.policy_id = id(104);
    r.ownership_perimeter = change(state, perimeter);
    let plan = PolicyEngine::set(state, &r).unwrap();
    apply(state, &plan);
}

/// Commit family 104's next revision, effective from 2026-11-01, which sets
/// the reporting currency to `currency`.
fn adopt_currency(state: &mut DomainState, prior: Option<u32>, currency: SourceValue<UnitId>) {
    let mut r = request(prior, scope(Some("2026-11-01"), None));
    r.policy_id = id(104);
    r.reporting_currency = change(state, currency);
    let plan = PolicyEngine::set(state, &r).unwrap();
    apply(state, &plan);
}

#[test]
fn cur_018_3_exact_total_needs_every_input() {
    let state = household(500);
    // 1,000.00 at 1/1 and 500.00 at 1/2.
    assert_eq!(
        total(&state, REQUESTED, &[CHECKING, SAVINGS]),
        AggregateV1::Exact(amount(1250))
    );
}

#[test]
fn cur_018_3_missing_opening_is_not_zero() {
    let mut state = household(500);
    add_account(&mut state, CASH, AccountClass::Asset, 1, "C");
    let aggregate = total(&state, REQUESTED, &[CHECKING, SAVINGS, CASH]);
    assert_eq!(
        subtotal(&aggregate),
        (amount(1250), vec![(id(CASH + 1), "unknown_opening")])
    );
    let AggregateV1::KnownSubtotal { omitted, .. } = &aggregate else {
        unreachable!()
    };
    assert_eq!(omitted[0].amount, SourceValue::Unknown);
    assert!(omitted[0].evidence.is_empty());
}

#[test]
fn cur_018_3_missing_rate_is_not_zero() {
    let mut state = household(500);
    add_account(&mut state, EURO, AccountClass::Asset, EUR, "E");
    publish(&mut state, report(2020, "E-1", 100, EUR));
    let all = [CHECKING, SAVINGS, EURO];
    let aggregate = total(&state, REQUESTED, &all);
    assert_eq!(
        subtotal(&aggregate),
        (amount(1250), vec![(id(EURO + 1), "missing_rate")])
    );
    let AggregateV1::KnownSubtotal { omitted, .. } = &aggregate else {
        unreachable!()
    };
    let euros = Amount::new(100, 0, UnitId(id(EUR).0)).unwrap();
    assert_eq!(omitted[0].amount, SourceValue::Known(euros));
    assert_eq!(omitted[0].evidence, vec![id(2020)]);

    // A governing EUR reporting currency counts the EUR account and omits
    // the USD ones.
    adopt_currency(&mut state, None, SourceValue::Known(UnitId(id(EUR).0)));
    assert_eq!(
        subtotal(&total(&state, REQUESTED, &all)),
        (
            euros,
            vec![
                (id(CHECKING + 1), "missing_rate"),
                (id(SAVINGS + 1), "missing_rate"),
            ]
        )
    );
    // An unknown reporting currency counts nothing.
    adopt_currency(&mut state, Some(1), SourceValue::Unknown);
    let AggregateV1::Indeterminate {
        components,
        obstructions,
    } = total(&state, REQUESTED, &all)
    else {
        panic!("indeterminate")
    };
    assert_eq!(components.len(), 3);
    assert_eq!(obstructions, vec!["unknown_reporting_currency"]);
}

#[test]
fn cur_018_3_missing_value_is_not_zero() {
    let mut state = household(500);
    publish(
        &mut state,
        vec![AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
            id: id(SHARES + 1),
            version: 1,
            units: vec![UnitDefinition {
                id: UnitId(id(SHARES).0),
                exponent: 0,
                native_code: "SYN".into(),
                kind: UnitKind::Security,
            }],
        })],
    );
    add_account(&mut state, HOLDINGS, AccountClass::Asset, SHARES, "H");
    publish(&mut state, report(2030, "H-1", 10, SHARES));
    assert_eq!(
        subtotal(&total(&state, REQUESTED, &[CHECKING, SAVINGS, HOLDINGS])),
        (amount(1250), vec![(id(HOLDINGS + 1), "missing_value")])
    );
}

#[test]
fn cur_018_3_unknown_ownership_is_not_full() {
    let mut state = household(500);
    adopt_perimeter(&mut state, None, None, SourceValue::Unknown);
    let components = vec![
        AggregateComponentV1 {
            component: id(CHECKING + 1),
            amount: SourceValue::Known(amount(1000)),
            evidence: vec![id(2000)],
        },
        AggregateComponentV1 {
            component: id(SAVINGS + 1),
            amount: SourceValue::Known(amount(500)),
            evidence: vec![id(2010)],
        },
    ];
    assert_eq!(
        total(&state, REQUESTED, &[CHECKING, SAVINGS]),
        AggregateV1::Indeterminate {
            components,
            obstructions: vec!["unknown_ownership".into()],
        }
    );
}

#[test]
fn cur_018_ownership_is_effective_dated() {
    let mut state = household(500);
    let before = state.clone();
    adopt_perimeter(
        &mut state,
        None,
        Some("2026-12-01"),
        SourceValue::Known(vec![share(CHECKING, 1, 1), share(SAVINGS, 1, 4)]),
    );
    let both = [CHECKING, SAVINGS];
    assert_eq!(
        total(&state, REQUESTED, &both),
        AggregateV1::Exact(amount(1250))
    );
    assert_eq!(
        total(&state, "2026-12-15", &both),
        AggregateV1::Exact(amount(1125))
    );
    // Through the commit before the revision, the catalog share applies.
    assert_eq!(
        total(&before, "2026-12-15", &both),
        AggregateV1::Exact(amount(1250))
    );
}

#[test]
fn cur_018_inexact_share_is_omitted() {
    let mut state = household(100);
    adopt_perimeter(
        &mut state,
        None,
        None,
        SourceValue::Known(vec![share(CHECKING, 1, 1), share(SAVINGS, 1, 3)]),
    );
    let both = [CHECKING, SAVINGS];
    assert_eq!(
        subtotal(&total(&state, REQUESTED, &both)),
        (amount(1000), vec![(id(SAVINGS + 1), "inexact_share")])
    );
    // 1/8 of 100.00 is exactly 12.50.
    adopt_perimeter(
        &mut state,
        Some(1),
        None,
        SourceValue::Known(vec![share(CHECKING, 1, 1), share(SAVINGS, 1, 8)]),
    );
    assert_eq!(
        total(&state, REQUESTED, &both),
        AggregateV1::Exact(Amount::new(10125, 1, UnitId(id(1).0)).unwrap())
    );
}

#[test]
fn cur_018_liability_subtracts() {
    let mut state = household(500);
    add_account(&mut state, CARD, AccountClass::Liability, 1, "D");
    // The card reports 400.00 owed.
    publish(&mut state, report(2040, "D-1", 400, 1));
    assert_eq!(
        total(&state, REQUESTED, &[CHECKING, SAVINGS, CARD]),
        AggregateV1::Exact(amount(850))
    );
}

/// Household A and B with card D, whose accounting index maps it.
fn carded() -> DomainState {
    let mut state = household(500);
    add_account(&mut state, CARD, AccountClass::Liability, 1, "D");
    state.accounting.as_mut().unwrap().accounts.insert(
        account(CARD),
        NativeAccountMappingV1 {
            account_id: account(CARD),
            unit_id: UnitId(id(1).0),
            account_class: AccountClass::Liability,
            path: format!("fixture/{}", account(CARD).0),
        },
    );
    state
}

/// The request that opens card D at the end of 2026-11-01 from a balance
/// of `owed` and posts `posted` to the card, balanced against account 50.
/// The balance evidence binds no alias, so it is never a selected
/// assertion.
fn card_opening(state: &mut DomainState, owed: i128, posted_to_card: i128) -> ActivityRequestV1 {
    let balance = evidence(state, 5000, owed, None);
    let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
        state.records.get_mut(&id(5000))
    else {
        panic!()
    };
    observation.semantic_kind = ObservationKind::Balance;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get_mut(&id(5001))
    else {
        panic!()
    };
    product.product.facets[0].kind = ObservationKind::Balance;
    let mut opening = posted(balance.clone(), 5100, 50);
    on_account(&mut opening, account(CARD));
    opening.legs[0].leg.movement = amount(posted_to_card);
    let terms = opening.financial.as_mut().unwrap();
    terms.postings[0].amount = amount(posted_to_card);
    terms.postings[1].amount = amount(-posted_to_card);
    terms.opening = Some(OpeningBasisV1 {
        segment_id: id(5200),
        component_id: id(CARD + 1),
        amount: balance.amount,
        cutoff: date("2026-11-01"),
        evidence: balance,
        included_activity: vec![],
    });
    opening
}

#[test]
fn cur_018_liability_opening_credits_the_amount_owed() {
    // A card balance of 400.00 owed opens the card with a 400.00 credit.
    let mut state = carded();
    let opening = card_opening(&mut state, 400, -400);
    assert!(RecognitionEngine::adopt(&state, &opening).is_ok());
    // Debiting the card by the amount owed contradicts the evidence.
    let mut state = carded();
    let opening = card_opening(&mut state, 400, 400);
    assert!(RecognitionEngine::adopt(&state, &opening).is_err());
    // So does crediting it by a balance in journal sign.
    let mut state = carded();
    let opening = card_opening(&mut state, -400, -400);
    assert!(RecognitionEngine::adopt(&state, &opening).is_err());
}

/// A balance claim on card D at the end of 2026-11-10 from the balance in
/// observation `n`.
fn card_claim(state: &DomainState, n: u32) -> ReconciliationClaimV1 {
    let request = ReconciliationRequestV1 {
        kind: ReconciliationKindV1::Balance,
        scope: AccountScope {
            accounts: vec![account(CARD)],
            components: vec![],
        },
        cutoffs: scope(None, Some("2026-11-10")),
        evidence: vec![id(n)],
        exclusions: vec![],
        basis: read_basis(state, "2026-11-10"),
    };
    ReconciliationEngine::evaluate(state, &request).unwrap()
}

#[test]
fn cur_018_card_opening_purchase_claim_and_aggregate_agree() {
    let mut state = carded();
    // The card opens at 400.00 owed at the end of 2026-11-01.
    let opening = card_opening(&mut state, 400, -400);
    let plan = RecognitionEngine::adopt(&state, &opening).unwrap();
    apply(&mut state, &plan);
    // A 50.00 purchase on 2026-11-03 credits the card.
    movement_on(&mut state, account(CARD), 5300, -50, "2026-11-03");
    let household = AccountScope {
        accounts: vec![],
        components: [CHECKING, SAVINGS, CARD].map(|n| id(n + 1)).to_vec(),
    };

    let response =
        PositionEngine::query(&state, &read_basis(&state, REQUESTED), &household).unwrap();
    // The journal reports the card's credit balance; 450.00 is owed.
    let [anchored] = response.anchored_balance.as_slice() else {
        panic!("one anchored balance")
    };
    assert_eq!(anchored.amount, SourceValue::Known(amount(-450)));
    // 1,000.00 + 250.00 - 450.00.
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(800)));

    // An unbound report of 450.00 owed agrees with the journal.
    let mut agreeing = Report::posted(6000, S1, 450, "2026-11-10");
    agreeing.original_ids = vec!["unbound"];
    publish(&mut state, agreeing.records());
    let claim = card_claim(&state, 6000);
    assert_eq!(claim.status, ReconciliationStatus::Proposed);
    assert!(claim.differences.is_empty());
    assert_eq!(claim.matched_allocations[0].amount, amount(450));
    // A report of 460.00 owed differs by 10.00 more owed than the journal.
    let mut higher = Report::posted(6100, S1, 460, "2026-11-10");
    higher.original_ids = vec!["unbound"];
    publish(&mut state, higher.records());
    let claim = card_claim(&state, 6100);
    assert_eq!(claim.reasons, vec![ClaimReasonV1::UnexplainedResidual]);
    assert_eq!(claim.differences, vec![amount(10)]);

    // An activity statement from 400.00 owed through the purchase to
    // 450.00 owed has no residual; one that closes at 460.00 owed is
    // 10.00 short.
    let mut opened = Report::posted(6300, S1, 400, "2026-11-02");
    opened.original_ids = vec!["unbound"];
    publish(&mut state, opened.records());
    let activity = |closing: u32| {
        let request = ReconciliationRequestV1 {
            kind: ReconciliationKindV1::Activity,
            scope: AccountScope {
                accounts: vec![account(CARD)],
                components: vec![],
            },
            cutoffs: scope(Some("2026-11-03"), Some("2026-11-10")),
            evidence: vec![id(5300), id(6300), id(closing)],
            exclusions: vec![],
            basis: read_basis(&state, "2026-11-10"),
        };
        ReconciliationEngine::evaluate(&state, &request).unwrap()
    };
    let claim = activity(6000);
    assert!(!claim.reasons.contains(&ClaimReasonV1::UnexplainedResidual));
    assert!(claim.differences.is_empty());
    assert_eq!(activity(6100).differences, vec![amount(10)]);

    // Selected, the card's own report of 450.00 owed subtracts the same.
    publish(&mut state, report(6200, "D-1", 450, 1));
    let response =
        PositionEngine::query(&state, &read_basis(&state, REQUESTED), &household).unwrap();
    assert!(response.selected_basis.contains(&id(6200)));
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(800)));
}

#[test]
fn cur_018_outside_perimeter_contributes_nothing() {
    let mut state = household(500);
    adopt_perimeter(
        &mut state,
        None,
        None,
        SourceValue::Known(vec![share(CHECKING, 1, 1)]),
    );
    assert_eq!(
        total(&state, REQUESTED, &[CHECKING, SAVINGS]),
        AggregateV1::Exact(amount(1000))
    );
}

// Brokerage overlap. Brokerage total T is declared over cash C and holdings
// H, each a USD asset owned 1/1. C reports 2,000.00.

/// Brokerage total T.
const BROKERAGE: u32 = 4500;

/// A version-2 request that declares T, a USD asset owned 1/1 with the S1
/// alias `"T-1"`, over `parts`.
fn brokerage_total(parts: &[u32]) -> CatalogRequestV1 {
    let mut request = checking(BROKERAGE, S1, "T");
    request.schema_version = 2;
    request.overlaps = parts.iter().map(|n| id(n + 1)).collect();
    request
}

/// T over C and H, where C reports 2,000.00. T reports `total` and H a
/// market value of `holdings`; `None` reports nothing.
fn brokerage(total: Option<i128>, holdings: Option<i128>) -> DomainState {
    let mut state = positions();
    add_account(&mut state, CASH, AccountClass::Asset, 1, "C");
    add_account(&mut state, HOLDINGS, AccountClass::Asset, 1, "H");
    let plan = CatalogEngine::set_complete(&state, &brokerage_total(&[CASH, HOLDINGS])).unwrap();
    apply(&mut state, &plan);
    publish(&mut state, report(7000, "C-1", 2000, 1));
    if let Some(value) = holdings {
        publish(&mut state, report(7010, "H-1", value, 1));
    }
    if let Some(value) = total {
        publish(&mut state, report(7020, "T-1", value, 1));
    }
    state
}

const BROKERED: &[u32] = &[BROKERAGE, CASH, HOLDINGS];

#[test]
fn cur_018_brokerage_total_counts_once() {
    let state = brokerage(Some(10000), Some(8000));
    assert_eq!(
        total(&state, REQUESTED, BROKERED),
        AggregateV1::Exact(amount(10000))
    );
}

#[test]
fn cur_018_brokerage_parts_when_total_unknown() {
    let state = brokerage(None, Some(8000));
    assert_eq!(
        total(&state, REQUESTED, BROKERED),
        AggregateV1::Exact(amount(10000))
    );
}

#[test]
fn cur_018_brokerage_unresolved_overlap() {
    let state = brokerage(None, None);
    assert_eq!(
        subtotal(&total(&state, REQUESTED, BROKERED)),
        (
            amount(2000),
            vec![
                (id(HOLDINGS + 1), "overlap_unresolved"),
                (id(BROKERAGE + 1), "overlap_unresolved"),
            ]
        )
    );
}

#[test]
fn cur_018_brokerage_total_wins_when_parts_disagree() {
    let state = brokerage(Some(10000), Some(7900));
    assert_eq!(
        total(&state, REQUESTED, BROKERED),
        AggregateV1::Exact(amount(10000))
    );
}

#[test]
fn cur_018_overlap_declaration_rules() {
    // A version-1 request keeps its wire form and its record bytes.
    let command: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/v1/recognition/catalog.json"
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(catalog()).unwrap(),
        command["operation"]["value"]
    );
    let plan = CatalogEngine::set_complete(&state(), &catalog()).unwrap();
    assert_eq!(
        serde_json::to_string(&plan.records).unwrap(),
        concat!(
            r#"[{"kind":"AccountComponent","value":{"id":"00000000-0000-0000-0000-000000000011","#,
            r#""account_id":"00000000-0000-0000-0000-000000000010","account_class":"Asset","#,
            r#""unit_id":"00000000-0000-0000-0000-000000000001","normal_sign":"Debit","aliases":[],"#,
            r#""ownership":{"account_id":"00000000-0000-0000-0000-000000000010","numerator":"1","#,
            r#""denominator":"1"},"coverage":[],"opening_boundary":{"state":"Unknown"},"#,
            r#""overlapping_components":[],"lifecycle":"Open","display_name":"Synthetic checking"}},"#,
            r#"{"kind":"AccountCatalogRevision","value":{"id":"00000000-0000-0000-0000-000000000012","#,
            r#""schema_version":1,"component_id":"00000000-0000-0000-0000-000000000011","#,
            r#""account_id":"00000000-0000-0000-0000-000000000010","prior_revision":null,"#,
            r#""catalog":{"account_id":"00000000-0000-0000-0000-000000000010","#,
            r#""component_id":"00000000-0000-0000-0000-000000000011","account_class":"Asset","#,
            r#""unit_id":"00000000-0000-0000-0000-000000000001","normal_sign":"Debit","aliases":[],"#,
            r#""lifecycle":"Open","display_name":"Synthetic checking"},"origin":{"kind":"External"}}}]"#,
        )
    );
    // Only version 2 carries `overlaps`.
    let mut wire = serde_json::to_value(catalog()).unwrap();
    wire["overlaps"] = json!([]);
    assert!(serde_json::from_value::<CatalogRequestV1>(wire.clone()).is_err());
    wire["schema_version"] = json!(2);
    assert!(serde_json::from_value::<CatalogRequestV1>(wire).is_ok());
    let mut request = catalog();
    request.overlaps = vec![id(11)];
    assert!(serde_json::to_value(&request).is_err());
    assert!(CatalogEngine::set_complete(&state(), &request).is_err());

    // Parts C and H, part R, which a revision closes, part P owned 1/2, and
    // part Q.
    const C: u32 = 830;
    const H: u32 = 840;
    const R: u32 = 860;
    const P: u32 = 880;
    const Q: u32 = 890;
    let mut state = world();
    for (n, name) in [(C, "C"), (H, "H"), (R, "R"), (Q, "Q")] {
        let plan = CatalogEngine::set_complete(&state, &checking(n, S1, name)).unwrap();
        replay(&mut state, plan.records);
    }
    let mut half = checking(P, S1, "P");
    half.ownership.numerator = CommitSeq(1);
    half.ownership.denominator = CommitSeq(2);
    let plan = CatalogEngine::set_complete(&state, &half).unwrap();
    replay(&mut state, plan.records);
    let mut closed = checking(R, S1, "R");
    closed.revision_id = id(R + 3);
    closed.prior_revision = Some(id(R + 2));
    closed.catalog.lifecycle = Lifecycle::Closed;
    let plan = CatalogEngine::set_complete(&state, &closed).unwrap();
    replay(&mut state, plan.records);

    let refuses = |state: &DomainState, request: &CatalogRequestV1| {
        assert!(
            CatalogEngine::set_complete(state, request).is_err(),
            "accepted {:?}",
            request.overlaps
        );
    };
    // An overlap list on a revision.
    let mut revision = checking(Q, S1, "Q");
    revision.schema_version = 2;
    revision.revision_id = id(Q + 3);
    revision.prior_revision = Some(id(Q + 2));
    revision.overlaps = vec![id(C + 1)];
    refuses(&state, &revision);
    revision.overlaps = vec![];
    assert!(CatalogEngine::set_complete(&state, &revision).is_ok());
    // An undeclared part, a closed part, the total itself, which does not
    // exist yet, a repeated part and a part owned at another share.
    for parts in [
        vec![id(999)],
        vec![id(R + 1)],
        vec![id(BROKERAGE + 1)],
        vec![id(C + 1), id(C + 1)],
        vec![id(P + 1)],
    ] {
        let mut request = brokerage_total(&[]);
        request.overlaps = parts;
        refuses(&state, &request);
    }

    // T over C and H. Its component lists the parts, and replay re-admits it.
    let plan = CatalogEngine::set_complete(&state, &brokerage_total(&[H, C])).unwrap();
    let AuthoritativeRecordV1::AccountComponent(root) = &plan.records[0] else {
        panic!("the total's component")
    };
    assert_eq!(root.overlapping_components, vec![id(C + 1), id(H + 1)]);
    let mut forged = plan.records.clone();
    if let AuthoritativeRecordV1::AccountComponent(root) = &mut forged[0] {
        root.overlapping_components = vec![id(C + 1), id(R + 1)];
    }
    refused(&state, forged);
    replay(&mut state, plan.records);

    // A second total over a part T lists, or over T itself, is refused; one
    // over Q is not.
    let second = |parts: Vec<RecordId>| {
        let mut request = checking(870, S1, "U");
        request.schema_version = 2;
        request.overlaps = parts;
        request
    };
    refuses(&state, &second(vec![id(C + 1)]));
    refuses(&state, &second(vec![id(BROKERAGE + 1)]));
    assert!(CatalogEngine::set_complete(&state, &second(vec![id(Q + 1)])).is_ok());
}

/// A's position on `requested`, evaluated at `instant`.
fn query_at(state: &DomainState, requested: &str, instant: &str) -> PositionResponseV1 {
    let mut read = serde_json::to_value(read_basis(state, requested)).unwrap();
    read["evaluation_instant"] = json!(instant);
    let read: ReadBasis = serde_json::from_value(read).unwrap();
    PositionEngine::query(state, &read, &only(CHECKING)).unwrap()
}

/// Commit family 104's next revision, effective from 2026-11-01, which sets
/// the freshness thresholds to `thresholds`.
fn adopt_thresholds(
    state: &mut DomainState,
    prior: Option<u32>,
    thresholds: SourceValue<Vec<FreshnessThresholdV1>>,
) {
    let mut r = request(prior, scope(Some("2026-11-01"), None));
    r.policy_id = id(104);
    r.freshness_thresholds = change(state, thresholds);
    let plan = PolicyEngine::set(state, &r).unwrap();
    apply(state, &plan);
}

/// The one freshness label, on the one selected assertion, with its
/// threshold in seconds.
fn label(response: &PositionResponseV1) -> (FreshnessStatus, u64) {
    let [freshness] = response.freshness.as_slice() else {
        panic!("one label, not {:?}", response.freshness)
    };
    assert_eq!(response.selected_basis, vec![freshness.source]);
    let selected = response
        .assertions
        .iter()
        .find(|a| a.id == freshness.source)
        .unwrap();
    assert_eq!(freshness.as_of, selected.as_of);
    (freshness.status, freshness.threshold_seconds.0)
}

#[test]
fn cur_018_1_no_freshness_threshold_is_not_assessed() {
    let mut state = positions();
    publish(
        &mut state,
        Report::posted(2000, S1, 1200, "2026-11-05").records(),
    );
    // No revision sets a threshold.
    let response = query(&state, "2026-11-06");
    assert_eq!(label(&response), (FreshnessStatus::NotAssessed, 0));
    // A threshold for S2 alone sets none for S1.
    adopt_thresholds(
        &mut state,
        None,
        SourceValue::Known(vec![threshold(S2, 60)]),
    );
    for instant in ["2026-11-05T00:00:00Z", "2026-12-31T00:00:00Z"] {
        let response = query_at(&state, "2026-11-06", instant);
        assert_eq!(label(&response), (FreshnessStatus::NotAssessed, 0));
    }
}

#[test]
fn cur_018_freshness_of_an_instant() {
    let mut state = positions();
    publish(
        &mut state,
        Report::posted(2000, S1, 1200, "2026-11-05T12:00:00Z").records(),
    );
    adopt_thresholds(
        &mut state,
        None,
        SourceValue::Known(vec![threshold(S1, 86_400)]),
    );
    for (instant, status) in [
        ("2026-11-06T11:00:00Z", FreshnessStatus::Fresh),
        ("2026-11-06T13:00:00Z", FreshnessStatus::Stale),
    ] {
        let response = query_at(&state, "2026-11-06", instant);
        assert_eq!(label(&response), (status, 86_400), "{instant}");
    }
    // A later revision that sets the threshold `Unknown` labels it so.
    adopt_thresholds(&mut state, Some(1), SourceValue::Unknown);
    let response = query_at(&state, "2026-11-06", "2026-11-06T11:00:00Z");
    assert_eq!(label(&response), (FreshnessStatus::Unknown, 0));
}

#[test]
fn cur_018_freshness_of_a_date() {
    let mut state = positions();
    publish(
        &mut state,
        Report::posted(2000, S1, 1200, "2026-11-05").records(),
    );
    adopt_thresholds(
        &mut state,
        None,
        SourceValue::Known(vec![threshold(S1, 172_800)]),
    );
    // The date spans 2026-11-04T10:00Z to 2026-11-06T12:00Z.
    for (instant, status) in [
        ("2026-11-06T09:00:00Z", FreshnessStatus::Fresh),
        ("2026-11-07T12:00:00Z", FreshnessStatus::Unknown),
        ("2026-11-09T00:00:00Z", FreshnessStatus::Stale),
    ] {
        let response = query_at(&state, "2026-11-06", instant);
        assert_eq!(label(&response), (status, 172_800), "{instant}");
    }
}

/// A's opening of 3,300 at the end of 2026-11-01 and a -100 leg on
/// 2026-11-03, so A's anchored balance is 3,200 from then on.
fn anchored() -> DomainState {
    let mut state = positions();
    let opening = supported_opening(&mut state);
    let plan = RecognitionEngine::adopt(&state, &opening).unwrap();
    apply(&mut state, &plan);
    movement(&mut state, 3000, -100, "2026-11-03");
    state
}

#[test]
fn cur_018_3_stale_assertion_stays_labeled() {
    let mut state = anchored();
    publish(
        &mut state,
        Report::posted(2000, S1, 3250, "2026-11-03").records(),
    );
    adopt_thresholds(
        &mut state,
        None,
        SourceValue::Known(vec![threshold(S1, 86_400)]),
    );
    // Evaluated on 2026-11-27, the 2026-11-03 balance is stale.
    let response = query(&state, "2026-11-06");
    assert_eq!(label(&response), (FreshnessStatus::Stale, 86_400));
    assert_eq!(response.selected_basis, vec![id(2000)]);
    assert_eq!(response.assertions[0].amount, amount(3250));
    assert_eq!(
        response.anchored_balance[0].amount,
        SourceValue::Known(amount(3200))
    );
    // The stale assertion is A's amount, not the anchored balance.
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(3250)));
}

/// Date `request`'s legs, postings and bookkeeping on `day`.
fn dated(request: &mut ActivityRequestV1, day: &str) {
    for item in &mut request.legs {
        item.leg.effective = date(day);
        item.leg.posting = date(day);
    }
    if let Some(terms) = request.financial.as_mut() {
        terms.bookkeeping_date = date(day);
        for posting in &mut terms.postings {
            posting.effective = date(day);
            posting.posting = date(day);
        }
    }
}

#[test]
fn cur_018_pending_stays_separate() {
    let mut state = anchored();
    publish(
        &mut state,
        Report::posted(2000, S1, 3200, "2026-11-05").records(),
    );
    let before = query(&state, "2026-11-06");
    assert!(before.pending.is_empty());

    // A pending -80 on A, effective 2026-11-04.
    let binding = evidence(&mut state, 3200, -80, None);
    let mut request = pending(binding, 3300);
    dated(&mut request, "2026-11-04");
    let leg = request.legs[0].leg.id;
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &plan);

    let response = query(&state, "2026-11-06");
    let [held] = response.pending.as_slice() else {
        panic!("one pending movement")
    };
    assert_eq!(held.account, account(CHECKING));
    assert_eq!(held.component, id(CHECKING + 1));
    assert_eq!(held.amount, amount(-80));
    assert_eq!(held.effective, date("2026-11-04"));
    assert_eq!(held.evidence, vec![RecordId(leg.0), id(3200)]);
    // Nothing else changes: the pending amount is not in the anchored
    // balance, the selected amount or the aggregate.
    assert_eq!(response.assertions, before.assertions);
    assert_eq!(response.selected_basis, before.selected_basis);
    assert_eq!(response.anchored_balance, before.anchored_balance);
    assert_eq!(response.movements, before.movements);
    assert_eq!(response.missing_inputs, before.missing_inputs);
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(3200)));
    // Before its effective date it is not listed.
    assert!(query(&state, "2026-11-03").pending.is_empty());

    // A posted adoption consumes it.
    let source = evidence(&mut state, 3210, -80, Some(id(3200)));
    let mut request = posted(source, 3400, EXPENSE);
    request.prior_pending = Some(id(3300));
    dated(&mut request, "2026-11-04");
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &plan);
    let response = query(&state, "2026-11-06");
    assert!(response.pending.is_empty());
    assert_eq!(
        response.anchored_balance[0].amount,
        SourceValue::Known(amount(3120))
    );
}

/// A Balance request on A at the end of `day` for observation `n`.
fn balance_request(state: &DomainState, n: u32, day: &str) -> ReconciliationRequestV1 {
    ReconciliationRequestV1 {
        kind: ReconciliationKindV1::Balance,
        scope: AccountScope {
            accounts: vec![account(CHECKING)],
            components: vec![],
        },
        cutoffs: scope(None, Some(day)),
        evidence: vec![id(n)],
        exclusions: vec![],
        basis: read_basis(state, day),
    }
}

/// The anchored state with observation 2000 from `report`, and A's position
/// on `requested`.
fn reported(report: &Report, requested: &str) -> (DomainState, PositionResponseV1) {
    let mut state = anchored();
    publish(&mut state, report.records());
    let response = query(&state, requested);
    (state, response)
}

#[test]
fn cur_018_discrepancy_only_for_comparable_balances() {
    // A selected posted 3,250 against the anchored 3,200 on 2026-11-05.
    let (state, response) = reported(&Report::posted(2000, S1, 3250, "2026-11-05"), "2026-11-05");
    let [claim] = response.discrepancies.as_slice() else {
        panic!("one discrepancy, not {:?}", response.discrepancies)
    };
    assert_eq!(claim.differences, vec![amount(50)]);
    // It is the claim the reconciliation engine evaluates, unrecorded.
    let request = balance_request(&state, 2000, "2026-11-05");
    assert_eq!(
        claim,
        &ReconciliationEngine::evaluate(&state, &request).unwrap()
    );
    assert!(!state.records.contains_key(&claim.id));
    // Neither balance moves.
    assert_eq!(
        response.anchored_balance[0].amount,
        SourceValue::Known(amount(3200))
    );
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(3250)));

    // An equal posted balance yields none.
    let (_, response) = reported(&Report::posted(2000, S1, 3200, "2026-11-05"), "2026-11-05");
    assert_eq!(response.selected_basis, vec![id(2000)]);
    assert!(response.discrepancies.is_empty());

    // An `Available` balance of 3,000 yields none.
    let mut available = Report::posted(2000, S1, 3000, "2026-11-05");
    available.kind = BalanceKindV1::Available;
    let (_, response) = reported(&available, "2026-11-05");
    assert_eq!(listed(&response), vec![id(2000)]);
    assert!(response.discrepancies.is_empty());

    // A posted balance as of an instant has no civil date to compare.
    let instant = Report::posted(2000, S1, 3250, "2026-11-05T12:00:00Z");
    let (_, response) = reported(&instant, "2026-11-06");
    assert_eq!(response.selected_basis, vec![id(2000)]);
    assert!(response.discrepancies.is_empty());
}

/// Coverage of effective dates from `start` to `end` with `status`.
fn covered(start: &str, end: &str, status: CoverageStatus) -> CoverageV1 {
    CoverageV1 {
        scope: scope(Some(start), Some(end)),
        status,
    }
}

#[test]
fn cur_018_coverage_of_selected_source() {
    let mut state = positions();
    let october = covered("2026-10-01", "2026-10-31", CoverageStatus::Complete);
    let november = covered("2026-11-01", "2026-11-05", CoverageStatus::Partial);
    let other = covered("2026-10-01", "2026-11-05", CoverageStatus::Complete);
    // S1's balance receipt commits first, and its later receipt has the
    // lower ID, so commit order and ID order differ.
    let mut selected = Report::posted(2000, S1, 1200, "2026-11-05");
    selected.coverage = vec![october.clone()];
    publish(&mut state, selected.records());
    publish(&mut state, vec![receipt(1990, S2, vec![other])]);
    publish(&mut state, vec![receipt(1980, S1, vec![november.clone()])]);
    let response = query(&state, "2026-11-06");
    assert_eq!(response.selected_basis, vec![id(2000)]);
    assert_eq!(response.coverage, vec![october, november]);
}

// Transfer overlap. Checking A, whose S1 alias is "A-1", sends 1,000.00 to
// savings B, whose S2 alias is "B-2", by one transfer association. Both are
// USD assets owned 1/1. The sending leg is dated 2026-11-04 and each
// receiving leg 2026-11-05. Each balance is as of 2026-11-05.

/// Savings B.
const SAVINGS_B: u32 = 4600;
/// The transfer association's family.
const TRANSFER: u32 = 5000;
const SENT: &str = "2026-11-04";
const RECEIVED: &str = "2026-11-05";
const AS_OF: &str = "2026-11-05";
const ASKED: &str = "2026-11-06";
const SCOPED: &[u32] = &[CHECKING, SAVINGS_B];

/// Declare B, owned 1/1.
fn declare_savings_b(state: &mut DomainState) {
    declare_savings_b_owned(state, 1, 1);
}

/// Declare B, owned `numerator / denominator`.
fn declare_savings_b_owned(state: &mut DomainState, numerator: u64, denominator: u64) {
    let mut request = catalog();
    request.revision_id = id(SAVINGS_B + 2);
    request.catalog.component_id = id(SAVINGS_B + 1);
    request.catalog.account_id = account(SAVINGS_B);
    request.ownership.account_id = account(SAVINGS_B);
    request.ownership.numerator = CommitSeq(numerator);
    request.ownership.denominator = CommitSeq(denominator);
    request.catalog.display_name = "B".into();
    request.catalog.aliases = vec![alias(S2, "B-2")];
    let plan = CatalogEngine::set_complete(state, &request).unwrap();
    apply(state, &plan);
}

/// A and B, with no balance or transfer yet.
fn transfer_world() -> DomainState {
    let mut state = positions();
    declare_savings_b(&mut state);
    state
}

/// Movement line `n` from `connection` of `value` dated `day`: observation
/// `n`, its product `n + 1` and its facet `n + 2`.
fn line(
    n: u32,
    connection: u32,
    value: i128,
    day: &str,
) -> (Vec<AuthoritativeRecordV1>, SourceFacetBindingV1) {
    let binding = SourceFacetBindingV1 {
        observation: id(n),
        parse_product: id(n + 1),
        facet: id(n + 2),
        amount: amount(value),
    };
    let records = vec![
        AuthoritativeRecordV1::ObservationVersion(ObservationVersionV1 {
            id: id(n),
            source: from(connection),
            original_ids: vec![format!("line:{n}")],
            sanitized_representation: "synthetic".into(),
            reported_fields: vec![],
            temporal: date(day),
            semantic_kind: ObservationKind::Movement,
            prior_version: None,
            tombstone_of: None,
            content_digest: Digest([0; 32]),
        }),
        AuthoritativeRecordV1::AddressedParseProduct(AddressedParseProductV1 {
            id: id(n + 1),
            schema_version: 1,
            product: ParseProductV1 {
                source_version_id: id(n),
                parser_id: "synthetic".into(),
                parser_version: "1".into(),
                facets: vec![MovementFacetV1 {
                    lineage_key: binding.facet,
                    kind: ObservationKind::Movement,
                    amount: SourceValue::Known(amount(value)),
                    relationships: vec![],
                    temporal: date(day),
                }],
            },
            balance_kinds: vec![],
        }),
    ];
    (records, binding)
}

/// Transfer leg `n + 5` on `account` of `value` dated `day`. With `lined`,
/// it commits line `n` from `connection` and allocates it to the leg.
fn transfer_leg(
    state: &mut DomainState,
    n: u32,
    connection: u32,
    account: AccountId,
    value: i128,
    day: &str,
    lined: bool,
) -> FinancialLegV1 {
    let mut allocations = vec![];
    if lined {
        let (records, binding) = line(n, connection, value, day);
        publish(state, records);
        allocations.push(EvidenceAllocationV1 {
            observation: binding.observation,
            facet: binding.facet,
            amount: binding.amount,
        });
    }
    FinancialLegV1 {
        id: LegId(id(n + 5).0),
        event_id: id(TRANSFER + 9),
        account_id: account,
        movement: amount(value),
        allocations,
        effective: date(day),
        posting: date(day),
        relationships: vec![],
        unresolved_residual: SourceValue::Absent,
        interpretation: SourceValue::Known(interpretation()),
    }
}

/// Revision `n` of the transfer family over `legs`, after `prior`, which
/// allocates `allocations` to the legs.
fn transfer_revision(
    n: u32,
    prior: Option<u32>,
    legs: &[LegId],
    allocations: Vec<QuantifiedFacetAllocationV1>,
) -> AuthoritativeRecordV1 {
    AuthoritativeRecordV1::AssociationRevision(AssociationRevisionV1 {
        id: id(n),
        schema_version: 1,
        root: id(TRANSFER),
        prior_revision: prior.map(id),
        kind: RelationshipKind::Transfer,
        legs: legs.to_vec(),
        allocations,
        captured_allocations: vec![],
        recognitions: vec![],
        residuals: vec![amount(0)],
        superseded_adoptions: vec![],
        survivor: None,
        retired_recognitions: vec![],
        interpretation: interpretation(),
        governing_policy: PolicyReferenceV1 {
            id: id(104),
            version: 1,
        },
    })
}

/// Commit the transfer: A's sending leg of 1,000.00, whose outflow line is
/// committed when `sent`, and one receiving leg on B for each of
/// `received`, whose receipt line is committed when its flag is set. It
/// returns the receiving legs.
fn transfer(state: &mut DomainState, sent: bool, received: &[(i128, bool)]) -> Vec<LegId> {
    let mut legs = vec![transfer_leg(
        state,
        5100,
        S1,
        account(CHECKING),
        -1000,
        SENT,
        sent,
    )];
    for (i, (value, lined)) in received.iter().enumerate() {
        let n = 5200 + 10 * u32::try_from(i).unwrap();
        legs.push(transfer_leg(
            state,
            n,
            S2,
            account(SAVINGS_B),
            *value,
            RECEIVED,
            *lined,
        ));
    }
    let ids: Vec<LegId> = legs.iter().map(|l| l.id).collect();
    let mut records: Vec<_> = legs
        .into_iter()
        .map(AuthoritativeRecordV1::FinancialLeg)
        .collect();
    records.push(transfer_revision(TRANSFER, None, &ids, vec![]));
    publish(state, records);
    ids[1..].to_vec()
}

/// Complete posting-date coverage from `start` through `end`.
fn posting_coverage(start: &str, end: &str) -> CoverageV1 {
    let mut scope = scope(Some(start), Some(end));
    scope.basis = DateBasis::Posting;
    CoverageV1 {
        scope,
        status: CoverageStatus::Complete,
    }
}

/// Receipt `n` from `connection` that reports complete posting-date
/// coverage from 2026-11-01 through 2026-11-05.
fn complete_through_as_of(n: u32, connection: u32) -> AuthoritativeRecordV1 {
    receipt(n, connection, vec![posting_coverage("2026-11-01", AS_OF)])
}

/// A reports `a` and B reports `b`, each a posted balance as of 2026-11-05.
/// The receipt of each source in `complete` reports complete posting-date
/// coverage from 2026-11-01 through 2026-11-05.
fn transfer_balances(state: &mut DomainState, a: i128, b: i128, complete: &[u32]) {
    for (n, connection, original, value) in [(2000, S1, "A-1", a), (2010, S2, "B-2", b)] {
        let mut report = Report::posted(n, connection, value, AS_OF);
        report.original_ids = vec![original];
        if complete.contains(&connection) {
            report.coverage = vec![posting_coverage("2026-11-01", AS_OF)];
        }
        publish(state, report.records());
    }
}

#[test]
fn cur_018_2_sender_included_receiver_excluded_adds_clearing_once() {
    let mut state = transfer_world();
    transfer(&mut state, true, &[(1000, false)]);
    transfer_balances(&mut state, 4000, 2000, &[S2]);
    // The 1,000.00 has left A and has not reached B, so it counts once as
    // clearing: 4,000.00 + 2,000.00 + 1,000.00.
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Exact(amount(7000))
    );
}

#[test]
fn cur_018_2_receiver_included_suppresses_clearing() {
    let mut state = transfer_world();
    transfer(&mut state, true, &[(1000, true)]);
    transfer_balances(&mut state, 4000, 3000, &[]);
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Exact(amount(7000))
    );
}

#[test]
fn cur_018_2_both_included_removes_overlap_once() {
    let mut state = transfer_world();
    transfer(&mut state, false, &[(1000, true)]);
    transfer_balances(&mut state, 5000, 3000, &[S1]);
    let response = PositionEngine::query(
        &state,
        &read_basis(&state, ASKED),
        &AccountScope {
            accounts: vec![],
            components: SCOPED.iter().map(|n| id(n + 1)).collect(),
        },
    )
    .unwrap();
    // A still holds the 1,000.00 and B already holds it, so it is removed
    // once: 5,000.00 + 3,000.00 - 1,000.00.
    assert_eq!(response.aggregate, AggregateV1::Exact(amount(7000)));
    // Neither assertion is rewritten.
    let mut amounts: Vec<Amount> = response.assertions.iter().map(|a| a.amount).collect();
    amounts.sort_by_key(|a| a.coefficient);
    assert_eq!(amounts, vec![amount(3000), amount(5000)]);
    assert_eq!(response.selected_basis, vec![id(2000), id(2010)]);
}

#[test]
fn cur_018_2_balance_receipt_before_feed() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/journeys/position/transfer-overlap.json"
    ))
    .unwrap();
    let whole = |v: &serde_json::Value| -> i128 { v.as_i64().unwrap().into() };
    assert_eq!(fixture["unit"], "USD");
    assert_eq!(whole(&fixture["transfer"]["amount"]), 1000);
    assert_eq!(fixture["transfer"]["sent"], SENT);
    assert_eq!(fixture["transfer"]["received"], RECEIVED);
    let requested = fixture["requested"].as_str().unwrap();
    let [a, b] = fixture["balances"].as_array().unwrap().as_slice() else {
        panic!("two balances")
    };
    assert_eq!((&a["component"], &a["source"]), (&json!("A"), &json!("S1")));
    assert_eq!((&b["component"], &b["source"]), (&json!("B"), &json!("S2")));
    assert_eq!((&a["as_of"], &b["as_of"]), (&json!(AS_OF), &json!(AS_OF)));

    let mut state = transfer_world();
    let [receiving] = transfer(
        &mut state,
        a["line"].as_bool().unwrap(),
        &[(1000, b["line"].as_bool().unwrap())],
    )[..] else {
        panic!("one receiving leg")
    };
    transfer_balances(&mut state, whole(&a["amount"]), whole(&b["amount"]), &[]);
    // B's balance arrived before B's feed, so it may or may not include the
    // receipt.
    let before = &fixture["before_feed"];
    assert_eq!(
        total(&state, requested, SCOPED),
        AggregateV1::Range {
            low: amount(whole(&before["low"])),
            high: amount(whole(&before["high"])),
        }
    );

    // B's feed brings the receipt line and complete coverage, and the next
    // revision of the association allocates the line to the receiving leg.
    let feed = &fixture["feed"];
    assert_eq!(
        (&feed["component"], &feed["source"]),
        (&json!("B"), &json!("S2"))
    );
    assert!(feed["line"].as_bool().unwrap());
    let (records, binding) = line(5300, S2, 1000, RECEIVED);
    publish(&mut state, records);
    let coverage = &feed["complete_coverage"];
    publish(
        &mut state,
        vec![receipt(
            5310,
            S2,
            vec![posting_coverage(
                coverage["start"].as_str().unwrap(),
                coverage["end"].as_str().unwrap(),
            )],
        )],
    );
    publish(
        &mut state,
        vec![transfer_revision(
            TRANSFER + 1,
            Some(TRANSFER),
            &[LegId(id(5105).0), receiving],
            vec![QuantifiedFacetAllocationV1 {
                source: binding,
                leg: receiving,
                allocated: amount(1000),
                mode: FacetAllocationModeV1::Additive,
            }],
        )],
    );
    assert_eq!(
        total(&state, requested, SCOPED),
        AggregateV1::Exact(amount(whole(&fixture["after_feed"]["exact"])))
    );
}

#[test]
fn cur_018_2_partial_receipts() {
    let mut state = transfer_world();
    transfer(&mut state, true, &[(600, true), (400, false)]);
    transfer_balances(&mut state, 4000, 2600, &[S2]);
    // Only the 400.00 receipt is in flight: 4,000.00 + 2,600.00 + 400.00.
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Exact(amount(7000))
    );
}

#[test]
fn cur_018_2_unknown_amount_is_indeterminate() {
    let mut state = transfer_world();
    let [receiving] = transfer(&mut state, true, &[(1000, false)])[..] else {
        panic!("one receiving leg")
    };
    let Some(AuthoritativeRecordV1::FinancialLeg(leg)) =
        state.records.get_mut(&RecordId(receiving.0))
    else {
        panic!("the receiving leg")
    };
    leg.unresolved_residual = SourceValue::Unknown;
    transfer_balances(&mut state, 4000, 2000, &[S2]);
    let AggregateV1::Indeterminate {
        components,
        obstructions,
    } = total(&state, ASKED, SCOPED)
    else {
        panic!("an indeterminate aggregate")
    };
    assert_eq!(obstructions, vec!["transfer_membership"]);
    assert_eq!(
        components
            .iter()
            .map(|c| (c.component, c.amount.clone()))
            .collect::<Vec<_>>(),
        vec![
            (id(CHECKING + 1), SourceValue::Known(amount(4000))),
            (id(SAVINGS_B + 1), SourceValue::Known(amount(2000))),
        ]
    );
}

#[test]
fn cur_018_journal_balance_membership() {
    // A's anchored balance adds its live legs after the cutoff, so it
    // includes a sending leg dated on or before the requested date.
    let mut state = anchored();
    declare_savings_b(&mut state);
    let sending = movement(&mut state, 3100, -1000, SENT);
    let receiving = transfer_leg(
        &mut state,
        5200,
        S2,
        account(SAVINGS_B),
        1000,
        RECEIVED,
        false,
    );
    let receiving_id = receiving.id;
    publish(
        &mut state,
        vec![
            AuthoritativeRecordV1::FinancialLeg(receiving),
            transfer_revision(TRANSFER, None, &[sending, receiving_id], vec![]),
        ],
    );
    let mut report = Report::posted(2010, S2, 2000, AS_OF);
    report.original_ids = vec!["B-2"];
    publish(&mut state, report.records());
    // A's 3,300.00 opening less 100.00 and 1,000.00 is 2,200.00. B may or
    // may not include the receipt: 4,200.00 to 5,200.00.
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Range {
            low: amount(4200),
            high: amount(5200),
        }
    );
    // Complete coverage shows that B excludes it, so it is in flight.
    publish(&mut state, vec![complete_through_as_of(5310, S2)]);
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Exact(amount(5200))
    );
}

#[test]
fn cur_018_transfer_beside_an_omission() {
    let mut state = transfer_world();
    add_account(&mut state, CASH, AccountClass::Asset, 1, "C");
    transfer(&mut state, true, &[(1000, false)]);
    transfer_balances(&mut state, 4000, 2000, &[]);
    let scoped = [CHECKING, SAVINGS_B, CASH];
    // C has no amount, and B may or may not include the receipt. A known
    // subtotal holds one amount, so the aggregate has none.
    let AggregateV1::Indeterminate { obstructions, .. } = total(&state, ASKED, &scoped) else {
        panic!("an indeterminate aggregate")
    };
    assert_eq!(obstructions, vec!["transfer_membership", "unknown_opening"]);
    // Once coverage shows that B excludes the receipt, the subtotal adds it.
    publish(&mut state, vec![complete_through_as_of(5310, S2)]);
    assert_eq!(
        subtotal(&total(&state, ASKED, &scoped)),
        (amount(7000), vec![(id(CASH + 1), "unknown_opening")])
    );
}

#[test]
fn cur_018_transfer_into_a_represented_part() {
    // Total T counts in place of its cash part C, so C's own lines never
    // show whether T's balance includes a receipt on C.
    let mut state = brokerage(Some(10000), Some(8000));
    publish(&mut state, report(2030, "A-1", 4000, 1));
    let sending = transfer_leg(&mut state, 5100, S1, account(CHECKING), -1000, SENT, true);
    let receiving = transfer_leg(&mut state, 5200, S1, account(CASH), 1000, RECEIVED, true);
    let legs = [sending.id, receiving.id];
    publish(
        &mut state,
        vec![
            AuthoritativeRecordV1::FinancialLeg(sending),
            AuthoritativeRecordV1::FinancialLeg(receiving),
            transfer_revision(TRANSFER, None, &legs, vec![]),
        ],
    );
    // A includes the outflow. T may or may not include the receipt:
    // 4,000.00 + 10,000.00, up to 1,000.00 more in flight.
    assert_eq!(
        total(&state, REQUESTED, &[CHECKING, BROKERAGE, CASH, HOLDINGS]),
        AggregateV1::Range {
            low: amount(14000),
            high: amount(15000),
        }
    );
}

#[test]
fn cur_018_transfer_inside_one_counted_total() {
    // Total T counts in place of cash C and holdings H, so 1,000.00 moving
    // from C to H stays inside T's amount.
    let mut state = brokerage(Some(10000), Some(8000));
    let sending = transfer_leg(&mut state, 5100, S1, account(CASH), -1000, SENT, false);
    let receiving = transfer_leg(
        &mut state,
        5200,
        S1,
        account(HOLDINGS),
        1000,
        RECEIVED,
        false,
    );
    let legs = [sending.id, receiving.id];
    publish(
        &mut state,
        vec![
            AuthoritativeRecordV1::FinancialLeg(sending),
            AuthoritativeRecordV1::FinancialLeg(receiving),
            transfer_revision(TRANSFER, None, &legs, vec![]),
        ],
    );
    assert_eq!(
        total(&state, REQUESTED, BROKERED),
        AggregateV1::Exact(amount(10000))
    );
}

#[test]
fn cur_018_transfer_fee_leg_moves_no_funds() {
    // The S2 transfer as admitted: 1,000.00 leaves checking A (account 10)
    // for accounts 60 and 70 as 600.00 and 400.00, and a separate 10.00 fee
    // leg leaves A as well. Each account gets an S1 alias.
    let (mut state, mut request) = association_fixtures::transfer_request();
    for (n, original) in [(CHECKING, "A-1"), (60, "S-1"), (70, "T-1")] {
        let current = CatalogEngine::current(&state, id(n + 1)).unwrap();
        let mut revision = catalog();
        revision.revision_id = id(n + 3);
        revision.prior_revision = Some(current.id);
        revision.catalog = current.catalog;
        revision.catalog.aliases = vec![alias(S1, original)];
        revision.ownership.account_id = account(n);
        let plan = CatalogEngine::set_complete(&state, &revision).unwrap();
        apply(&mut state, &plan);
    }
    // Each S1 balance as of 2026-11-10 includes its movement line. The fee
    // line has not arrived, and no coverage rules it out.
    for (i, leg) in request.legs.iter_mut().take(3).enumerate() {
        let n = 6100 + 10 * u32::try_from(i).unwrap();
        let TemporalValue::Date { civil, .. } = leg.effective else {
            panic!("a dated leg")
        };
        let (records, binding) = line(n, S1, leg.movement.coefficient, &civil.to_string());
        publish(&mut state, records);
        leg.allocations = vec![EvidenceAllocationV1 {
            observation: binding.observation,
            facet: binding.facet,
            amount: binding.amount,
        }];
    }
    let mut records: Vec<_> = request
        .legs
        .into_iter()
        .map(AuthoritativeRecordV1::FinancialLeg)
        .collect();
    records.push(AuthoritativeRecordV1::AssociationRevision(request.revision));
    publish(&mut state, records);
    publish(&mut state, report(6000, "A-1", 4000, 1));
    publish(&mut state, report(6010, "S-1", 600, 1));
    publish(&mut state, report(6020, "T-1", 400, 1));
    // Every balance includes its share of the transfer, and the fee moves
    // no funds between them, so nothing is in flight.
    assert_eq!(
        total(&state, REQUESTED, &[CHECKING, 60, 70]),
        AggregateV1::Exact(amount(5000))
    );
}

#[test]
fn cur_018_transfer_between_unequal_shares() {
    // B is owned 1/2. After the transfer the household holds 1,000.00 less
    // in A and 500.00 more through B, so both orders of arrival give
    // 5,500.00.
    let half = || {
        let mut state = positions();
        declare_savings_b_owned(&mut state, 1, 2);
        state
    };
    // A still holds the funds and B holds them too: 5,000.00 + 1,500.00,
    // less A's copy of 1,000.00 at A's share.
    let mut state = half();
    transfer(&mut state, false, &[(1000, true)]);
    transfer_balances(&mut state, 5000, 3000, &[S1]);
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Exact(amount(5500))
    );
    // The funds are in flight: 4,000.00 + 1,000.00, plus 1,000.00 at B's
    // share.
    let mut state = half();
    transfer(&mut state, true, &[(1000, false)]);
    transfer_balances(&mut state, 4000, 2000, &[S2]);
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Exact(amount(5500))
    );
}

#[test]
fn cur_018_senders_at_different_shares() {
    // A, owned 1/1, and savings S, owned 1/2, send 600.00 and 400.00 to B.
    // Both still hold the funds and B holds them too, so the copy to remove
    // has no single share.
    let mut state = transfer_world();
    let legs = [
        transfer_leg(&mut state, 5100, S1, account(CHECKING), -600, SENT, false),
        transfer_leg(&mut state, 5110, S1, account(SAVINGS), -400, SENT, false),
        transfer_leg(
            &mut state,
            5200,
            S2,
            account(SAVINGS_B),
            1000,
            RECEIVED,
            true,
        ),
    ];
    let ids: Vec<LegId> = legs.iter().map(|l| l.id).collect();
    let mut records: Vec<_> = legs
        .into_iter()
        .map(AuthoritativeRecordV1::FinancialLeg)
        .collect();
    records.push(transfer_revision(TRANSFER, None, &ids, vec![]));
    publish(&mut state, records);
    transfer_balances(&mut state, 5000, 3000, &[S1]);
    let mut savings = Report::posted(2020, S1, 2000, AS_OF);
    savings.original_ids = vec!["B-1"];
    publish(&mut state, savings.records());
    let AggregateV1::Indeterminate { obstructions, .. } =
        total(&state, ASKED, &[CHECKING, SAVINGS, SAVINGS_B])
    else {
        panic!("an indeterminate aggregate")
    };
    assert_eq!(obstructions, vec!["transfer_membership"]);
}

#[test]
fn cur_018_later_transfer_leaves_earlier_total() {
    // Both balances are as of 2026-11-03, before the transfer's legs, and
    // no coverage is reported.
    let mut state = transfer_world();
    transfer(&mut state, true, &[(1000, true)]);
    for (n, connection, original, value) in [(2000, S1, "A-1", 5000), (2010, S2, "B-2", 2000)] {
        let mut report = Report::posted(n, connection, value, "2026-11-03");
        report.original_ids = vec![original];
        publish(&mut state, report.records());
    }
    // On 2026-11-03 no funds have moved.
    assert_eq!(
        total(&state, "2026-11-03", SCOPED),
        AggregateV1::Exact(amount(7000))
    );
    // Later, the same balances still predate both legs, so neither
    // includes them.
    assert_eq!(
        total(&state, ASKED, SCOPED),
        AggregateV1::Exact(amount(7000))
    );
}
