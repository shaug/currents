//! CUR-018 position queries, explanations and dataset policy commands through
//! the encrypted service.
use currents_domain::position::PositionEngine;
use currents_protocol::{CommandBodyV1, OperationV1, QueryKindV1, QueryOutputV1, QueryV1};
use currents_service as service_api;
use currents_types::*;

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

#[tokio::test]
async fn cur_018_4_service_query_uses_one_basis() {
    let dir = tempfile::tempdir().unwrap();
    let (mut f, _) = bootstrap(&dir.path().join("one-basis.db")).await;
    // The test host authorizes queries that name record 700 only, so the
    // component is record 700.
    let component = support::id(700);
    let mut request = fixture::catalog();
    request.revision_id = support::id(702);
    request.catalog.component_id = component;
    request.catalog.account_id = AccountId(support::id(703).0);
    request.ownership.account_id = request.catalog.account_id;
    request.catalog.display_name = "Synthetic position".into();
    submit(
        &mut f,
        56_001,
        OperationV1::SetAccountCatalogComplete(request),
    )
    .await;
    let state = refresh_financial(&mut f, None).await;
    let head = f.prefix().await.through;
    assert_eq!(state.through, head);

    let basis: ReadBasis = serde_json::from_value(serde_json::json!({
        "dataset_id": support::id(1000),
        "through_commit": head,
        "effective_scope": {"start": null, "end": "2026-11-15", "basis": "Effective"},
        "evaluation_instant": "2026-11-15T12:00:00Z",
        "query_semantics_version": 1,
        "selection_policy_version": 1,
        "unit_catalog_version": 1
    }))
    .unwrap();
    let scope = AccountScope {
        accounts: vec![],
        components: vec![component],
    };
    let cases = [
        (
            QueryKindV1::QueryPosition,
            QueryOutputV1::Position(PositionEngine::query(&state, &basis, &scope).unwrap()),
        ),
        (
            QueryKindV1::Explain,
            QueryOutputV1::Explanation(PositionEngine::explain(&state, &basis, component).unwrap()),
        ),
    ];
    for (kind, expected) in cases {
        let query = QueryV1 {
            kind,
            basis: basis.clone(),
            accounts: vec![],
            records: vec![component],
        };
        let response = f.service.query(f.ctx(), query.clone()).await.unwrap();
        assert_eq!(response.basis, basis);
        assert_eq!(response.processed_head, head);
        assert_eq!(response.output, expected);
        // The same query naming the previous commit is refused.
        let mut previous = query;
        previous.basis.through_commit = CommitSeq(head.0 - 1);
        let error = f.service.query(f.ctx(), previous).await.unwrap_err();
        assert_eq!(error.class, ErrorClass::StaleBasis);
    }
}

/// Connections S1 and S2.
const S1: u32 = 801;
const S2: u32 = 802;
/// Every balance is as of this date.
const AS_OF: &str = "2026-11-05";

/// Components A and B on S1, and C on S2. B also has an alias on S2.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Name {
    A,
    B,
    C,
}

const NAMES: [Name; 3] = [Name::A, Name::B, Name::C];

impl Name {
    fn index(self) -> u128 {
        match self {
            Name::A => 0,
            Name::B => 1,
            Name::C => 2,
        }
    }
    fn account(self) -> AccountId {
        AccountId(support::id(1703 + 10 * self.index()).0)
    }
    /// The test host authorizes queries that name record 700 only, so the
    /// component a test reads, its focus, is record 700.
    fn component(self, focus: Name) -> RecordId {
        if self == focus {
            support::id(700)
        } else {
            support::id(710 + 10 * self.index())
        }
    }
}

fn alias(connection: u32, original_id: &str) -> SourceAliasV1 {
    SourceAliasV1 {
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: fixture::id(connection),
            device_id: None,
        },
        original_id: original_id.into(),
    }
}

/// The state that stored history replays to through `through`, with its
/// accounting index. `empty` is the native head before the genesis commit.
async fn replay_through(
    path: &std::path::Path,
    empty: NativeHead,
    through: CommitSeq,
) -> DomainState {
    let store = open(path).await;
    let mut state = DomainState {
        accounting: Some(AccountingIndexV1::empty(empty)),
        ..DomainState::default()
    };
    for commit in store.history().read_prefix(through).await.unwrap() {
        let mut bytes = vec![];
        for record in &commit.records {
            if let AuthoritativeRecordV1::FrozenAccountingManifest(m) = record {
                let mut stream = store
                    .history()
                    .stream_content(commit.seq, m.instructions.digest)
                    .await
                    .unwrap();
                while let Some(chunk) = stream.next().await.unwrap() {
                    bytes.extend(chunk);
                }
            }
        }
        let mut projection = state.clone();
        currents_domain::ReplayEngine::apply(&mut projection, &commit).unwrap();
        currents_accounting::AccountingAdapter::advance_index(&mut state, &commit, &bytes).unwrap();
        projection.accounting = state.accounting;
        state = projection;
    }
    store.close().await.unwrap();
    state
}

/// A position read at `head` for 15 November.
fn read_basis(head: CommitSeq) -> ReadBasis {
    serde_json::from_value(serde_json::json!({
        "dataset_id": support::id(1000),
        "through_commit": head,
        "effective_scope": {"start": null, "end": "2026-11-15", "basis": "Effective"},
        "evaluation_instant": "2026-11-15T12:00:00Z",
        "query_semantics_version": 1,
        "selection_policy_version": 1,
        "unit_catalog_version": 1
    }))
    .unwrap()
}

/// A service whose genesis retains one balance for A, two for B with the
/// same as-of date that disagree (one per connection), and one for C. Owner
/// commands then declare the three components.
struct World {
    dir: tempfile::TempDir,
    f: support::Fixture,
    empty: NativeHead,
    focus: Name,
}

impl World {
    async fn new(focus: Name) -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Observation `n`, its connection, original ID and amount.
        let balances = [
            (3000, S1, "A-1", 1000),
            (3010, S1, "B-1", 500),
            (3020, S2, "B-2", 600),
            (3030, S2, "C-1", 300),
        ];
        let texts = balances.map(|(n, ..)| format!("synthetic balance {n}"));
        let evidence: Vec<RetainedEvidence> = balances
            .iter()
            .zip(&texts)
            .map(|(&(n, _, _, value), text)| RetainedEvidence {
                n,
                value,
                day: AS_OF,
                kind: ObservationKind::Balance,
                text,
            })
            .collect();
        // Trusted synthetic history: no parser emits balance kinds yet.
        let (mut f, empty) = bootstrap_genesis(
            &dir.path().join("policy.db"),
            &evidence,
            |n, observation, product| {
                let (_, connection, original_id, _) = balances.iter().find(|b| b.0 == n).unwrap();
                observation.source.connection_id = fixture::id(*connection);
                observation.original_ids = vec![(*original_id).into()];
                product.schema_version = 2;
                product.balance_kinds = vec![BalanceFacetKindV1 {
                    facet: product.product.facets[0].lineage_key,
                    kind: BalanceKindV1::Posted,
                }];
            },
        )
        .await;
        for name in NAMES {
            let mut request = fixture::catalog();
            request.revision_id = support::id(1702 + 10 * name.index());
            request.catalog.component_id = name.component(focus);
            request.catalog.account_id = name.account();
            request.ownership.account_id = name.account();
            request.catalog.display_name = format!("Synthetic {name:?}");
            request.catalog.aliases = match name {
                Name::A => vec![alias(S1, "A-1")],
                Name::B => vec![alias(S1, "B-1"), alias(S2, "B-2")],
                Name::C => vec![alias(S2, "C-1")],
            };
            submit(
                &mut f,
                57_100 + name.index(),
                OperationV1::SetAccountCatalogComplete(request),
            )
            .await;
            refresh_financial(&mut f, None).await;
        }
        Self {
            dir,
            f,
            empty,
            focus,
        }
    }

    async fn head(&self) -> CommitSeq {
        self.f.prefix().await.through
    }

    /// A body for command `n` at the current head.
    async fn body(&self, n: u128, request: SetDatasetPolicyV1) -> CommandBodyV1 {
        let mut body = self.f.body(n);
        body.expected_head = self.head().await;
        body.operation = OperationV1::SetDatasetPolicy(request);
        body
    }

    /// The focus component's position through the service at `basis`.
    async fn query(&self, basis: &ReadBasis) -> PositionResponseV1 {
        let query = QueryV1 {
            kind: QueryKindV1::QueryPosition,
            basis: basis.clone(),
            accounts: vec![],
            records: vec![self.focus.component(self.focus)],
        };
        let response = self.f.service.query(self.f.ctx(), query).await.unwrap();
        let QueryOutputV1::Position(position) = response.output else {
            panic!("a position response")
        };
        position
    }

    /// Previews and commits `request` as command `n`, after checking that
    /// the preview lists the revision and the accounts of `affected`. Returns
    /// the focus component's position before and after, and the result. The
    /// position before is also what history replayed through the prior
    /// commit gives.
    async fn change(
        &mut self,
        n: u128,
        request: SetDatasetPolicyV1,
        affected: &[Name],
    ) -> (PositionResponseV1, PositionResponseV1, CommandResultV1) {
        let prior = self.head().await;
        let before = self.query(&read_basis(prior)).await;
        let body = self.body(n, request).await;
        let preview = self.f.service.preview(self.f.ctx(), body.clone()).unwrap();
        let mut accounts: Vec<AccountId> = affected.iter().map(|n| n.account()).collect();
        accounts.sort();
        assert_eq!(preview.effects.accounts, accounts);
        assert_eq!(preview.approval_scope.accounts, accounts);
        assert_eq!(preview.approval_scope.operation, DecisionKind::Policy);
        assert!(preview.effects.invalidations.is_empty());
        assert!(preview.effects.amounts.is_empty());
        let result = self
            .f
            .service
            .submit(self.f.ctx(), body.clone(), self.f.approve(&body))
            .await
            .unwrap();
        assert_eq!(result.commit, Some(CommitSeq(prior.0 + 1)));
        assert_eq!(result.effects, preview.effects);
        let state = refresh_financial(&mut self.f, None).await;
        // The preview's one record is the committed revision.
        let [revision] = result.effects.records.as_slice() else {
            panic!("one revision")
        };
        assert!(matches!(
            state.records.get(revision),
            Some(AuthoritativeRecordV1::DatasetPolicyRevision(_))
        ));
        let after = self.query(&read_basis(state.through)).await;
        // A query replayed through the prior commit reads as before.
        let replayed = replay_through(
            &self.dir.path().join("policy.db"),
            self.empty.clone(),
            prior,
        )
        .await;
        let scope = AccountScope {
            accounts: vec![],
            components: vec![self.focus.component(self.focus)],
        };
        assert_eq!(
            PositionEngine::query(&replayed, &read_basis(prior), &scope).unwrap(),
            before
        );
        (before, after, result)
    }
}

/// A revision of the bootstrap policy family, in effect on every day, that
/// changes no field yet.
fn policy(prior_version: Option<u32>) -> SetDatasetPolicyV1 {
    SetDatasetPolicyV1 {
        policy_id: fixture::id(104),
        prior_version,
        effective_scope: DateScope {
            start: None,
            end: None,
            basis: DateBasis::Effective,
        },
        ownership_perimeter: None,
        source_priority: None,
        freshness_thresholds: None,
        timezone: None,
        reporting_currency: None,
    }
}

fn change<T>(head: CommitSeq, value: SourceValue<T>) -> Option<PolicyChangeV1<T>> {
    Some(PolicyChangeV1 {
        value,
        basis: read_basis(head),
    })
}

/// A first revision whose perimeter gives A `numerator`/2 and keeps B and C
/// at their catalog shares of 1/1.
fn perimeter(head: CommitSeq, numerator: u64) -> SetDatasetPolicyV1 {
    let share = |name: Name, numerator, denominator| OwnershipV1 {
        account_id: name.account(),
        numerator: CommitSeq(numerator),
        denominator: CommitSeq(denominator),
    };
    let mut request = policy(None);
    request.ownership_perimeter = change(
        head,
        SourceValue::Known(vec![
            share(Name::A, numerator, 2),
            share(Name::B, 1, 1),
            share(Name::C, 1, 1),
        ]),
    );
    request
}

/// Counts plan preparations, so a test can show a refusal came before any.
struct Counting {
    inner: RelationshipPlanner,
    prepared: std::sync::atomic::AtomicUsize,
}

impl service_api::CommandPlanner for Counting {
    fn prepare(
        &self,
        body: &CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<service_api::CommandPlan, EngineError> {
        self.prepared
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.prepare(body, prefix)
    }
    fn query(
        &self,
        query: &QueryV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<currents_protocol::QueryResponseV1, EngineError> {
        self.inner.query(query, prefix)
    }
}

#[tokio::test]
async fn cur_018_policy_commands_authorized_and_versioned() {
    use std::sync::atomic::Ordering::SeqCst;
    let mut world = World::new(Name::A).await;
    let state = refresh_financial(&mut world.f, None).await;
    let counting = std::sync::Arc::new(Counting {
        inner: RelationshipPlanner {
            state,
            evidence: None,
        },
        prepared: std::sync::atomic::AtomicUsize::new(0),
    });
    world.f.planner.set(counting.clone());
    let head = world.head().await;
    let next = CommitSeq(head.0 + 1);
    let body = world.body(57_001, perimeter(head, 1)).await;

    // A client the host does not grant the command is refused before any
    // plan is prepared, at preview and at submit, and so is a submit that
    // presents another capability.
    world.f.host.unrelated.store(true, SeqCst);
    let refused = world.f.service.preview(world.f.ctx(), body.clone());
    assert_eq!(refused.unwrap_err().class, ErrorClass::Unauthorized);
    let refused = world
        .f
        .service
        .submit(world.f.ctx(), body.clone(), world.f.auth(None))
        .await;
    assert_eq!(refused.unwrap_err().class, ErrorClass::Unauthorized);
    world.f.host.unrelated.store(false, SeqCst);
    let mut forged = world.f.auth(None);
    forged.capability = "synthetic-capability-forged".into();
    let refused = world
        .f
        .service
        .submit(world.f.ctx(), body.clone(), forged)
        .await;
    assert_eq!(refused.unwrap_err().class, ErrorClass::Unauthorized);
    assert_eq!(counting.prepared.load(SeqCst), 0);
    assert_eq!(world.head().await, head);

    // Two approved requests name the same prior version. The first commits.
    // The second, now naming a stale prior version, is refused at submit and
    // the head does not move.
    let rival = world.body(57_002, perimeter(head, 2)).await;
    let rival_auth = world.f.approve(&rival);
    let first = world
        .f
        .service
        .submit(world.f.ctx(), body.clone(), world.f.approve(&body))
        .await
        .unwrap();
    assert_eq!(first.state, CommandState::Committed);
    assert_eq!(first.commit, Some(next));
    let refused = world
        .f
        .service
        .submit(world.f.ctx(), rival, rival_auth)
        .await;
    assert_eq!(refused.unwrap_err().class, ErrorClass::StaleBasis);
    assert_eq!(world.head().await, next);
    // Restated at the new head, the stale prior version has no plan to
    // approve.
    refresh_financial(&mut world.f, None).await;
    let restated = world.body(57_003, perimeter(next, 2)).await;
    let refused = world.f.service.preview(world.f.ctx(), restated);
    assert_eq!(refused.unwrap_err().class, ErrorClass::InvalidReference);
    assert_eq!(world.head().await, next);

    // An identical retry returns the original result without a second
    // commit, and the same command ID with a changed body is refused.
    let retry = world
        .f
        .service
        .submit(world.f.ctx(), body.clone(), world.f.auth(None))
        .await
        .unwrap();
    assert_eq!(retry, first);
    let mut changed = body;
    changed.operation = OperationV1::SetDatasetPolicy(perimeter(head, 2));
    let refused = world
        .f
        .service
        .submit(world.f.ctx(), changed, world.f.auth(None))
        .await;
    assert_eq!(refused.unwrap_err().class, ErrorClass::InvalidReference);
    assert_eq!(world.head().await, next);
}

#[tokio::test]
async fn cur_018_policy_freshness_edit_affects_only_its_source() {
    let mut world = World::new(Name::C).await;
    let head = world.head().await;
    let mut request = policy(None);
    request.freshness_thresholds = change(
        head,
        SourceValue::Known(vec![FreshnessThresholdV1 {
            source: fixture::id(S2),
            seconds: CommitSeq(604_800),
        }]),
    );
    let (before, after, _) = world.change(57_010, request, &[Name::C]).await;
    let label = |status, threshold| {
        vec![FreshnessV1 {
            source: fixture::id(3030),
            status,
            as_of: fixture::date(AS_OF),
            threshold_seconds: CommitSeq(threshold),
        }]
    };
    // Even the latest instant 5 November can mean is more than seven days
    // before the evaluation instant.
    assert_eq!(after.freshness, label(FreshnessStatus::Stale, 604_800));
    assert_eq!(before.freshness, label(FreshnessStatus::NotAssessed, 0));
    assert_eq!(after.selected_basis, before.selected_basis);
}

#[tokio::test]
async fn cur_018_policy_priority_edit_affects_only_ties() {
    let mut world = World::new(Name::B).await;
    let head = world.head().await;
    let mut request = policy(None);
    request.source_priority = change(
        head,
        SourceValue::Known(vec![fixture::id(S2), fixture::id(S1)]),
    );
    let (before, after, _) = world.change(57_020, request, &[Name::B]).await;
    let conflict = "conflict".to_string();
    assert_eq!(after.selected_basis, vec![fixture::id(3020)]);
    assert!(!after.missing_inputs.contains(&conflict));
    assert!(before.selected_basis.is_empty());
    assert!(before.missing_inputs.contains(&conflict));
}

#[tokio::test]
async fn cur_018_policy_ownership_edit_affects_only_named_account() {
    let mut world = World::new(Name::A).await;
    let head = world.head().await;
    let (before, after, _) = world.change(57_030, perimeter(head, 1), &[Name::A]).await;
    assert_eq!(before.aggregate, AggregateV1::Exact(fixture::amount(1000)));
    assert_eq!(after.aggregate, AggregateV1::Exact(fixture::amount(500)));
}

#[tokio::test]
async fn cur_018_policy_unknown_not_defaulted() {
    let mut world = World::new(Name::A).await;
    let head = world.head().await;
    let mut request = policy(None);
    request.timezone = change(head, SourceValue::Unknown);
    request.reporting_currency = change(head, SourceValue::Unknown);
    let (_, after, result) = world.change(57_040, request, &NAMES).await;
    let state = world.f.service.verified_projection().unwrap();
    let Some(AuthoritativeRecordV1::DatasetPolicyRevision(revision)) =
        state.records.get(&result.effects.records[0])
    else {
        panic!("the committed revision")
    };
    assert_eq!(
        revision.timezone.as_ref().map(|c| &c.value),
        Some(&SourceValue::Unknown)
    );
    assert_eq!(
        revision.reporting_currency.as_ref().map(|c| &c.value),
        Some(&SourceValue::Unknown)
    );
    // Every component is omitted for the unknown reporting currency, so the
    // aggregate has no total in any unit. The service reads the focus
    // component; the position engine reads all three at the same head.
    let everything = PositionEngine::query(
        &state,
        &read_basis(state.through),
        &AccountScope {
            accounts: vec![],
            components: NAMES.iter().map(|n| n.component(Name::A)).collect(),
        },
    )
    .unwrap();
    for response in [&after, &everything] {
        let AggregateV1::Indeterminate {
            components,
            obstructions,
        } = &response.aggregate
        else {
            panic!("no total: {:?}", response.aggregate)
        };
        assert_eq!(components.len(), response.scope.components.len());
        assert_eq!(
            obstructions,
            &vec!["unknown_reporting_currency".to_string()]
        );
    }
    // No zone and no reporting unit stands in for either unknown.
    let usd = serde_json::to_string(&fixture::id(1)).unwrap();
    let revision = serde_json::to_string(revision).unwrap();
    assert!(!revision.contains("\"zone\"") && !revision.contains(&usd));
    for response in [&after, &everything] {
        assert!(
            !serde_json::to_string(response)
                .unwrap()
                .contains("\"zone\"")
        );
    }
}
