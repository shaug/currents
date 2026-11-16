//! Actual encrypted Service preview/submit qualification. All input is synthetic.
#[path = "../../currents-domain/tests/support/recognition.rs"]
#[allow(dead_code)]
mod data;
#[path = "support/lifecycle.rs"]
#[allow(dead_code)]
mod support;
use currents_accounting::AccountingAdapter;
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_domain::{catalog::CatalogEngine, recognition::RecognitionEngine};
use currents_protocol::*;
use currents_service as service_api;
use currents_storage::{EncryptedStore, HistoryAppend};
use currents_types::*;
use service_api::*;
use std::{path::Path, sync::Arc};
use support::Fixture;

struct ScopedHost(Arc<support::Host>);
impl HostGrants for ScopedHost {
    fn revoke_sessions(&self, d: DatasetId, i: InstanceId) {
        self.0.revoke_sessions(d, i);
    }
    fn authenticate(
        &self,
        c: &RequestContext,
        d: DatasetId,
        a: Option<&str>,
        n: OffsetDateTime,
    ) -> Result<(), EngineError> {
        self.0.authenticate(c, d, a, n)
    }
    fn authorize_command(&self, c: &RequestContext, b: &CommandBodyV1) -> Result<(), EngineError> {
        self.0.authorize_command(c, b)
    }
    fn authorize_effects(&self, c: &RequestContext, p: &PreviewV1) -> Result<(), EngineError> {
        self.0.authorize_effects(c, p)
    }
    fn publish_challenge(&self, c: &ApprovalChallenge) -> Result<(), EngineError> {
        self.0.publish_challenge(c)
    }
    fn verify_approval(
        &self,
        c: &RequestContext,
        a: &AuthorizationEnvelope,
        h: &ApprovalChallenge,
        n: OffsetDateTime,
    ) -> Result<(), EngineError> {
        self.0.verify_approval(c, a, h, n)
    }
    fn authorize_result(&self, c: &RequestContext, a: ResultAccess<'_>) -> Result<(), EngineError> {
        self.0.authorize_result(c, a)
    }
    fn authorize_query(&self, c: &RequestContext, q: &QueryV1) -> Result<(), EngineError> {
        let mut guard = q.clone();
        guard.accounts.clear();
        guard.records.clear();
        self.0.authorize_query(c, &guard)?;
        if !matches!(q.kind, QueryKindV1::QueryHistory)
            || q.accounts != vec![AccountId(data::id(10).0)]
            || !q.records.is_empty()
        {
            return Err(EngineError {
                class: ErrorClass::Unauthorized,
                affected_scope: vec![],
                next_action: SafeNextAction::Authenticate,
            });
        }
        Ok(())
    }
}

struct Planner {
    state: DomainState,
    capture: Option<CaptureBatchV1>,
}
impl CommandPlanner for Planner {
    fn prepare(
        &self,
        body: &CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<CommandPlan, EngineError> {
        if self.state.accounting.as_ref().map(|a| &a.prefix) != Some(prefix) {
            return Err(EngineError::invalid());
        }
        let (decision_plan, mut accounts, operation) = match &body.operation {
            OperationV1::SetAccountCatalogComplete(r) => (
                CatalogEngine::set_complete(&self.state, r)?,
                vec![r.catalog.account_id],
                DecisionKind::Catalog,
            ),
            OperationV1::AdoptActivity(r) => (
                RecognitionEngine::adopt(&self.state, r)?,
                r.legs.iter().map(|l| l.leg.account_id).collect(),
                DecisionKind::Recognition,
            ),
            OperationV1::PublishImport(r) => {
                let capture = self.capture.clone().ok_or_else(EngineError::invalid)?;
                if r.receipt != capture.receipt.id || r.checkpoint != capture.cursor {
                    return Err(EngineError::invalid());
                }
                (
                    currents_domain::evidence::EvidenceEngine::publish(
                        &self.state,
                        currents_domain::evidence::ValidatedEvidenceBatchV1::new(capture)?,
                    )?,
                    vec![],
                    DecisionKind::Interpretation,
                )
            }
            _ => return Err(EngineError::version()),
        };
        let frozen_batch = AccountingAdapter::freeze(&decision_plan.accounting, &self.state)?;
        // The trusted adapter must expose every financial consequence, including
        // the explicit counteraccount, to the existing C03 authorization scope.
        let mut amounts = Vec::new();
        for operation in &frozen_batch.operations {
            if let FrozenNativeOperationV1::Entry(entry) = operation {
                accounts.extend(entry.postings.iter().map(|p| p.original.account_id));
                amounts.extend(entry.postings.iter().map(|p| p.original.amount));
            }
        }
        accounts.sort();
        accounts.dedup();
        let records = decision_plan
            .records
            .iter()
            .map(|r| {
                serde_json::from_value(serde_json::to_value(r).unwrap()["value"]["id"].clone())
                    .unwrap()
            })
            .collect();
        Ok(CommandPlan {
            import: self.capture.clone().map(ImportEvidence::new).transpose()?,
            effects: EffectsV1 {
                records,
                accounts: accounts.clone(),
                amounts,
                invalidations: decision_plan.invalidations.clone(),
            },
            approval_scope: ApprovalScopeV1 {
                accounts,
                operation,
                fields: vec!["explicit-recognition".into()],
            },
            decision_plan,
            frozen_batch,
            warnings: vec![],
            decision_id: support::id(body.command_id.0.as_u128() + 200_000),
            manifest_id: support::id(body.command_id.0.as_u128() + 300_000),
            policy: PolicyReferenceV1 {
                id: data::id(104),
                version: 1,
            },
        })
    }
    fn query(
        &self,
        query: &QueryV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<QueryResponseV1, EngineError> {
        // Narrow synthetic current-history projection, not C09 position selection.
        if !matches!(query.kind, QueryKindV1::QueryHistory)
            || query.basis.through_commit != self.state.through
            || self.state.accounting.as_ref().map(|a| &a.prefix) != Some(prefix)
            || query.basis.effective_scope.start.is_some()
            || query.basis.effective_scope.end.is_some()
            || !query.records.is_empty()
            || query.basis.query_semantics_version != 1
            || query.basis.selection_policy_version != 1
            || query.basis.unit_catalog_version != 1
        {
            return Err(EngineError::version());
        }
        RecognitionEngine::audit(&self.state)?;
        let mut items = Vec::new();
        for record in self.state.records.values() {
            if let AuthoritativeRecordV1::Recognition(recognition) = record {
                for leg_id in &recognition.leg_ids {
                    let Some(AuthoritativeRecordV1::FinancialLeg(leg)) =
                        self.state.records.get(&RecordId(leg_id.0))
                    else {
                        return Err(EngineError::invalid());
                    };
                    if !query.accounts.contains(&leg.account_id) {
                        continue;
                    }
                    items.push(HistoryItemV1 {
                        leg: leg.id,
                        account: leg.account_id,
                        amount: SourceValue::Known(leg.movement),
                        effective: leg.effective.clone(),
                        posting: leg.posting.clone(),
                        organization: vec![],
                        evidence: leg.allocations.iter().map(|a| a.observation).collect(),
                    });
                }
            }
        }
        Ok(QueryResponseV1 {
            processed_head: prefix.through,
            basis: query.basis.clone(),
            health: vec![Health::SourcePartial],
            output: QueryOutputV1::History(HistoryResponseV1 {
                basis: query.basis.clone(),
                items,
                unknown_date_bucket: vec![],
                incomplete_buckets: vec![IncompleteBucketV1 {
                    accounts: query.accounts.clone(),
                    scope: query.basis.effective_scope.clone(),
                    missing_inputs: vec!["Unknown opening; movements only, not a balance".into()],
                }],
                next_cursor: None,
            }),
        })
    }
}
async fn open(path: &Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x10; 32]), CipherProfileV1)
        .await
        .unwrap()
}
async fn snapshot(store: &EncryptedStore, empty: NativeHead) -> DomainState {
    let tip = store.writer().audited_prefix().await.unwrap();
    let mut state = DomainState {
        accounting: Some(AccountingIndexV1::empty(empty)),
        ..DomainState::default()
    };
    for commit in store.history().read_prefix(tip.through).await.unwrap() {
        let mut bytes = vec![];
        for r in &commit.records {
            if let AuthoritativeRecordV1::FrozenAccountingManifest(m) = r {
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
        AccountingAdapter::advance_index(&mut state, &commit, &bytes).unwrap();
        projection.accounting = state.accounting;
        RecognitionEngine::audit(&projection).unwrap();
        state = projection;
    }
    state
}
async fn refresh(f: &mut Fixture, path: &Path, empty: NativeHead) -> DomainState {
    f.service.close().await.unwrap();
    let store = open(path).await;
    let state = snapshot(&store, empty).await;
    f.host.reconnect();
    f.service = Service::new(
        store,
        DatasetId(support::id(1000).0),
        InstanceId(support::id(999).0),
        ServiceProfile::Synthetic,
        Arc::new(ScopedHost(f.host.clone())),
        Arc::new(Planner {
            state: state.clone(),
            capture: None,
        }),
        f.clock.clone(),
    )
    .await
    .unwrap();
    support::recover_service(&f.service).await;
    let verified = f.service.verified_projection().unwrap();
    assert_eq!(verified.records, state.records);
    assert_eq!(verified.dataset_policies, state.dataset_policies);
    assert_eq!(verified.domain_digest, state.domain_digest);
    assert_eq!(verified.through, state.through);
    assert_eq!(
        verified.accounting.as_ref().unwrap().prefix,
        state.accounting.as_ref().unwrap().prefix
    );
    assert_eq!(
        verified.accounting.as_ref().unwrap().recognitions,
        state.accounting.as_ref().unwrap().recognitions
    );
    verified
}
async fn submit(f: &mut Fixture, n: u128, operation: OperationV1) -> CommandResultV1 {
    let mut body = f.body(n);
    body.expected_head = f.prefix().await.through;
    body.operation = operation;
    let approval = f.approve(&body);
    f.service.submit(f.ctx(), body, approval).await.unwrap()
}
async fn bootstrap(path: &Path) -> (Fixture, NativeHead) {
    let f = Fixture::at_path(path.to_path_buf()).await;
    let empty = f.prefix().await.native_head;
    f.service.close().await.unwrap();
    let store = open(path).await;
    let mut fixture = data::monetary_state();
    let mut records = Vec::new();
    for (_, r) in std::mem::take(&mut fixture.records) {
        match r {
            AuthoritativeRecordV1::Dataset(mut d) => {
                d.id = DatasetId(support::id(1000).0);
                d.ownership_perimeter.clear();
                records.push(AuthoritativeRecordV1::Dataset(d));
            }
            AuthoritativeRecordV1::DatasetPolicy(mut p) => {
                p.opening_policy = p.id;
                p.withholding_policy = p.id;
                records.push(AuthoritativeRecordV1::DatasetPolicy(p));
            }
            AuthoritativeRecordV1::UnitCatalog(_)
            | AuthoritativeRecordV1::DateConversionPolicy(_) => records.push(r),
            _ => {}
        }
    }
    let mut staged = Vec::new();
    let units = records
        .iter()
        .filter_map(|r| match r {
            AuthoritativeRecordV1::UnitCatalog(c) => Some(c.units.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let native_state = DomainState {
        accounting: Some(AccountingIndexV1::empty(empty.clone())),
        ..DomainState::default()
    };
    let batch = AccountingAdapter::freeze(
        &AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
            accounts: vec![],
            units,
        }),
        &native_state,
    )
    .unwrap();
    staged.push(
        store
            .stage(&AccountingAdapter::instruction_bytes(&batch).unwrap())
            .await
            .unwrap(),
    );
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(
        AccountingAdapter::manifest(data::id(990), &batch).unwrap(),
    ));
    store
        .writer()
        .append_closure(
            HistoryAppend {
                prior_head: CommitSeq(0),
                records,
                decisions: vec![],
                staged,
                native: None,
                result: CommandResultV1 {
                    command_id: CommandId(support::id(100_001).0),
                    digest: Digest([13; 32]),
                    state: CommandState::Committed,
                    commit: None,
                    effects: EffectsV1 {
                        records: vec![],
                        accounts: vec![],
                        amounts: vec![],
                        invalidations: vec![],
                    },
                    health: vec![],
                },
            },
            AccountingAdapter::prepare_closure(&batch).unwrap(),
            &currents_accounting::ClosureControl::default(),
        )
        .await
        .unwrap();
    store.close().await.unwrap();
    let mut f = f;
    f.service = f.reopen().await; // Host/clock only; production domain planner installed by refresh.
    refresh(&mut f, path, empty.clone()).await;
    (f, empty)
}

fn capture(n: u32) -> CaptureBatchV1 {
    capture_version(n, "1")
}
fn capture_version(n: u32, version: &str) -> CaptureBatchV1 {
    use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
    let context = ImportContextV1 {
        receipt_id: data::id(n + if version == "1" { 2 } else { 20_000 }),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: data::id(102),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-01T12:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-01T12:00:00Z\"").unwrap(),
        coverage: vec![],
        row_ids: vec![ImportRowIdsV1 {
            observation: data::id(n),
            product: data::id(n + if version == "1" { 1 } else { 10_000 }),
            facet: data::id(900),
        }],
    };
    let bytes = match n {
        200 => include_bytes!("../../../fixtures/journeys/s1/authorization.csv").as_slice(),
        210 => include_bytes!("../../../fixtures/journeys/s1/posted.csv").as_slice(),
        220 => include_bytes!("../../../fixtures/journeys/s1/enrichment.csv").as_slice(),
        _ => panic!("unsupported synthetic stage"),
    }
    .to_vec();
    FileParser::parse(
        BoundedEvidence::new("pending.csv", bytes, context).unwrap(),
        ParserProfileV1 {
            schema_version: 1,
            format: ImportFormatV1::SyntheticCsv,
            parser_version: version.into(),
            unit: UnitDefinition {
                id: UnitId(data::id(1).0),
                exponent: 2,
                native_code: "USD".into(),
                kind: UnitKind::Currency,
            },
        },
    )
    .unwrap()
}
#[tokio::test]
async fn cur_014_1_import_retains_real_parsed_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slice.db");
    let (mut f, empty) = bootstrap(&path).await;
    let state = refresh(&mut f, &path, empty).await;
    f.service.close().await.unwrap();
    f.host.reconnect();
    let capture = capture(200);
    let request = ImportEvidence::new(capture.clone()).unwrap().request();
    f.service = Service::new(
        open(&path).await,
        DatasetId(support::id(1000).0),
        InstanceId(support::id(999).0),
        ServiceProfile::Synthetic,
        Arc::new(ScopedHost(f.host.clone())),
        Arc::new(Planner {
            state,
            capture: Some(capture),
        }),
        f.clock.clone(),
    )
    .await
    .unwrap();
    support::recover_service(&f.service).await;
    let result = submit(&mut f, 120, OperationV1::PublishImport(request)).await;
    assert_eq!(result.state, CommandState::Committed);
    f.service.close().await.unwrap();
}

async fn install_capture(f: &mut Fixture, path: &Path, empty: NativeHead, capture: CaptureBatchV1) {
    let state = refresh(f, path, empty).await;
    f.service.close().await.unwrap();
    f.host.reconnect();
    f.service = Service::new(
        open(path).await,
        DatasetId(support::id(1000).0),
        InstanceId(support::id(999).0),
        ServiceProfile::Synthetic,
        Arc::new(ScopedHost(f.host.clone())),
        Arc::new(Planner {
            state,
            capture: Some(capture),
        }),
        f.clock.clone(),
    )
    .await
    .unwrap();
    support::recover_service(&f.service).await;
}
async fn import_stage(f: &mut Fixture, path: &Path, empty: &NativeHead, n: u32) {
    let capture = capture(n);
    let request = ImportEvidence::new(capture.clone()).unwrap().request();
    let before = f.prefix().await.native_head;
    install_capture(f, path, empty.clone(), capture).await;
    let mut body = f.body(10_000 + u128::from(n));
    body.expected_head = f.prefix().await.through;
    body.operation = OperationV1::PublishImport(request.clone());
    let mut wrong = body.clone();
    if let OperationV1::PublishImport(ref mut r) = wrong.operation {
        r.staged_batch_digest = Digest([99; 32]);
    }
    assert!(f.service.preview(f.ctx(), wrong).is_err());
    let approval = f.approve(&body);
    let result = f
        .service
        .submit(f.ctx(), body.clone(), approval)
        .await
        .unwrap();
    assert_eq!(result.state, CommandState::Committed);
    assert_eq!(
        f.prefix().await.native_head,
        before,
        "import must not recognize money"
    );
    let state = refresh(f, path, empty.clone()).await;
    assert!(state.records.contains_key(&data::id(n + 2)));
    // Retry old expected_head after reopen without another approval.
    assert_eq!(
        f.service.submit(f.ctx(), body, f.auth(None)).await.unwrap(),
        result
    );
}
async fn setup_accounts(f: &mut Fixture, path: &Path, empty: &NativeHead) {
    submit(
        f,
        120,
        OperationV1::SetAccountCatalogComplete(data::catalog()),
    )
    .await;
    refresh(f, path, empty.clone()).await;
    let mut r = data::catalog();
    r.revision_id = data::id(32);
    r.catalog.component_id = data::id(31);
    r.catalog.account_id = AccountId(data::id(30).0);
    r.ownership.account_id = r.catalog.account_id;
    r.catalog.account_class = AccountClass::Expense;
    r.catalog.normal_sign = NormalSign::Debit;
    r.origin = AccountOriginV1::Internal(InternalAccountV1 {
        role: ControlRoleV1::ExpenseFlow,
        created_at: data::date("2026-01-01"),
    });
    submit(f, 130, OperationV1::SetAccountCatalogComplete(r)).await;
    refresh(f, path, empty.clone()).await;
}
fn binding(n: u32) -> SourceFacetBindingV1 {
    let c = capture(n);
    let facet = &c.products[0].product.facets[0];
    let SourceValue::Known(amount) = facet.amount else {
        panic!("synthetic amount")
    };
    SourceFacetBindingV1 {
        observation: c.observations[0].id,
        parse_product: c.products[0].id,
        facet: facet.lineage_key,
        amount,
    }
}
fn history_query(f: &Fixture, through: CommitSeq) -> QueryV1 {
    let mut q = f.query();
    q.kind = QueryKindV1::QueryHistory;
    q.accounts = vec![AccountId(data::id(10).0)];
    q.records.clear();
    q.basis.through_commit = through;
    q
}
#[tokio::test]
async fn cur_014_1_slice_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("slice.db");
    let (mut f, empty) = bootstrap(&path).await;
    setup_accounts(&mut f, &path, &empty).await;
    import_stage(&mut f, &path, &empty, 200).await;
    let before = f.prefix().await.native_head;
    submit(
        &mut f,
        300,
        OperationV1::AdoptActivity(data::pending(binding(200), 300)),
    )
    .await;
    assert_eq!(f.prefix().await.native_head, before);
    let state = refresh(&mut f, &path, empty.clone()).await;
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
    import_stage(&mut f, &path, &empty, 210).await;
    let mut posted = data::posted(binding(210), 400, 30);
    posted.prior_pending = Some(data::id(300));
    let mut body = f.body(400);
    body.expected_head = f.prefix().await.through;
    body.operation = OperationV1::AdoptActivity(posted);
    let approval = f.approve(&body);
    let result = f
        .service
        .submit(f.ctx(), body.clone(), approval)
        .await
        .unwrap();
    let state = refresh(&mut f, &path, empty.clone()).await;
    assert!(RecognitionEngine::pending(&state).unwrap().is_empty());
    assert_eq!(
        f.service
            .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
            .await
            .unwrap(),
        result
    );
    assert_eq!(
        f.service
            .submit(f.ctx(), body.clone(), f.auth(None))
            .await
            .unwrap(),
        result
    );
    let q = history_query(&f, f.prefix().await.through);
    let answer = f.service.query(f.ctx(), q.clone()).await.unwrap();
    let QueryOutputV1::History(h) = answer.output else {
        panic!("history")
    };
    assert_eq!(h.items.len(), 1);
    assert_eq!(h.items[0].amount, SourceValue::Known(data::amount(-96)));
    assert!(!h.incomplete_buckets.is_empty());
    let mut unauthorized = q;
    unauthorized.accounts.push(AccountId(data::id(30).0));
    assert!(f.service.query(f.ctx(), unauthorized).await.is_err());
    import_stage(&mut f, &path, &empty, 220).await;
    let state = refresh(&mut f, &path, empty.clone()).await;
    assert_eq!(state.accounting.as_ref().unwrap().recognitions.len(), 1);
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let tip = store.writer().audited_prefix().await.unwrap();
    let commits = store.history().read_prefix(tip.through).await.unwrap();
    assert_eq!(
        commits
            .iter()
            .filter(|c| c.command_result.command_id == body.command_id)
            .count(),
        1
    );
    for n in [200, 210, 220] {
        let capture = capture(n);
        let commit=commits.iter().find(|c|c.records.iter().any(|r|matches!(r,AuthoritativeRecordV1::ImportReceipt(v) if v.id==capture.receipt.id))).unwrap();
        for content in capture.contents {
            let mut stream = store
                .history()
                .stream_content(commit.seq, content.digest)
                .await
                .unwrap();
            let mut bytes = vec![];
            while let Some(chunk) = stream.next().await.unwrap() {
                bytes.extend(chunk);
            }
            assert_eq!(bytes, content.bytes);
        }
    }
    let expected = store.history().replay_evidence().await.unwrap();
    let report =
        ReplayCoordinator::rebuild(store.history(), open(&dir.path().join("replayed.db")).await)
            .await
            .unwrap();
    assert!(expected.native_leaf_hash.is_some());
    assert!(expected.matches(&report));
    assert!(report.problems.is_empty());
    store.close().await.unwrap();
}
#[tokio::test]
async fn cur_014_3_synthetic_enforcement() {
    let f = Fixture::new().await;
    assert!(f.production().await.is_err());
    f.service.close().await.unwrap();
}

async fn ready_post(path: &Path) -> (Fixture, NativeHead, CommandBodyV1) {
    let (mut f, empty) = bootstrap(path).await;
    setup_accounts(&mut f, path, &empty).await;
    import_stage(&mut f, path, &empty, 200).await;
    submit(
        &mut f,
        300,
        OperationV1::AdoptActivity(data::pending(binding(200), 300)),
    )
    .await;
    refresh(&mut f, path, empty.clone()).await;
    import_stage(&mut f, path, &empty, 210).await;
    let mut posted = data::posted(binding(210), 400, 30);
    posted.prior_pending = Some(data::id(300));
    let mut body = f.body(400);
    body.expected_head = f.prefix().await.through;
    body.operation = OperationV1::AdoptActivity(posted);
    (f, empty, body)
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cur_014_2_slice_fault() {
    use currents_accounting::test_fixtures::{self, ClosureFault};
    if let Some(path) = std::env::var_os("CUR014_CRASH_PATH") {
        let path = std::path::PathBuf::from(path);
        let empty: NativeHead =
            serde_json::from_slice(&std::fs::read(path.with_extension("empty")).unwrap()).unwrap();
        let body: CommandBodyV1 =
            serde_json::from_slice(&std::fs::read(path.with_extension("body")).unwrap()).unwrap();
        let mut f = Fixture::at_path(path.clone()).await;
        refresh(&mut f, &path, empty).await;
        if std::env::var("CUR014_CRASH_PHASE").unwrap() == "before" {
            let control = Arc::new(test_fixtures::fault_control(
                ClosureFault::PauseBeforeCommit,
            ));
            f.service
                .set_test_control(body.command_id, control.clone())
                .unwrap();
            let _task = f.spawn(&body);
            support::wait_for(|| control.completed_operations() > 0).await;
            assert!(!control.commit_submitted());
        } else {
            let approval = f.approve(&body);
            let result = f.service.submit(f.ctx(), body, approval).await.unwrap();
            assert_eq!(result.state, CommandState::Committed);
        }
        std::fs::write(path.with_extension("boundary"), b"reached").unwrap();
        std::future::pending::<()>().await;
        return;
    }
    for phase in ["before", "after"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.db");
        let (mut f, empty, body) = ready_post(&path).await;
        let before = f.prefix().await;
        f.service.close().await.unwrap();
        std::fs::write(
            path.with_extension("empty"),
            serde_json::to_vec(&empty).unwrap(),
        )
        .unwrap();
        std::fs::write(path.with_extension("body"), canonical_bytes(&body).unwrap()).unwrap();
        let log = std::fs::File::create(dir.path().join("child.log")).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cur_014_2_slice_fault", "--nocapture"])
            .env("CUR014_CRASH_PATH", &path)
            .env("CUR014_CRASH_PHASE", phase)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let reached = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            while !path.with_extension("boundary").exists() {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "crash child exited before boundary: {}",
                    std::fs::read_to_string(dir.path().join("child.log")).unwrap()
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        if reached.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "crash boundary timeout: {}",
                std::fs::read_to_string(dir.path().join("child.log")).unwrap()
            );
        }
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        let state = refresh(&mut f, &path, empty.clone()).await;
        let status = f
            .service
            .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
            .await
            .unwrap();
        assert_eq!(
            status.state,
            if phase == "before" {
                CommandState::DefinitelyNotCommitted
            } else {
                CommandState::Committed
            }
        );
        assert_eq!(
            state.accounting.as_ref().unwrap().recognitions.len(),
            usize::from(phase == "after")
        );
        if phase == "before" {
            assert_eq!(f.prefix().await, before);
        }
        let auth = if phase == "before" {
            f.approve(&body)
        } else {
            f.auth(None)
        };
        let result = f.service.submit(f.ctx(), body.clone(), auth).await.unwrap();
        assert_eq!(result.state, CommandState::Committed);
        refresh(&mut f, &path, empty).await;
        let q = history_query(&f, f.prefix().await.through);
        let QueryOutputV1::History(h) = f.service.query(f.ctx(), q).await.unwrap().output else {
            panic!("history")
        };
        assert_eq!(h.items.len(), 1);
        assert_eq!(h.items[0].amount, SourceValue::Known(data::amount(-96)));
        assert_eq!(
            f.service.submit(f.ctx(), body, f.auth(None)).await.unwrap(),
            result
        );
        f.service.close().await.unwrap();
    }
    // Lost caller/acknowledgment is distinct from a killed process.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lost.db");
    let (mut f, empty, body) = ready_post(&path).await;
    let control = Arc::new(test_fixtures::fault_control(
        ClosureFault::LostCommitAcknowledgment,
    ));
    f.service
        .set_test_control(body.command_id, control.clone())
        .unwrap();
    let caller = f.spawn(&body);
    support::wait_for(|| control.commit_submitted()).await;
    caller.abort();
    test_fixtures::release_control(&control);
    f.service.drain().await.unwrap();
    refresh(&mut f, &path, empty).await;
    let result = f
        .service
        .submit(f.ctx(), body.clone(), f.auth(None))
        .await
        .unwrap();
    assert_eq!(result.state, CommandState::Committed);
    assert_eq!(
        f.service
            .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
            .await
            .unwrap(),
        result
    );
    let mut changed = body;
    changed.expected_head.0 += 1;
    assert!(
        f.service
            .submit(f.ctx(), changed, f.auth(None))
            .await
            .is_err()
    );
    f.service.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cur_014_2_import_rejection_and_cancel() {
    use currents_accounting::test_fixtures::{self, ClosureFault};
    let first = capture(200);
    let second = capture_version(200, "2");
    assert_eq!(first.contents[0].digest, second.contents[0].digest);
    assert_ne!(
        ImportEvidence::new(first.clone())
            .unwrap()
            .request()
            .staged_batch_digest,
        ImportEvidence::new(second)
            .unwrap()
            .request()
            .staged_batch_digest,
        "raw file digest is not batch identity"
    );
    let mut bad = first.clone();
    bad.contents[0].bytes.push(0);
    assert!(ImportEvidence::new(bad).is_err());
    let mut missing = first.clone();
    missing.contents.pop();
    assert!(ImportEvidence::new(missing).is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("canceled.db");
    let (mut f, empty) = bootstrap(&path).await;
    install_capture(&mut f, &path, empty.clone(), first.clone()).await;
    let mut unrelated = f.body(500);
    unrelated.expected_head = f.prefix().await.through;
    unrelated.operation = OperationV1::SetAccountCatalogComplete(data::catalog());
    assert!(
        f.service.preview(f.ctx(), unrelated).is_err(),
        "unrelated command cannot carry content"
    );
    let mut body = f.body(501);
    body.expected_head = f.prefix().await.through;
    body.operation =
        OperationV1::PublishImport(ImportEvidence::new(first.clone()).unwrap().request());
    for kind in 0..3 {
        let mut wrong = body.clone();
        let OperationV1::PublishImport(ref mut r) = wrong.operation else {
            unreachable!()
        };
        match kind {
            0 => r.staged_batch_digest = first.contents[0].digest,
            1 => r.receipt = data::id(9999),
            _ => r.checkpoint = SourceValue::Known("foreign".into()),
        };
        assert!(f.service.preview(f.ctx(), wrong).is_err());
    }
    let before = f.prefix().await;
    let control = Arc::new(test_fixtures::fault_control(
        ClosureFault::PauseBeforeCommit,
    ));
    f.service
        .set_test_control(body.command_id, control.clone())
        .unwrap();
    let task = f.spawn(&body);
    support::wait_pending(
        &f.service,
        f.ctx(),
        body.command_id,
        command_digest(&body).unwrap(),
    )
    .await;
    f.service.cancel(f.ctx(), body.command_id).await.unwrap();
    test_fixtures::release_control(&control);
    assert_eq!(task.await.unwrap().unwrap_err().class, ErrorClass::Canceled);
    assert_eq!(f.prefix().await, before);
    let state = refresh(&mut f, &path, empty.clone()).await;
    assert!(!state.records.contains_key(&first.receipt.id));
    install_capture(&mut f, &path, empty.clone(), first).await;
    let approval = f.approve(&body);
    assert_eq!(
        f.service
            .submit(f.ctx(), body.clone(), approval)
            .await
            .unwrap()
            .state,
        CommandState::Committed
    );
    refresh(&mut f, &path, empty.clone()).await;
    assert_eq!(
        f.service
            .submit(f.ctx(), body, f.auth(None))
            .await
            .unwrap()
            .state,
        CommandState::Committed
    );
    let reparse = capture_version(200, "2");
    let request = ImportEvidence::new(reparse.clone()).unwrap().request();
    install_capture(&mut f, &path, empty.clone(), reparse).await;
    submit(&mut f, 502, OperationV1::PublishImport(request)).await;
    let state = refresh(&mut f, &path, empty).await;
    assert!(state.records.contains_key(&data::id(202)));
    assert!(state.records.contains_key(&data::id(20_200)));
    assert!(state.records.contains_key(&data::id(10_200)));
    f.service.close().await.unwrap();
}
