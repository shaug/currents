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
struct Planner {
    state: DomainState,
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
    fn query(&self, _: &QueryV1, _: &AccountingPrefixV1) -> Result<QueryResponseV1, EngineError> {
        Err(EngineError::version())
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
        f.host.clone(),
        Arc::new(Planner {
            state: state.clone(),
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
fn evidence_bytes(n: u32) -> String {
    match n {
        200 => include_str!("../../../fixtures/journeys/s1/authorization.json").into(),
        210 => include_str!("../../../fixtures/journeys/s1/posted.json").into(),
        220 => include_str!("../../../fixtures/journeys/s1/enrichment.json").into(),
        _ => format!("synthetic observation {n}"),
    }
}
// Trusted synthetic history bootstrap, not a claim to exercise an external provider adapter.
fn synthetic_receipt(n: u32, observation: &ObservationVersionV1) -> AuthoritativeRecordV1 {
    AuthoritativeRecordV1::ImportReceipt(ImportReceiptV1 {
        id: data::id(n + 2),
        source: observation.source.clone(),
        adapter_version: "cur013-synthetic-history/1".into(),
        parser_version: "1".into(),
        requested_coverage: vec![],
        reported_coverage: vec![],
        received_time: serde_json::from_str("\"2026-11-01T12:00:00Z\"").unwrap(),
        result: ImportResult::Complete,
        cursor: SourceValue::Absent,
        truncated: false,
        complete: true,
        contents: vec![ContentReferenceV1 {
            digest: observation.content_digest,
            byte_count: CommitSeq(observation.sanitized_representation.len() as u64),
            media_type: "text/plain".into(),
        }],
    })
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
    for (n, value, prior) in [
        (200, -80, None),
        (210, -96, Some(data::id(200))),
        (230, -96, None),
        (250, 3300, None),
        (260, 3300, None),
    ] {
        data::evidence(&mut fixture, n, value, prior);
        let content = store.stage(evidence_bytes(n).as_bytes()).await.unwrap();
        let Some(AuthoritativeRecordV1::ObservationVersion(o)) =
            fixture.records.get_mut(&data::id(n))
        else {
            panic!()
        };
        o.content_digest = content.digest();
        o.sanitized_representation = evidence_bytes(n);
        if n == 260 {
            o.semantic_kind = ObservationKind::Balance;
        }
        records.push(synthetic_receipt(n, o));
        if n == 260 {
            let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) =
                fixture.records.get_mut(&data::id(n + 1))
            else {
                panic!()
            };
            p.product.facets[0].kind = ObservationKind::Balance;
        }
        staged.push(content);
        records.push(fixture.records[&data::id(n)].clone());
        records.push(fixture.records[&data::id(n + 1)].clone());
    }
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
#[tokio::test]
async fn cur_013_catalog_commands_service() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let (mut f, empty) = bootstrap(&path).await;
    let before = f.prefix().await;
    let mut premature = f.body(100);
    premature.expected_head = before.through;
    premature.operation = OperationV1::AdoptActivity(data::posted(binding(210, -96), 400, 30));
    assert!(f.service.preview(f.ctx(), premature).is_err());
    assert_eq!(f.prefix().await, before);
    let request = data::catalog();
    let result = submit(
        &mut f,
        101,
        OperationV1::SetAccountCatalogComplete(request.clone()),
    )
    .await;
    assert_eq!(result.state, CommandState::Committed);
    let declared = f.prefix().await;
    assert_ne!(before.native_head, declared.native_head);
    let state = refresh(&mut f, &path, empty.clone()).await;
    let mapping = state.accounting.as_ref().unwrap().accounts[&request.catalog.account_id].clone();
    assert!(state.accounting.as_ref().unwrap().recognitions.is_empty());
    let mut update = request;
    update.prior_revision = Some(update.revision_id);
    update.revision_id = data::id(13);
    update.catalog.display_name = "Explicit renamed checking".into();
    submit(&mut f, 102, OperationV1::SetAccountCatalogComplete(update)).await;
    assert_eq!(f.prefix().await.native_head, declared.native_head);
    let state = refresh(&mut f, &path, empty).await;
    assert_eq!(
        state.accounting.as_ref().unwrap().accounts[&mapping.account_id],
        mapping
    );
    assert_eq!(
        CatalogEngine::current(&state, data::id(11))
            .unwrap()
            .catalog
            .display_name,
        "Explicit renamed checking"
    );
    f.service.close().await.unwrap();
}

#[tokio::test]
async fn catalog_authority_and_ownership_negatives_leave_exact_prefix_unchanged() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog-negatives.db");
    let (mut f, empty) = bootstrap(&path).await;
    let before = f.prefix().await;
    let valid = data::catalog();
    let mut body = f.body(110);
    body.expected_head = before.through;
    body.operation = OperationV1::SetAccountCatalogComplete(valid.clone());
    f.host.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        f.service.preview(f.ctx(), body.clone()).unwrap_err().class,
        ErrorClass::Unauthorized
    );
    f.host.revoked.store(false, Ordering::SeqCst);
    for (numerator, denominator, account) in [(1, 0, 10), (2, 1, 10), (1, 1, 99)] {
        let mut invalid = valid.clone();
        invalid.ownership.numerator = CommitSeq(numerator);
        invalid.ownership.denominator = CommitSeq(denominator);
        invalid.ownership.account_id = AccountId(data::id(account).0);
        body.operation = OperationV1::SetAccountCatalogComplete(invalid);
        assert!(f.service.preview(f.ctx(), body.clone()).is_err());
        assert_eq!(f.prefix().await, before);
    }
    let mut wire = serde_json::to_value(&valid).unwrap();
    wire.as_object_mut().unwrap().remove("ownership");
    assert!(serde_json::from_value::<CatalogRequestV1>(wire).is_err());
    assert_eq!(f.prefix().await, before);
    submit(
        &mut f,
        111,
        OperationV1::SetAccountCatalogComplete(valid.clone()),
    )
    .await;
    refresh(&mut f, &path, empty).await;
    let accepted = f.prefix().await;
    let mut change = valid;
    change.revision_id = data::id(13);
    change.prior_revision = Some(data::id(12));
    change.ownership.denominator = CommitSeq(2);
    body.expected_head = accepted.through;
    body.operation = OperationV1::SetAccountCatalogComplete(change);
    assert!(f.service.preview(f.ctx(), body).is_err());
    assert_eq!(f.prefix().await, accepted);
    f.service.close().await.unwrap();
}

async fn declare_activity_accounts(f: &mut Fixture, path: &Path, empty: &NativeHead) {
    submit(
        f,
        120,
        OperationV1::SetAccountCatalogComplete(data::catalog()),
    )
    .await;
    refresh(f, path, empty.clone()).await;
    for (n, role, class, sign) in [
        (
            30,
            ControlRoleV1::ExpenseFlow,
            AccountClass::Expense,
            NormalSign::Debit,
        ),
        (
            40,
            ControlRoleV1::UnresolvedEquity,
            AccountClass::Equity,
            NormalSign::Credit,
        ),
        (
            50,
            ControlRoleV1::OpeningEquity,
            AccountClass::Equity,
            NormalSign::Credit,
        ),
    ] {
        let mut request = data::catalog();
        request.revision_id = data::id(n + 2);
        request.catalog.component_id = data::id(n + 1);
        request.catalog.account_id = AccountId(data::id(n).0);
        request.ownership.account_id = request.catalog.account_id;
        request.catalog.account_class = class;
        request.catalog.normal_sign = sign;
        request.origin = AccountOriginV1::Internal(InternalAccountV1 {
            role,
            created_at: data::date("2026-01-01"),
        });
        submit(
            f,
            u128::from(n) + 1000,
            OperationV1::SetAccountCatalogComplete(request),
        )
        .await;
        refresh(f, path, empty.clone()).await;
    }
}
fn binding(n: u32, value: i128) -> SourceFacetBindingV1 {
    SourceFacetBindingV1 {
        observation: data::id(n),
        parse_product: data::id(n + 1),
        facet: data::id(900),
        amount: data::amount(value),
    }
}
#[tokio::test]
async fn cur_013_1_service_pending_posted_reopen_and_no_double_money() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recognition.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    let before = f.prefix().await;
    let original = refresh(&mut f, &path, empty.clone()).await;
    let original_receipts: Vec<_> = [202, 212]
        .into_iter()
        .map(|n| {
            let record = original
                .records
                .get(&data::id(n))
                .expect("S1 must retain actual import receipts");
            assert!(matches!(record, AuthoritativeRecordV1::ImportReceipt(_)));
            (data::id(n), record.clone())
        })
        .collect();
    let pending = data::pending(binding(200, -80), 300);
    submit(&mut f, 201, OperationV1::AdoptActivity(pending)).await;
    assert_eq!(f.prefix().await.native_head, before.native_head);
    let state = refresh(&mut f, &path, empty.clone()).await;
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
    assert!(state.accounting.as_ref().unwrap().recognitions.is_empty());
    let mut posted = data::posted(binding(210, -96), 400, 30);
    posted.prior_pending = Some(data::id(300));
    let mut preview_body = f.body(202);
    preview_body.expected_head = f.prefix().await.through;
    preview_body.operation = OperationV1::AdoptActivity(posted.clone());
    let preview = f.service.preview(f.ctx(), preview_body).unwrap();
    assert!(preview.effects.amounts.contains(&data::amount(-96)));
    assert!(preview.effects.amounts.contains(&data::amount(96)));
    submit(&mut f, 202, OperationV1::AdoptActivity(posted.clone())).await;
    let accepted = f.prefix().await;
    assert_ne!(accepted.native_head, before.native_head);
    let state = refresh(&mut f, &path, empty.clone()).await;
    assert!(RecognitionEngine::pending(&state).unwrap().is_empty());
    let entries = &state.accounting.as_ref().unwrap().recognitions;
    assert_eq!(entries.len(), 1);
    let entry = &entries[&RecognitionId(data::id(403).0)][0];
    assert_eq!(entry.postings.len(), 2);
    assert_eq!(entry.postings[0].original.amount, data::amount(-96));
    for record in [200, 201, 210, 211, 300, 301, 302] {
        assert!(state.records.contains_key(&data::id(record)));
    }
    append_later_enrichment(&mut f, &path, empty.clone()).await;
    let after_enrichment = f.prefix().await;
    assert_eq!(after_enrichment.native_head, accepted.native_head);
    let accepted = after_enrichment;
    for request in [
        posted,
        data::posted(binding(210, -96), 500, 30),
        data::posted(binding(220, -96), 600, 30),
    ] {
        let mut body = f.body(203);
        body.expected_head = accepted.through;
        body.operation = OperationV1::AdoptActivity(request);
        assert!(f.service.preview(f.ctx(), body).is_err());
        assert_eq!(f.prefix().await, accepted);
    }
    // All authority and original source content survive a second encrypted reopen.
    let reopened = refresh(&mut f, &path, empty).await;
    assert!(RecognitionEngine::pending(&reopened).unwrap().is_empty());
    assert_eq!(reopened.accounting.as_ref().unwrap().recognitions.len(), 1);
    for (id, receipt) in original_receipts {
        assert_eq!(reopened.records.get(&id), Some(&receipt));
    }
    assert!(matches!(
        reopened.records.get(&data::id(222)),
        Some(AuthoritativeRecordV1::ImportReceipt(_))
    ));
    for record in [200, 201, 210, 211, 220, 221] {
        assert!(reopened.records.contains_key(&data::id(record)));
    }
    f.service.close().await.unwrap();
}

#[tokio::test]
async fn cur_013_2_service_corroboration_is_one_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corroboration.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    let mut request = data::posted(binding(210, -96), 400, 30);
    let corroboration = binding(230, -96);
    request.legs[0].sources.push(corroboration.clone());
    request.legs[0].leg.allocations.push(EvidenceAllocationV1 {
        observation: corroboration.observation,
        facet: corroboration.facet,
        amount: corroboration.amount,
    });
    submit(&mut f, 301, OperationV1::AdoptActivity(request.clone())).await;
    let state = refresh(&mut f, &path, empty).await;
    assert_eq!(state.accounting.as_ref().unwrap().recognitions.len(), 1);
    let entry =
        &state.accounting.as_ref().unwrap().recognitions[&RecognitionId(data::id(403).0)][0];
    assert_eq!(entry.postings[0].original.amount, data::amount(-96));
    let before = f.prefix().await;
    let mut same_leg = data::posted(binding(210, -96), 500, 30);
    same_leg.legs[0].leg.id = request.legs[0].leg.id;
    same_leg.event.legs = request.event.legs;
    for request in [same_leg, data::posted(corroboration, 600, 30)] {
        let mut body = f.body(302);
        body.expected_head = before.through;
        body.operation = OperationV1::AdoptActivity(request);
        assert!(f.service.preview(f.ctx(), body).is_err());
        assert_eq!(f.prefix().await, before);
    }
    f.service.close().await.unwrap();
}
#[tokio::test]
async fn cur_013_3_service_unknown_and_supported_openings_are_distinct() {
    for supported in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opening.db");
        let (mut f, empty) = bootstrap(&path).await;
        declare_activity_accounts(&mut f, &path, &empty).await;
        let mut request = data::posted(
            binding(if supported { 260 } else { 250 }, 3300),
            700,
            if supported { 50 } else { 40 },
        );
        let terms = request.financial.as_mut().unwrap();
        if supported {
            terms.opening = Some(OpeningBasisV1 {
                segment_id: data::id(707),
                component_id: data::id(11),
                amount: data::amount(3300),
                cutoff: data::date("2026-11-01"),
                evidence: binding(260, 3300),
                included_activity: vec![binding(210, -96)],
            });
        } else {
            terms.unresolved_account = Some(AccountId(data::id(40).0));
        }
        submit(&mut f, 401, OperationV1::AdoptActivity(request)).await;
        let state = refresh(&mut f, &path, empty).await;
        let entries = &state.accounting.as_ref().unwrap().recognitions;
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[&RecognitionId(data::id(703).0)][0].postings[0]
                .original
                .amount,
            data::amount(3300)
        );
        let Some(AuthoritativeRecordV1::AccountComponent(component)) =
            state.records.get(&data::id(11))
        else {
            panic!()
        };
        assert_eq!(component.opening_boundary, SourceValue::Unknown);
        assert_eq!(
            state
                .records
                .values()
                .any(|r| matches!(r, AuthoritativeRecordV1::AccountingSegment(_))),
            supported
        );
        assert!(RecognitionEngine::audit(&state).is_ok());
        if supported {
            let before = f.prefix().await;
            let mut body = f.body(402);
            body.expected_head = before.through;
            body.operation = OperationV1::AdoptActivity(data::posted(binding(210, -96), 800, 30));
            assert!(f.service.preview(f.ctx(), body).is_err());
            assert_eq!(f.prefix().await, before);
        }
        f.service.close().await.unwrap();
    }
}
#[tokio::test]
async fn cur_013_4_service_invalid_money_never_changes_domain_or_native_head() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid-money.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    let before = f.prefix().await;
    let valid = data::posted(binding(210, -96), 400, 30);
    let mut invalid = Vec::new();
    let mut r = valid.clone();
    r.financial.as_mut().unwrap().postings.pop();
    invalid.push(r);
    let mut r = valid.clone();
    r.financial.as_mut().unwrap().postings.clear();
    invalid.push(r);
    let mut r = valid.clone();
    r.financial.as_mut().unwrap().postings[1].amount = data::amount(95);
    invalid.push(r);
    let mut r = valid.clone();
    r.financial.as_mut().unwrap().postings[0].amount =
        Amount::new(-96001, 3, UnitId(data::id(1).0)).unwrap();
    invalid.push(r);
    let mut r = valid.clone();
    r.financial.as_mut().unwrap().postings[0].amount.unit_id = UnitId(data::id(999).0);
    invalid.push(r);
    let mut r = valid.clone();
    r.financial.as_mut().unwrap().bookkeeping_date = TemporalValue::Unknown {
        original_text: "absent".into(),
    };
    invalid.push(r);
    let mut r = valid.clone();
    r.financial.as_mut().unwrap().date_policy = PolicyReferenceV1 {
        id: data::id(104),
        version: 1,
    };
    invalid.push(r);
    let mut r = data::posted(binding(260, 3300), 700, 50);
    r.financial.as_mut().unwrap().opening = None;
    invalid.push(r);
    for request in invalid {
        let mut body = f.body(501);
        body.expected_head = before.through;
        body.operation = OperationV1::AdoptActivity(request);
        assert!(f.service.preview(f.ctx(), body.clone()).is_err());
        assert!(f.service.submit(f.ctx(), body, f.auth(None)).await.is_err());
        assert_eq!(f.prefix().await, before);
    }
    // Positive control: the accepted shape still executes after every rejected request.
    submit(&mut f, 502, OperationV1::AdoptActivity(valid)).await;
    let state = refresh(&mut f, &path, empty).await;
    assert_eq!(state.accounting.as_ref().unwrap().recognitions.len(), 1);
    f.service.close().await.unwrap();
}

#[tokio::test]
async fn persisted_catalog_and_adoption_references_reject_wrong_kind_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reference-closure.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    let before = f.prefix().await;
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let state = snapshot(&store, empty).await;
    let Some(AuthoritativeRecordV1::AccountCatalogRevision(mut revision)) =
        state.records.get(&data::id(32)).cloned()
    else {
        panic!()
    };
    revision.id = data::id(33);
    revision.prior_revision = Some(data::id(12)); // another component's revision
    let mut groups = vec![vec![AuthoritativeRecordV1::AccountCatalogRevision(
        revision,
    )]];
    let request = data::pending(binding(230, -96), 500);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    let mut records = plan.records.clone();
    for record in &mut records {
        if let AuthoritativeRecordV1::ActivityAdoption(a) = record {
            a.governing_policy.id = data::id(103);
        }
    }
    groups.push(records);
    let mut records = plan.records;
    for record in &mut records {
        if let AuthoritativeRecordV1::ActivityAdoption(a) = record {
            a.schema_version = 2;
        }
    }
    groups.push(records);
    for (i, records) in groups.into_iter().enumerate() {
        let result = store
            .writer()
            .append(HistoryAppend {
                prior_head: before.through,
                records,
                decisions: vec![],
                staged: vec![],
                native: None,
                result: CommandResultV1 {
                    command_id: CommandId(support::id(7000 + i as u128).0),
                    digest: Digest([77; 32]),
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
            })
            .await;
        assert!(result.is_err());
        assert_eq!(store.writer().audited_prefix().await.unwrap(), before);
    }
    store.close().await.unwrap();
}

async fn append_later_enrichment(f: &mut Fixture, path: &Path, empty: NativeHead) {
    f.service.close().await.unwrap();
    let store = open(path).await;
    let before = store.writer().audited_prefix().await.unwrap();
    let mut input = DomainState::default();
    data::evidence(&mut input, 220, -96, Some(data::id(210)));
    let staged = store.stage(evidence_bytes(220).as_bytes()).await.unwrap();
    let Some(AuthoritativeRecordV1::ObservationVersion(o)) = input.records.get_mut(&data::id(220))
    else {
        panic!()
    };
    o.content_digest = staged.digest();
    o.sanitized_representation = evidence_bytes(220);
    let receipt = synthetic_receipt(220, o);
    input.records.insert(data::id(222), receipt);
    store
        .writer()
        .append(HistoryAppend {
            prior_head: before.through,
            records: input.records.into_values().collect(),
            decisions: vec![],
            staged: vec![staged],
            native: None,
            result: CommandResultV1 {
                command_id: CommandId(support::id(9000).0),
                digest: Digest([19; 32]),
                state: CommandState::Committed,
                commit: None,
                effects: EffectsV1 {
                    records: vec![data::id(220), data::id(221)],
                    accounts: vec![],
                    amounts: vec![],
                    invalidations: vec![],
                },
                health: vec![],
            },
        })
        .await
        .unwrap();
    assert_eq!(
        store.writer().audited_prefix().await.unwrap().native_head,
        before.native_head
    );
    store.close().await.unwrap();
    f.service = f.reopen().await;
    refresh(f, path, empty).await;
}

#[tokio::test]
async fn historical_adoption_keeps_exact_policy_versions_after_later_policy_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("policy-history.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    submit(
        &mut f,
        601,
        OperationV1::AdoptActivity(data::posted(binding(210, -96), 400, 30)),
    )
    .await;
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let state = snapshot(&store, empty.clone()).await;
    assert!(RecognitionEngine::audit(&state).is_ok());
    let prefix = store.writer().audited_prefix().await.unwrap();
    let Some(AuthoritativeRecordV1::DatasetPolicy(mut policy)) =
        state.records.get(&data::id(104)).cloned()
    else {
        panic!()
    };
    let Some(AuthoritativeRecordV1::DateConversionPolicy(mut date)) =
        state.records.get(&data::id(103)).cloned()
    else {
        panic!()
    };
    policy.version = 2;
    policy.freshness_seconds = CommitSeq(2);
    date.version = 2;
    date.bookkeeping = BookkeepingFallbackV1::Explicit;
    // Exercise existing immutable policy-version history, not a new policy-editing command.
    store
        .writer()
        .append(HistoryAppend {
            prior_head: prefix.through,
            records: vec![
                AuthoritativeRecordV1::DatasetPolicy(policy),
                AuthoritativeRecordV1::DateConversionPolicy(date),
            ],
            decisions: vec![],
            staged: vec![],
            native: None,
            result: CommandResultV1 {
                command_id: CommandId(support::id(9100).0),
                digest: Digest([23; 32]),
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
        })
        .await
        .unwrap();
    assert_eq!(
        store.writer().audited_prefix().await.unwrap().native_head,
        prefix.native_head
    );
    store.close().await.unwrap();
    let store = open(&path).await;
    let mut state = snapshot(&store, empty).await;
    assert_eq!(
        state.dataset_policies[&(data::id(104), 1)].freshness_seconds,
        CommitSeq(1)
    );
    assert_eq!(
        state.dataset_policies[&(data::id(104), 2)].freshness_seconds,
        CommitSeq(2)
    );
    assert_eq!(
        state.accounting.as_ref().unwrap().recognitions[&RecognitionId(data::id(403).0)][0]
            .date_policy
            .version,
        1
    );
    assert!(RecognitionEngine::audit(&state).is_ok());
    let Some(AuthoritativeRecordV1::ActivityAdoption(a)) = state.records.get_mut(&data::id(400))
    else {
        panic!()
    };
    a.governing_policy.version = 3;
    assert!(RecognitionEngine::audit(&state).is_err());
    store.close().await.unwrap();
}

#[tokio::test]
async fn coherent_catalog_corruption_cannot_activate_replay_projection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid-replay.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let before = store.writer().audited_prefix().await.unwrap();
    let state = snapshot(&store, empty).await;
    let mut revision = CatalogEngine::current(&state, data::id(11)).unwrap();
    revision.id = data::id(13);
    revision.prior_revision = Some(data::id(12));
    revision.origin = AccountOriginV1::Internal(InternalAccountV1 {
        role: ControlRoleV1::ExpenseFlow,
        created_at: data::date("2026-01-01"),
    });
    // Ordinary typed storage preserves a coherent hash chain and references;
    // economic role/identity rejection belongs to the delivered C08 replay owner.
    store
        .writer()
        .append(HistoryAppend {
            prior_head: before.through,
            records: vec![AuthoritativeRecordV1::AccountCatalogRevision(revision)],
            decisions: vec![],
            staged: vec![],
            native: None,
            result: CommandResultV1 {
                command_id: CommandId(support::id(99000).0),
                digest: Digest([29; 32]),
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
        })
        .await
        .unwrap();
    assert_eq!(
        store.writer().audited_prefix().await.unwrap().native_head,
        before.native_head
    );
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let destination = open(&dir.path().join("rejected-shadow.db")).await;
    assert!(f.service.recover_into(destination).await.is_err());
    assert!(f.service.verified_projection().is_err());
    f.service.close().await.unwrap();
}

fn anchor_opening_request() -> ActivityRequestV1 {
    let mut request = data::posted(binding(260, 3300), 700, 50);
    request.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: data::id(707),
        component_id: data::id(11),
        amount: data::amount(3300),
        cutoff: data::date("2026-11-01"),
        evidence: binding(260, 3300),
        included_activity: vec![binding(210, -96)],
    });
    request
}

async fn reject_anchor_dates(f: &mut Fixture, request: ActivityRequestV1, n: u128) {
    let before = f.prefix().await;
    let mut body = f.body(n);
    body.expected_head = before.through;
    body.operation = OperationV1::AdoptActivity(request);
    assert!(
        f.service.preview(f.ctx(), body.clone()).is_err(),
        "mismatched native posting dates must not receive an approvable preview"
    );
    assert!(f.service.submit(f.ctx(), body, f.auth(None)).await.is_err());
    assert_eq!(f.prefix().await, before);
}

#[tokio::test]
async fn anchor_posting_date_service_opening_rejects_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("anchor-opening-dates.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    let before = f.prefix().await;
    for (i, day) in ["2026-10-31", "2026-11-02"].into_iter().enumerate() {
        let mut bad = anchor_opening_request();
        for p in &mut bad.financial.as_mut().unwrap().postings {
            p.effective = data::date(day);
        }
        reject_anchor_dates(&mut f, bad, 410 + i as u128).await;
    }
    refresh(&mut f, &path, empty.clone()).await;
    assert_eq!(f.prefix().await, before);
    submit(
        &mut f,
        420,
        OperationV1::AdoptActivity(anchor_opening_request()),
    )
    .await;
    let accepted = f.prefix().await;
    let state = refresh(&mut f, &path, empty).await;
    assert_eq!(f.prefix().await, accepted);
    assert_eq!(state.accounting.as_ref().unwrap().recognitions.len(), 1);
    f.service.close().await.unwrap();
}

#[tokio::test]
async fn anchor_posting_date_service_activity_rejects_before_or_at_cutoff() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("anchor-activity-dates.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    submit(
        &mut f,
        430,
        OperationV1::AdoptActivity(anchor_opening_request()),
    )
    .await;
    refresh(&mut f, &path, empty.clone()).await;
    let before = f.prefix().await;
    let mut valid = data::posted(binding(230, -96), 800, 30);
    valid.legs[0].leg.effective = data::date("2026-11-02");
    for p in &mut valid.financial.as_mut().unwrap().postings {
        p.effective = data::date("2026-11-02");
    }
    for (i, day) in ["2026-10-31", "2026-11-01"].into_iter().enumerate() {
        let mut bad = valid.clone();
        for p in &mut bad.financial.as_mut().unwrap().postings {
            p.effective = data::date(day);
        }
        reject_anchor_dates(&mut f, bad, 440 + i as u128).await;
    }
    refresh(&mut f, &path, empty.clone()).await;
    assert_eq!(f.prefix().await, before);
    submit(&mut f, 450, OperationV1::AdoptActivity(valid)).await;
    let accepted = f.prefix().await;
    let state = refresh(&mut f, &path, empty).await;
    assert_eq!(f.prefix().await, accepted);
    assert_eq!(state.accounting.as_ref().unwrap().recognitions.len(), 2);
    f.service.close().await.unwrap();
}

#[derive(Clone, Copy)]
enum CatalogBatchCase {
    SiblingFork,
    AliasCollision,
    Independent,
}

async fn catalog_batch_recovery(case: CatalogBatchCase) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog-batch.db");
    let (mut f, empty) = bootstrap(&path).await;
    declare_activity_accounts(&mut f, &path, &empty).await;
    // There are catalog declarations but no activity/adoption records.
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let before = store.writer().audited_prefix().await.unwrap();
    let state = snapshot(&store, empty).await;
    assert!(
        !state
            .records
            .values()
            .any(|r| matches!(r, AuthoritativeRecordV1::ActivityAdoption(_)))
    );
    let mut first = CatalogEngine::current(&state, data::id(11)).unwrap();
    first.prior_revision = Some(first.id);
    first.id = data::id(98001);
    first.catalog.display_name = "first update".into();
    let mut second = if matches!(case, CatalogBatchCase::SiblingFork) {
        first.clone()
    } else {
        let mut r = CatalogEngine::current(&state, data::id(31)).unwrap();
        r.prior_revision = Some(r.id);
        r
    };
    second.id = data::id(98002);
    second.catalog.display_name = "second update".into();
    if matches!(case, CatalogBatchCase::AliasCollision) {
        first.catalog.aliases.push(SourceAliasV1 {
            source: SourceIdentityV1 {
                source: "synthetic".into(),
                connection_id: data::id(102),
                device_id: None,
            },
            original_id: "new-shared-alias".into(),
        });
        second.catalog.aliases.push(SourceAliasV1 {
            source: SourceIdentityV1 {
                source: "synthetic".into(),
                connection_id: data::id(102),
                device_id: None,
            },
            original_id: "new-shared-alias".into(),
        });
    }
    // Each update is valid on the exact prior prefix. Only their combination
    // violates catalog uniqueness; hashes, references and native state are valid.
    for revision in [&first, &second] {
        let Some(AuthoritativeRecordV1::AccountComponent(root)) =
            state.records.get(&revision.component_id)
        else {
            panic!()
        };
        CatalogEngine::set_complete(
            &state,
            &CatalogRequestV1 {
                schema_version: revision.schema_version,
                revision_id: revision.id,
                prior_revision: revision.prior_revision,
                catalog: revision.catalog.clone(),
                ownership: root.ownership.clone(),
                origin: revision.origin.clone(),
            },
        )
        .unwrap();
    }
    store
        .writer()
        .append(HistoryAppend {
            prior_head: before.through,
            records: vec![
                AuthoritativeRecordV1::AccountCatalogRevision(first.clone()),
                AuthoritativeRecordV1::AccountCatalogRevision(second.clone()),
            ],
            decisions: vec![],
            staged: vec![],
            native: None,
            result: CommandResultV1 {
                command_id: CommandId(support::id(98000).0),
                digest: Digest([31; 32]),
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
        })
        .await
        .unwrap();
    let tip = store.writer().audited_prefix().await.unwrap();
    assert_eq!(tip.native_head, before.native_head);
    assert!(store.history().read_prefix(tip.through).await.is_ok());
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let destination = open(&dir.path().join("shadow.db")).await;
    let result = f.service.recover_into(destination).await;
    if matches!(case, CatalogBatchCase::Independent) {
        result.unwrap();
        let projection = f.service.verified_projection().unwrap();
        assert_eq!(
            CatalogEngine::current(&projection, first.component_id).unwrap(),
            first
        );
        assert_eq!(
            CatalogEngine::current(&projection, second.component_id).unwrap(),
            second
        );
    } else {
        assert!(
            result.is_err(),
            "combined invalid catalog must not activate recovery"
        );
        assert!(f.service.verified_projection().is_err());
    }
    f.service.close().await.unwrap();
}

#[tokio::test]
async fn catalog_batch_sibling_fork_cannot_activate_recovery() {
    catalog_batch_recovery(CatalogBatchCase::SiblingFork).await;
}
#[tokio::test]
async fn catalog_batch_alias_collision_cannot_activate_recovery() {
    catalog_batch_recovery(CatalogBatchCase::AliasCollision).await;
}
#[tokio::test]
async fn catalog_batch_independent_revisions_recover() {
    catalog_batch_recovery(CatalogBatchCase::Independent).await;
}
