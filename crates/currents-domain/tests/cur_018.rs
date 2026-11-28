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

/// Publish `records` as the next commit. The accounting index follows it.
fn publish(state: &mut DomainState, records: Vec<AuthoritativeRecordV1>) {
    let plan = DecisionPlan {
        records,
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "synthetic".into(),
        }),
    };
    apply(state, &plan);
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
        }
    }

    fn records(&self) -> Vec<AuthoritativeRecordV1> {
        let source = from(self.connection);
        let facet = id(self.n + 2);
        let receipt: ImportReceiptV1 = serde_json::from_value(json!({
            "id": id(self.n + 3),
            "source": source,
            "adapter_version": "synthetic/1",
            "parser_version": "1",
            "requested_coverage": [],
            "reported_coverage": [],
            "received_time": "2026-11-07T00:00:00Z",
            "result": "Complete",
            "cursor": {"state": "Absent"},
            "truncated": false,
            "complete": true,
            "contents": []
        }))
        .unwrap();
        vec![
            AuthoritativeRecordV1::ImportReceipt(receipt),
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
