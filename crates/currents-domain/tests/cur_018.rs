//! CUR-018 effective-dated dataset policy revisions and position selection,
//! using synthetic identities and exact public interfaces.
use currents_domain::{
    ReplayEngine, catalog::CatalogEngine, policy::PolicyEngine, position::PositionEngine,
    recognition::RecognitionEngine,
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

/// A with aliases for S1 and S2, B with an alias for S1, and a EUR unit.
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
        responses.push(response);
    }
    responses
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

/// Commit a posted activity on A of `value` on `day` against expense,
/// recognized from movement observation `n`.
fn movement(state: &mut DomainState, n: u32, value: i128, day: &str) -> LegId {
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
    assert!(!matches!(response.aggregate, AggregateV1::Exact(_)));
    assert_eq!(
        response.aggregate,
        AggregateV1::Indeterminate {
            components: vec![AggregateComponentV1 {
                component: id(CHECKING + 1),
                amount: SourceValue::Unknown,
                evidence: vec![],
            }],
            obstructions: vec!["aggregation_unavailable".into()],
        }
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
