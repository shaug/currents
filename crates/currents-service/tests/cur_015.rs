//! Actual synthetic relationship input through the accepted trusted evidence seam.
use currents_service as service_api;

use currents_service::ImportEvidence;
use currents_types::*;
fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
fn relationship_capture() -> RelationshipCaptureBatchV1 {
    use currents_acquisition::{capture::BoundedEvidence, identity::RelationshipSourceParser};
    let context = ImportContextV1 {
        receipt_id: id(100),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(101),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
        coverage: vec![],
        row_ids: vec![],
    };
    RelationshipSourceParser::capture(
        BoundedEvidence::new(
            "relationship.json",
            include_bytes!("../../../fixtures/journeys/s3/relationship-source.json").to_vec(),
            context,
        )
        .unwrap(),
        id(102),
    )
    .unwrap()
}
#[test]
fn relationship_import_identity_binds_complete_retained_metadata() {
    let capture = relationship_capture();
    let evidence = ImportEvidence::new_relationship(capture.clone()).unwrap();
    let request = evidence.request();
    assert_eq!(request.receipt, capture.receipt.id);
    assert_eq!(request.checkpoint, SourceValue::Absent);
    let plan = evidence.plan(&DomainState::default()).unwrap();
    assert_eq!(plan.records.len(), 2);
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    let encoded = serde_json::to_string(&evidence).unwrap();
    assert!(!encoded.contains("paystub-1"));
    assert!(!encoded.contains("5000"));
    let mut different = capture.clone();
    different.capture.original_capture_time =
        serde_json::from_str("\"2026-10-31T00:00:00Z\"").unwrap();
    assert_ne!(
        ImportEvidence::new_relationship(different)
            .unwrap()
            .request()
            .staged_batch_digest,
        request.staged_batch_digest
    );
    let mut forged = capture;
    forged.capture.rows[0].facets[0].amount = "9999".into();
    assert!(ImportEvidence::new_relationship(forged).is_err());
}

#[path = "support/lifecycle.rs"]
#[allow(dead_code)]
mod support;
struct RelationshipPlanner {
    state: DomainState,
    evidence: Option<ImportEvidence>,
}
impl currents_service::CommandPlanner for RelationshipPlanner {
    fn prepare(
        &self,
        body: &currents_protocol::CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<currents_service::CommandPlan, EngineError> {
        if self.state.accounting.as_ref().map(|v| &v.prefix) != Some(prefix) {
            return Err(EngineError::invalid());
        }
        let decision_plan = match &body.operation {
            currents_protocol::OperationV1::PublishImport(request) => {
                let evidence = self.evidence.as_ref().ok_or_else(EngineError::invalid)?;
                if *request != evidence.request() {
                    return Err(EngineError::invalid());
                }
                evidence.plan(&self.state)?
            }
            currents_protocol::OperationV1::SetAccountCatalogComplete(request) => {
                currents_domain::catalog::CatalogEngine::set_complete(&self.state, request)?
            }
            currents_protocol::OperationV1::AssociateLegsComplete(request) => {
                currents_domain::associations::IdentityEngine::associate(&self.state, request)?
            }
            currents_protocol::OperationV1::AdoptActivity(request) => {
                currents_domain::recognition::RecognitionEngine::adopt(&self.state, request)?
            }
            currents_protocol::OperationV1::ExpirePending(request) => {
                currents_domain::expiry::PendingExpiryEngine::expire(&self.state, request)?
            }
            _ => return Err(EngineError::version()),
        };
        let frozen_batch =
            currents_accounting::AccountingAdapter::freeze(&decision_plan.accounting, &self.state)?;
        let records = decision_plan
            .records
            .iter()
            .map(|r| {
                serde_json::from_value(serde_json::to_value(r).unwrap()["value"]["id"].clone())
                    .unwrap()
            })
            .collect();
        let mut accounts = Vec::new();
        let mut amounts = Vec::new();
        for op in &frozen_batch.operations {
            match op {
                FrozenNativeOperationV1::Account(a) => accounts.push(a.account_id),
                FrozenNativeOperationV1::Entry(e) => {
                    for p in &e.postings {
                        accounts.push(p.original.account_id);
                        amounts.push(p.original.amount);
                    }
                }
                _ => {}
            }
        }
        // Metadata-only changes still disclose the affected financial account.
        for record in &decision_plan.records {
            match record {
                AuthoritativeRecordV1::AccountCatalogRevision(r) => accounts.push(r.account_id),
                AuthoritativeRecordV1::FinancialLeg(leg) => accounts.push(leg.account_id),
                AuthoritativeRecordV1::PendingExpiry(expiry) => {
                    let Some(AuthoritativeRecordV1::ActivityAdoption(pending)) =
                        self.state.records.get(&expiry.pending_adoption)
                    else {
                        return Err(EngineError::invalid());
                    };
                    for leg in &pending.legs {
                        let Some(AuthoritativeRecordV1::FinancialLeg(leg)) =
                            self.state.records.get(&RecordId(leg.0))
                        else {
                            return Err(EngineError::invalid());
                        };
                        accounts.push(leg.account_id);
                    }
                }
                _ => {}
            }
        }
        accounts.sort();
        accounts.dedup();
        let policy = self
            .state
            .dataset_policies
            .values()
            .next()
            .ok_or_else(EngineError::invalid)?;
        let invalidations = decision_plan.invalidations.clone();
        Ok(currents_service::CommandPlan {
            import: self.evidence.clone(),
            decision_plan,
            frozen_batch,
            effects: EffectsV1 {
                records,
                accounts: accounts.clone(),
                amounts,
                invalidations,
            },
            warnings: vec![],
            approval_scope: currents_protocol::ApprovalScopeV1 {
                accounts,
                operation: match &body.operation {
                    currents_protocol::OperationV1::SetAccountCatalogComplete(_) => {
                        DecisionKind::Catalog
                    }
                    currents_protocol::OperationV1::AdoptActivity(_) => DecisionKind::Recognition,
                    _ => DecisionKind::Interpretation,
                },
                fields: vec!["relationship-source".into()],
            },
            decision_id: id(body.command_id.0.as_u128() + 200_000),
            manifest_id: id(body.command_id.0.as_u128() + 300_000),
            policy: PolicyReferenceV1 {
                id: policy.id,
                version: policy.version,
            },
        })
    }
    fn query(
        &self,
        _: &currents_protocol::QueryV1,
        _: &AccountingPrefixV1,
    ) -> Result<currents_protocol::QueryResponseV1, EngineError> {
        Err(EngineError::version())
    }
}
#[tokio::test]
async fn relationship_capture_uses_real_service_staging_and_reopens_exactly() {
    use currents_cipher::{CipherProfileV1, DatasetKey};
    use currents_service::{Service, ServiceProfile};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relationship-service.db");
    let mut f = support::Fixture::at_path(path.clone()).await;
    let bootstrap = f.body(10000);
    f.service
        .submit(f.ctx(), bootstrap.clone(), f.approve(&bootstrap))
        .await
        .unwrap();
    support::recover_service(&f.service).await;
    let state = f.service.verified_projection().unwrap();
    f.service.close().await.unwrap();
    f.host.reconnect();
    let capture = relationship_capture();
    let evidence = ImportEvidence::new_relationship(capture.clone()).unwrap();
    let request = evidence.request();
    let store =
        currents_storage::EncryptedStore::open(&path, DatasetKey::new([0x10; 32]), CipherProfileV1)
            .await
            .unwrap();
    f.service = Service::new(
        store,
        DatasetId(support::id(1000).0),
        InstanceId(support::id(999).0),
        ServiceProfile::Synthetic,
        f.host.clone(),
        std::sync::Arc::new(RelationshipPlanner {
            state,
            evidence: Some(evidence),
        }),
        f.clock.clone(),
    )
    .await
    .unwrap();
    support::recover_service(&f.service).await;
    let before = f.prefix().await;
    let mut body = f.body(10001);
    body.expected_head = before.through;
    body.operation = currents_protocol::OperationV1::PublishImport(request);
    let result = f
        .service
        .submit(f.ctx(), body.clone(), f.approve(&body))
        .await
        .unwrap();
    assert_eq!(
        f.prefix().await.native_head,
        before.native_head,
        "capture is not recognition"
    );
    support::recover_service(&f.service).await;
    let verified = f.service.verified_projection().unwrap();
    assert_eq!(
        verified.records.get(&capture.capture.id),
        Some(&AuthoritativeRecordV1::RelationshipCapture(
            capture.capture.clone()
        ))
    );
    f.service.close().await.unwrap();
    f.service = f.reopen().await;
    support::recover_service(&f.service).await;
    assert_eq!(
        f.service
            .submit(f.ctx(), body.clone(), f.auth(None))
            .await
            .unwrap(),
        result
    );
    assert_eq!(
        f.service
            .verified_projection()
            .unwrap()
            .records
            .get(&capture.capture.id),
        Some(&AuthoritativeRecordV1::RelationshipCapture(
            capture.capture.clone()
        ))
    );
    f.service.close().await.unwrap();
    let store =
        currents_storage::EncryptedStore::open(&path, DatasetKey::new([0x10; 32]), CipherProfileV1)
            .await
            .unwrap();
    let mut stream = store
        .history()
        .stream_content(result.commit.unwrap(), capture.capture.raw.digest)
        .await
        .unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await.unwrap() {
        bytes.extend(chunk);
    }
    drop(stream);
    assert_eq!(bytes, capture.contents[0].bytes);
    store.close().await.unwrap();
}

#[path = "../../currents-domain/tests/support/associations.rs"]
#[allow(dead_code)]
mod association_fixtures;
#[path = "../../currents-domain/tests/support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use currents_accounting::AccountingAdapter;
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_storage::{EncryptedStore, HistoryAppend};
use std::path::Path;
use support::Fixture;
async fn open(path: &Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x10; 32]), CipherProfileV1)
        .await
        .unwrap()
}
async fn refresh_financial(
    f: &mut Fixture,
    path: &Path,
    evidence: Option<ImportEvidence>,
) -> DomainState {
    support::recover_service(&f.service).await;
    let state = f.service.verified_projection().unwrap();
    f.service.close().await.unwrap();
    f.host.reconnect();
    f.service = currents_service::Service::new(
        open(path).await,
        DatasetId(support::id(1000).0),
        InstanceId(support::id(999).0),
        currents_service::ServiceProfile::Synthetic,
        f.host.clone(),
        std::sync::Arc::new(RelationshipPlanner {
            state: state.clone(),
            evidence,
        }),
        f.clock.clone(),
    )
    .await
    .unwrap();
    support::recover_service(&f.service).await;
    state
}
async fn submit(
    f: &mut Fixture,
    n: u128,
    operation: currents_protocol::OperationV1,
) -> CommandResultV1 {
    let mut body = f.body(n);
    body.expected_head = f.prefix().await.through;
    body.operation = operation;
    let preview = f
        .service
        .preview(f.ctx(), body.clone())
        .unwrap_or_else(|e| panic!("synthetic preview command {n}: {e:?}"));
    if matches!(
        body.operation,
        currents_protocol::OperationV1::ExpirePending(_)
    ) {
        assert_eq!(
            preview.effects.accounts,
            vec![AccountId(fixture::id(10).0)],
            "expiry must disclose its affected account despite no native operation"
        );
    }
    if let currents_protocol::OperationV1::SetAccountCatalogComplete(ref request) = body.operation {
        assert!(
            preview
                .effects
                .accounts
                .contains(&request.catalog.account_id),
            "catalog-only changes retain their account authorization scope"
        );
    }
    f.service
        .submit(f.ctx(), body.clone(), f.approve(&body))
        .await
        .unwrap_or_else(|e| panic!("synthetic submit command {n}: {e:?}"))
}

async fn bootstrap(path: &Path) -> (Fixture, NativeHead) {
    bootstrap_with_balance(path, false).await
}
async fn bootstrap_with_balance(path: &Path, balance: bool) -> (Fixture, NativeHead) {
    let f = Fixture::at_path(path.to_path_buf()).await;
    let empty = f.prefix().await.native_head;
    f.service.close().await.unwrap();
    let store = open(path).await;
    let mut fixture = fixture::monetary_state();
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
    if balance {
        // Trusted synthetic history, as in CUR013; the file adapter only emits movements.
        fixture::evidence(&mut fixture, 250, -96, None);
        let bytes = "synthetic opening balance";
        let content = store.stage(bytes.as_bytes()).await.unwrap();
        let Some(AuthoritativeRecordV1::ObservationVersion(o)) =
            fixture.records.get_mut(&fixture::id(250))
        else {
            panic!()
        };
        o.content_digest = content.digest();
        o.sanitized_representation = bytes.into();
        o.semantic_kind = ObservationKind::Balance;
        records.push(AuthoritativeRecordV1::ImportReceipt(ImportReceiptV1 {
            id: fixture::id(252),
            source: o.source.clone(),
            adapter_version: "cur015-synthetic-history/1".into(),
            parser_version: "1".into(),
            requested_coverage: vec![],
            reported_coverage: vec![],
            received_time: serde_json::from_str("\"2026-11-01T12:00:00Z\"").unwrap(),
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![ContentReferenceV1 {
                digest: content.digest(),
                byte_count: CommitSeq(bytes.len() as u64),
                media_type: "text/plain".into(),
            }],
        }));
        let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) =
            fixture.records.get_mut(&fixture::id(251))
        else {
            panic!()
        };
        p.product.facets[0].kind = ObservationKind::Balance;
        records.push(fixture.records[&fixture::id(250)].clone());
        records.push(fixture.records[&fixture::id(251)].clone());
        staged.push(content);
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
        AccountingAdapter::manifest(fixture::id(990), &batch).unwrap(),
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
    refresh_financial(&mut f, path, None).await;
    (f, empty)
}

type EconomicEntry = (RecognitionId, String, String, Vec<FrozenPostingV1>);
async fn real_service_transfer(order: Option<[usize; 3]>, remap: bool) -> Vec<EconomicEntry> {
    real_service_transfer_mode(order, remap, false).await
}
async fn real_service_transfer_mode(
    order: Option<[usize; 3]>,
    remap: bool,
    parser_correction: bool,
) -> Vec<EconomicEntry> {
    use currents_protocol::OperationV1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("transfer.db");
    let (mut f, _) = bootstrap(&path).await;
    let (template, mut request) = association_fixtures::transfer_request();
    declare_template(&mut f, &path, &template).await;
    let rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(include_bytes!(
        "../../../fixtures/journeys/s2/relationship-source.json"
    ))
    .unwrap();
    let groups: Vec<(u32, Vec<RelationshipSourceRowV1>)> = match order {
        None => vec![(1500, rows.clone())],
        Some(order) => order
            .into_iter()
            .chain([3])
            .map(|i| (1500 + 10 * u32::try_from(i).unwrap(), vec![rows[i].clone()]))
            .collect(),
    };
    let unchanged_native = f.prefix().await.native_head;
    for (n, rows) in groups {
        let context = ImportContextV1 {
            receipt_id: fixture::id(n),
            source: SourceIdentityV1 {
                source: "synthetic".into(),
                connection_id: fixture::id(900),
                device_id: None,
            },
            captured_at: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
            received_at: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
            coverage: vec![],
            row_ids: vec![],
        };
        let capture = currents_acquisition::identity::RelationshipSourceParser::capture(
            currents_acquisition::capture::BoundedEvidence::new(
                "transfer.json",
                serde_json::to_vec(&rows).unwrap(),
                context,
            )
            .unwrap(),
            fixture::id(n + 1),
        )
        .unwrap();
        let evidence = ImportEvidence::new_relationship(capture).unwrap();
        let import = evidence.request();
        refresh_financial(&mut f, &path, Some(evidence)).await;
        submit(
            &mut f,
            12000 + u128::from(n),
            OperationV1::PublishImport(import),
        )
        .await;
        assert_eq!(
            f.prefix().await.native_head,
            unchanged_native,
            "each arrival alone is evidence, not money"
        );
    }
    if order.is_some() {
        request.revision.interpretation.evidence =
            (0..4).map(|i| fixture::id(1501 + 10 * i)).collect();
        for event in &mut request.events {
            event.interpretation = SourceValue::Known(request.revision.interpretation.clone());
        }
        for leg in &mut request.legs {
            leg.interpretation = SourceValue::Known(request.revision.interpretation.clone());
        }
        for edge in &mut request.revision.captured_allocations {
            let position = rows
                .iter()
                .position(|r| r.source_record == edge.source.source_record)
                .unwrap();
            edge.source.capture = fixture::id(1501 + 10 * u32::try_from(position).unwrap());
        }
    }
    let before = refresh_financial(&mut f, &path, None).await;
    let result = submit(
        &mut f,
        12001,
        OperationV1::AssociateLegsComplete(request.clone()),
    )
    .await;
    let mut after = refresh_financial(&mut f, &path, None).await;
    assert_eq!(after.through, result.commit.unwrap());
    assert_ne!(
        after.accounting.as_ref().unwrap().prefix.native_head,
        before.accounting.as_ref().unwrap().prefix.native_head
    );
    let index = after.accounting.as_ref().unwrap();
    let mut expense = fixture::amount(0);
    for recognition in &request.recognitions {
        let entries = &index.recognitions[&recognition.id];
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].postings[0].original.effective,
            recognition.postings[0].effective
        );
        for p in &entries[0].postings {
            if p.original.account_id == AccountId(fixture::id(30).0) {
                expense = expense.checked_add(&p.original.amount).unwrap();
            }
        }
    }
    assert_eq!(expense, fixture::amount(10));
    if remap {
        use currents_accounting::test_fixtures::{self, ClosureFault};
        let original = after.clone();
        let native_before = after
            .accounting
            .as_ref()
            .unwrap()
            .prefix
            .native_head
            .clone();
        let mut revised: Vec<RelationshipSourceRowV1> = serde_json::from_slice(include_bytes!(
            "../../../fixtures/journeys/s2/relationship-source.json"
        ))
        .unwrap();
        for (i, row) in revised.iter_mut().enumerate() {
            row.source_revision = "revised".into();
            row.prior_revision = Some("original".into());
            row.facets[0].amount = ["-1100", "700", "400", "-10"][i].into();
        }
        let mut next = association_fixtures::revised_transfer(&request, fixture::id(1701));
        if parser_correction {
            let Some(AuthoritativeRecordV1::RelationshipCapture(capture)) =
                after.records.get(&fixture::id(1501)).cloned()
            else {
                panic!()
            };
            let original_raw = capture.raw.clone();
            for (i, row) in rows.iter().take(2).enumerate() {
                let n = 1700 + u32::try_from(i).unwrap() * 10;
                let context = ImportContextV1 {
                    receipt_id: fixture::id(n),
                    source: capture.source.clone(),
                    captured_at: capture.original_capture_time.clone(),
                    received_at: serde_json::from_str("\"2026-11-06T00:00:00Z\"").unwrap(),
                    coverage: vec![],
                    row_ids: vec![],
                };
                let profile = RelationshipCorrectionProfileV1 {
                    schema_version: 1,
                    source_record: row.source_record.clone(),
                    source_revision: row.source_revision.clone(),
                    corrections: vec![RelationshipAmountCorrectionV1 {
                        facet: row.facets[0].lineage.clone(),
                        expected_amount: row.facets[0].amount.clone(),
                        amount: revised[i].facets[0].amount.clone(),
                    }],
                };
                let batch = currents_acquisition::identity::RelationshipSourceParser::correct(
                    &capture,
                    None,
                    serde_json::to_vec(&profile).unwrap(),
                    context,
                    fixture::id(n + 1),
                )
                .unwrap();
                assert_eq!(batch.revision.row.source_revision, "original");
                let evidence = ImportEvidence::new_relationship_parse(batch).unwrap();
                let import = evidence.request();
                refresh_financial(&mut f, &path, Some(evidence)).await;
                submit(
                    &mut f,
                    16000 + u128::from(n),
                    OperationV1::PublishImport(import),
                )
                .await;
                assert_eq!(f.prefix().await.native_head, native_before);
                next.revision.captured_allocations[i].source.parse_revision =
                    Some(fixture::id(n + 1));
            }
            for edge in &mut next.revision.captured_allocations {
                edge.source.capture = capture.id;
                edge.source.source_revision = "original".into();
            }
            next.revision.interpretation.evidence =
                vec![capture.id, fixture::id(1701), fixture::id(1711)];
            for event in &mut next.events {
                event.interpretation = SourceValue::Known(next.revision.interpretation.clone());
            }
            for leg in &mut next.legs {
                leg.interpretation = SourceValue::Known(next.revision.interpretation.clone());
            }
            let current = refresh_financial(&mut f, &path, None).await;
            assert_eq!(
                current.records.get(&capture.id),
                Some(&AuthoritativeRecordV1::RelationshipCapture(capture.clone()))
            );
            assert_eq!(capture.raw, original_raw);
            let stale = request.revision.captured_allocations[0].source.clone();
            assert!(
                currents_domain::associations::IdentityEngine::captured_facet(
                    &current,
                    &stale,
                    fixture::amount(1).unit_id
                )
                .is_err()
            );
        } else {
            publish_relationship_rows(&mut f, &path, 1700, revised.clone()).await;
        }
        assert_eq!(f.prefix().await.native_head, native_before);
        let mut body = f.body(14000);
        body.expected_head = f.prefix().await.through;
        body.operation = OperationV1::AssociateLegsComplete(next.clone());
        let before = f.prefix().await;
        let control = std::sync::Arc::new(test_fixtures::fault_control(
            ClosureFault::PauseBeforeCommit,
        ));
        f.service
            .set_test_control(body.command_id, control.clone())
            .unwrap();
        let caller = f.spawn(&body);
        support::wait_for(|| control.completed_operations() == 8).await;
        f.service.cancel(f.ctx(), body.command_id).await.unwrap();
        test_fixtures::release_control(&control);
        assert_eq!(
            caller.await.unwrap().unwrap_err().class,
            ErrorClass::Canceled
        );
        assert_eq!(f.prefix().await, before);
        // Settled canceled admissions intentionally prevent activation in the
        // same session. Exercise the documented close/reopen recovery boundary.
        f.service.close().await.unwrap();
        f.service = f.reopen().await;
        refresh_financial(&mut f, &path, None).await;
        let result = f
            .service
            .submit(f.ctx(), body.clone(), f.approve(&body))
            .await
            .unwrap();
        after = refresh_financial(&mut f, &path, None).await;
        for (id, record) in original.records {
            assert_eq!(after.records.get(&id), Some(&record));
        }
        let index = after.accounting.as_ref().unwrap();
        assert!(
            request
                .recognitions
                .iter()
                .all(|r| index.reversed.contains(&r.id))
        );
        assert!(
            next.recognitions
                .iter()
                .all(|r| !index.reversed.contains(&r.id))
        );
        assert_eq!(
            next.recognitions
                .iter()
                .map(|r| index.recognitions[&r.id][0].postings[0]
                    .original
                    .amount
                    .coefficient)
                .collect::<Vec<_>>(),
            vec![-1100, 700, 400, -10]
        );
        assert_eq!(
            f.service.submit(f.ctx(), body, f.auth(None)).await.unwrap(),
            result
        );
        let before_tombstone = f.prefix().await.native_head;
        for row in &mut revised {
            row.source_revision = "deleted".into();
            row.prior_revision = Some(
                if parser_correction {
                    "original"
                } else {
                    "revised"
                }
                .into(),
            );
            row.status = RelationshipSourceStatusV1::Tombstone;
            row.facets.clear();
        }
        publish_relationship_rows(&mut f, &path, 1800, revised).await;
        after = f.service.verified_projection().unwrap();
        assert_eq!(f.prefix().await.native_head, before_tombstone);
    }

    f.service.close().await.unwrap();
    f.service = f.reopen().await;
    support::recover_service(&f.service).await;
    assert_eq!(
        f.service.verified_projection().unwrap().records,
        after.records
    );
    f.service.close().await.unwrap();
    // Native documents bind the exact prior domain digest, so different arrival
    // histories deliberately have different authenticated heads. Compare exact
    // dated native postings, not their history-specific document identities.
    after
        .accounting
        .unwrap()
        .recognitions
        .into_iter()
        .flat_map(|(id, entries)| {
            entries
                .into_iter()
                .map(move |e| (id, e.booking_date, e.value_date, e.postings))
        })
        .collect()
}

#[tokio::test]
async fn cur_015_1_real_service_transfer_keeps_dates_and_only_separate_fee_is_spending() {
    real_service_transfer(None, false).await;
}
#[tokio::test]
async fn cur_015_1_separate_arrival_permutations_rebuild_same_economic_postings() {
    let mut expected = None;
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let head = real_service_transfer(Some(order), false).await;
        if let Some(old) = &expected {
            assert_eq!(&head, old);
        } else {
            expected = Some(head);
        }
    }
}

async fn declare_template(f: &mut Fixture, path: &Path, template: &DomainState) {
    use currents_protocol::OperationV1;
    let mut serial = 11000;
    for record in template.records.values() {
        if let AuthoritativeRecordV1::AccountCatalogRevision(r) = record {
            let Some(AuthoritativeRecordV1::AccountComponent(root)) =
                template.records.get(&r.component_id)
            else {
                panic!()
            };
            submit(
                f,
                serial,
                OperationV1::SetAccountCatalogComplete(CatalogRequestV1 {
                    schema_version: r.schema_version,
                    revision_id: r.id,
                    prior_revision: r.prior_revision,
                    catalog: r.catalog.clone(),
                    ownership: root.ownership.clone(),
                    origin: r.origin.clone(),
                }),
            )
            .await;
            serial += 1;
            refresh_financial(f, path, None).await;
        }
    }
}
async fn publish_relationship_rows(
    f: &mut Fixture,
    path: &Path,
    n: u32,
    rows: Vec<RelationshipSourceRowV1>,
) {
    let context = ImportContextV1 {
        receipt_id: fixture::id(n),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: fixture::id(900),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
        coverage: vec![],
        row_ids: vec![],
    };
    let capture = currents_acquisition::identity::RelationshipSourceParser::capture(
        currents_acquisition::capture::BoundedEvidence::new(
            "relationship.json",
            serde_json::to_vec(&rows).unwrap(),
            context,
        )
        .unwrap(),
        fixture::id(n + 1),
    )
    .unwrap();
    let evidence = ImportEvidence::new_relationship(capture).unwrap();
    let import = evidence.request();
    refresh_financial(f, path, Some(evidence)).await;
    submit(
        f,
        12000 + u128::from(n),
        currents_protocol::OperationV1::PublishImport(import),
    )
    .await;
    refresh_financial(f, path, None).await;
}
#[tokio::test]
async fn cur_015_2_real_service_payroll_retains_five_facets_and_owner_tax_choice() {
    use currents_protocol::OperationV1;
    for prepayment in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payroll.db");
        let (mut f, _) = bootstrap(&path).await;
        let (template, mut request) = association_fixtures::payroll_request();
        declare_template(&mut f, &path, &template).await;
        let rows = serde_json::from_slice(include_bytes!(
            "../../../fixtures/journeys/s3/relationship-source.json"
        ))
        .unwrap();
        publish_relationship_rows(&mut f, &path, 1600, rows).await;
        let before = f.prefix().await;
        let mut net = request.clone();
        net.legs = vec![request.legs[4].clone()];
        net.recognitions = vec![request.recognitions[4].clone()];
        net.revision.legs = vec![net.legs[0].id];
        net.revision.recognitions = vec![net.recognitions[0].id];
        net.revision.captured_allocations = vec![request.revision.captured_allocations[4].clone()];
        net.revision.residuals = vec![fixture::amount(3300)];
        net.events[0].legs = net.revision.legs.clone();
        let mut body = f.body(13000);
        body.expected_head = before.through;
        body.operation = OperationV1::AssociateLegsComplete(net);
        assert_eq!(
            f.service.preview(f.ctx(), body.clone()).unwrap_err().class,
            ErrorClass::InvalidReference
        );
        let mut unresolved = request.clone();
        unresolved.legs[1].account_id = AccountId(fixture::id(40).0);
        unresolved.recognitions[1].postings[0].account_id = unresolved.legs[1].account_id;
        body.operation = OperationV1::AssociateLegsComplete(unresolved);
        assert_eq!(
            f.service.preview(f.ctx(), body).unwrap_err().class,
            ErrorClass::InvalidReference
        );
        assert_eq!(f.prefix().await, before);
        if prepayment {
            request.legs[1].account_id = AccountId(fixture::id(120).0);
            request.recognitions[1].postings[0].account_id = request.legs[1].account_id;
            request.revision.interpretation.reason =
                "Explicit synthetic owner choice: tax prepayment, not tax expense".into();
            for leg in &mut request.legs {
                leg.interpretation = SourceValue::Known(request.revision.interpretation.clone());
            }
            for event in &mut request.events {
                event.interpretation = SourceValue::Known(request.revision.interpretation.clone());
            }
        }
        submit(
            &mut f,
            13001,
            OperationV1::AssociateLegsComplete(request.clone()),
        )
        .await;
        let state = refresh_financial(&mut f, &path, None).await;
        let index = state.accounting.as_ref().unwrap();
        assert_eq!(
            request
                .recognitions
                .iter()
                .map(|r| index.recognitions[&r.id][0].postings[0]
                    .original
                    .amount
                    .coefficient)
                .collect::<Vec<_>>(),
            vec![-5000, 1000, 500, 200, 3300]
        );
        assert_eq!(
            index.recognitions[&request.recognitions[1].id][0].postings[0]
                .original
                .account_id,
            AccountId(fixture::id(if prepayment { 120 } else { 30 }).0)
        );
        f.service.close().await.unwrap();
        f.service = f.reopen().await;
        support::recover_service(&f.service).await;
        assert_eq!(
            f.service.verified_projection().unwrap().records,
            state.records
        );
        f.service.close().await.unwrap();
    }
}

#[tokio::test]
async fn cur_015_3_real_service_remap_cancellation_reopen_and_tombstone() {
    real_service_transfer(None, true).await;
}
fn legacy_capture(n: u32, version: &str, effective: &str) -> CaptureBatchV1 {
    let context = ImportContextV1 {
        receipt_id: fixture::id(n + if version == "1" { 2 } else { 20_000 }),
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: fixture::id(102),
            device_id: None,
        },
        captured_at: serde_json::from_str("\"2026-11-01T12:00:00Z\"").unwrap(),
        received_at: serde_json::from_str("\"2026-11-02T12:00:00Z\"").unwrap(),
        coverage: vec![],
        row_ids: vec![ImportRowIdsV1 {
            observation: fixture::id(n),
            product: fixture::id(n + if version == "1" { 1 } else { 10_000 }),
            facet: fixture::id(900),
        }],
    };
    let bytes=format!("original_id,amount,unit,effective,description\nsynthetic-{n},-96,USD,{effective},synthetic independent observation\n").into_bytes();
    currents_acquisition::parse::FileParser::parse(
        currents_acquisition::capture::BoundedEvidence::new("duplicate.csv", bytes, context)
            .unwrap(),
        ParserProfileV1 {
            schema_version: 1,
            format: ImportFormatV1::SyntheticCsv,
            parser_version: version.into(),
            unit: UnitDefinition {
                id: UnitId(fixture::id(1).0),
                exponent: 2,
                native_code: "USD".into(),
                kind: UnitKind::Currency,
            },
        },
    )
    .unwrap()
}
async fn publish_legacy(f: &mut Fixture, path: &Path, n: u32, version: &str, effective: &str) {
    let evidence = ImportEvidence::new(legacy_capture(n, version, effective)).unwrap();
    let import = evidence.request();
    refresh_financial(f, path, Some(evidence)).await;
    submit(
        f,
        20000 + u128::from(n) + if version == "1" { 0 } else { 50000 },
        currents_protocol::OperationV1::PublishImport(import),
    )
    .await;
    refresh_financial(f, path, None).await;
}
fn legacy_binding(n: u32) -> SourceFacetBindingV1 {
    SourceFacetBindingV1 {
        observation: fixture::id(n),
        parse_product: fixture::id(n + 1),
        facet: fixture::id(900),
        amount: fixture::amount(-96),
    }
}
#[tokio::test]
async fn cur_015_4_real_service_duplicates_expiry_reparse_and_reconnect() {
    use currents_domain::recognition::RecognitionEngine;
    use currents_protocol::OperationV1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lifecycle.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    let mut requests = Vec::new();
    for (n, day) in [
        (200, "2026-11-01"),
        (220, "2026-11-02"),
        (240, "2026-11-03"),
    ] {
        publish_legacy(&mut f, &path, n, "1", day).await;
        let mut request = fixture::posted(legacy_binding(n), n + 100, 30);
        request.legs[0].leg.effective = fixture::date(day);
        request.legs[0].leg.posting = fixture::date(day);
        let terms = request.financial.as_mut().unwrap();
        terms.bookkeeping_date = fixture::date(day);
        for p in &mut terms.postings {
            p.effective = fixture::date(day);
            p.posting = fixture::date(day);
        }
        submit(
            &mut f,
            30000 + u128::from(n),
            OperationV1::AdoptActivity(request.clone()),
        )
        .await;
        refresh_financial(&mut f, &path, None).await;
        requests.push(request);
    }
    let before_reparse = f.prefix().await.native_head;
    publish_legacy(&mut f, &path, 200, "2", "2026-11-01").await;
    assert_eq!(f.prefix().await.native_head, before_reparse);
    let mut repeated = fixture::posted(
        SourceFacetBindingV1 {
            parse_product: fixture::id(10200),
            ..legacy_binding(200)
        },
        8000,
        30,
    );
    repeated.interpretation.reason =
        "Explicit synthetic reparse must not create a second economic movement".into();
    repeated.event.interpretation = SourceValue::Known(repeated.interpretation.clone());
    repeated.legs[0].leg.interpretation = SourceValue::Known(repeated.interpretation.clone());
    let mut rejected = f.body(39000);
    rejected.expected_head = f.prefix().await.through;
    rejected.operation = OperationV1::AdoptActivity(repeated);
    assert!(f.service.preview(f.ctx(), rejected).is_err());
    let survivor = requests[1].event.legs[0];
    let revision = AssociationRevisionV1 {
        id: fixture::id(9000),
        schema_version: 1,
        root: fixture::id(9000),
        prior_revision: None,
        kind: RelationshipKind::Duplicate,
        legs: requests.iter().map(|r| r.event.legs[0]).collect(),
        allocations: [200, 220, 240]
            .into_iter()
            .map(|n| QuantifiedFacetAllocationV1 {
                source: legacy_binding(n),
                leg: survivor,
                allocated: fixture::amount(-96),
                mode: FacetAllocationModeV1::Corroborating,
            })
            .collect(),
        captured_allocations: vec![],
        recognitions: vec![requests[1].financial.as_ref().unwrap().recognition_id],
        residuals: vec![],
        superseded_adoptions: vec![requests[0].adoption_id, requests[2].adoption_id],
        survivor: Some(survivor),
        retired_recognitions: vec![
            requests[0].financial.as_ref().unwrap().recognition_id,
            requests[2].financial.as_ref().unwrap().recognition_id,
        ],
        interpretation: fixture::interpretation(),
        governing_policy: requests[0].governing_policy.clone(),
    };
    let old = f.service.verified_projection().unwrap();
    submit(
        &mut f,
        40000,
        OperationV1::AssociateLegsComplete(CompleteAssociationRequestV1 {
            revision: revision.clone(),
            events: vec![],
            legs: vec![],
            recognitions: vec![],
        }),
    )
    .await;
    let deduped = refresh_financial(&mut f, &path, None).await;
    let index = deduped.accounting.as_ref().unwrap();
    assert_eq!(
        index
            .recognitions
            .keys()
            .filter(|id| !index.reversed.contains(id))
            .copied()
            .collect::<Vec<_>>(),
        revision.recognitions
    );
    assert_eq!(
        index.recognitions[&revision.recognitions[0]][0].value_date,
        "2026-11-02"
    );
    for (key, value) in old.records {
        assert_eq!(deduped.records.get(&key), Some(&value));
    }
    publish_legacy(&mut f, &path, 260, "1", "2026-11-01").await;
    let pending = fixture::pending(legacy_binding(260), 360);
    submit(&mut f, 41000, OperationV1::AdoptActivity(pending.clone())).await;
    let state = refresh_financial(&mut f, &path, None).await;
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
    let unchanged = f.prefix().await.native_head;
    f.service.close().await.unwrap();
    f.service = f.reopen().await;
    let state = refresh_financial(&mut f, &path, None).await;
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
    let expiry = PendingExpiryV1 {
        id: fixture::id(9100),
        schema_version: 1,
        pending_adoption: pending.adoption_id,
        interpretation: fixture::interpretation(),
        governing_policy: pending.governing_policy,
    };
    submit(&mut f, 41001, OperationV1::ExpirePending(expiry)).await;
    let state = refresh_financial(&mut f, &path, None).await;
    assert!(RecognitionEngine::pending(&state).unwrap().is_empty());
    assert_eq!(f.prefix().await.native_head, unchanged);
    let mut reconnect = fixture::catalog();
    reconnect.revision_id = fixture::id(9200);
    reconnect.prior_revision = Some(fixture::id(12));
    reconnect.catalog.aliases = vec![
        SourceAliasV1 {
            source: SourceIdentityV1 {
                source: "synthetic".into(),
                connection_id: fixture::id(102),
                device_id: None,
            },
            original_id: "checking-before".into(),
        },
        SourceAliasV1 {
            source: SourceIdentityV1 {
                source: "synthetic".into(),
                connection_id: fixture::id(902),
                device_id: None,
            },
            original_id: "checking-after".into(),
        },
    ];
    submit(
        &mut f,
        42000,
        OperationV1::SetAccountCatalogComplete(reconnect.clone()),
    )
    .await;
    let state = refresh_financial(&mut f, &path, None).await;
    assert_eq!(
        currents_domain::catalog::CatalogEngine::current(&state, fixture::id(11))
            .unwrap()
            .catalog
            .aliases,
        reconnect.catalog.aliases
    );
    assert_eq!(f.prefix().await.native_head, unchanged);
    f.service.close().await.unwrap();
    f.service = f.reopen().await;
    support::recover_service(&f.service).await;
    assert_eq!(
        f.service.verified_projection().unwrap().records,
        state.records
    );
    f.service.close().await.unwrap();
}

#[tokio::test]
async fn cur_015_opening_boundary_encrypted_initial_and_remap() {
    let mut failures = vec![];
    use association_fixtures::{association_date, boundary_rows, revised_transfer};
    use currents_protocol::OperationV1;
    for remap in [false, true] {
        for day in ["2026-10-31", "2026-11-01", "2026-11-03"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("boundary.db");
            let (mut f, _) = bootstrap_with_balance(&path, true).await;
            let (template, mut original) = association_fixtures::transfer_request();
            declare_template(&mut f, &path, &template).await;
            let binding = legacy_binding(250);
            let mut opening = fixture::posted(binding.clone(), 700, 50);
            opening.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
                segment_id: fixture::id(707),
                component_id: fixture::id(11),
                amount: binding.amount,
                cutoff: fixture::date("2026-11-01"),
                evidence: binding,
                included_activity: vec![],
            });
            submit(&mut f, 21001, OperationV1::AdoptActivity(opening)).await;
            refresh_financial(&mut f, &path, None).await;
            let request = if remap {
                association_date(&mut original, "2026-11-03");
                publish_relationship_rows(&mut f, &path, 1500, boundary_rows("2026-11-03", false))
                    .await;
                submit(
                    &mut f,
                    21002,
                    OperationV1::AssociateLegsComplete(original.clone()),
                )
                .await;
                refresh_financial(&mut f, &path, None).await;
                publish_relationship_rows(&mut f, &path, 1700, boundary_rows(day, true)).await;
                let mut next = revised_transfer(&original, fixture::id(1701));
                association_date(&mut next, day);
                next
            } else {
                publish_relationship_rows(&mut f, &path, 1500, boundary_rows(day, false)).await;
                association_date(&mut original, day);
                original
            };
            let before = f.prefix().await;
            let before_records = f.service.verified_projection().unwrap().records.clone();
            if day == "2026-11-03" {
                submit(&mut f, 21003, OperationV1::AssociateLegsComplete(request)).await;
                refresh_financial(&mut f, &path, None).await;
                assert_ne!(f.prefix().await.native_head, before.native_head);
            } else {
                let mut body = f.body(21003);
                body.expected_head = before.through;
                body.operation = OperationV1::AssociateLegsComplete(request);
                if f.service.preview(f.ctx(), body.clone()).is_ok() {
                    failures.push(format!("accepted remap={remap}, day={day}"));
                }
                assert!(f.service.submit(f.ctx(), body, f.auth(None)).await.is_err());
                assert_eq!(f.prefix().await, before);
                assert_eq!(
                    f.service.verified_projection().unwrap().records,
                    before_records
                );
            }
            let expected = f.service.verified_projection().unwrap().records.clone();
            f.service.close().await.unwrap();
            f.service = f.reopen().await;
            support::recover_service(&f.service).await;
            assert_eq!(f.service.verified_projection().unwrap().records, expected);
            f.service.close().await.unwrap();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn cur_015_remap_lineage_rejection_preserves_encrypted_replay() {
    use association_fixtures::{association_date, boundary_rows, revised_transfer};
    use currents_protocol::OperationV1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lineage.db");
    let (mut f, _) = bootstrap_with_balance(&path, true).await;
    let (template, mut original) = association_fixtures::transfer_request();
    declare_template(&mut f, &path, &template).await;
    let binding = legacy_binding(250);
    let mut opening = fixture::posted(binding.clone(), 700, 50);
    opening.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: fixture::id(707),
        component_id: fixture::id(11),
        amount: binding.amount,
        cutoff: fixture::date("2026-11-01"),
        evidence: binding,
        included_activity: vec![],
    });
    submit(&mut f, 21001, OperationV1::AdoptActivity(opening)).await;
    refresh_financial(&mut f, &path, None).await;
    let opening_lineage = f
        .service
        .verified_projection()
        .unwrap()
        .records
        .values()
        .find_map(|record| match record {
            AuthoritativeRecordV1::Recognition(value) => Some(value.lineage),
            _ => None,
        })
        .unwrap();
    association_date(&mut original, "2026-11-03");
    publish_relationship_rows(&mut f, &path, 1500, boundary_rows("2026-11-03", false)).await;
    submit(
        &mut f,
        21002,
        OperationV1::AssociateLegsComplete(original.clone()),
    )
    .await;
    refresh_financial(&mut f, &path, None).await;
    publish_relationship_rows(&mut f, &path, 1700, boundary_rows("2026-11-03", true)).await;
    let mut next = revised_transfer(&original, fixture::id(1701));
    association_date(&mut next, "2026-11-03");
    for (offset, lineage) in [opening_lineage, original.recognitions[0].lineage]
        .into_iter()
        .enumerate()
    {
        let mut colliding = next.clone();
        colliding.recognitions[0].lineage = lineage;
        let before = f.prefix().await;
        let records = f.service.verified_projection().unwrap().records.clone();
        let mut body = f.body(21100 + offset as u128);
        body.expected_head = before.through;
        body.operation = OperationV1::AssociateLegsComplete(colliding);
        assert!(f.service.preview(f.ctx(), body.clone()).is_err());
        assert!(f.service.submit(f.ctx(), body, f.auth(None)).await.is_err());
        assert_eq!(f.prefix().await, before);
        assert_eq!(f.service.verified_projection().unwrap().records, records);
    }
    submit(&mut f, 21003, OperationV1::AssociateLegsComplete(next)).await;
    refresh_financial(&mut f, &path, None).await;
    let expected = f.service.verified_projection().unwrap().records.clone();
    f.service.close().await.unwrap();
    f.service = f.reopen().await;
    support::recover_service(&f.service).await;
    assert_eq!(f.service.verified_projection().unwrap().records, expected);
    f.service.close().await.unwrap();
}

#[tokio::test]
async fn cur_015_3_real_parser_remap_preserves_raw_cancellation_retry_and_reopen() {
    real_service_transfer_mode(None, true, true).await;
}

#[test]
fn cur_015_3_parser_profile_rejects_forgery_and_preserves_capture_wire() {
    let capture = relationship_capture();
    let wire = serde_json::to_vec(&capture.capture).unwrap();
    let evidence = ImportEvidence::new_relationship(capture.clone()).unwrap();
    let mut state = DomainState::default();
    for record in evidence.plan(&state).unwrap().records {
        let id = match &record {
            AuthoritativeRecordV1::ImportReceipt(r) => r.id,
            AuthoritativeRecordV1::RelationshipCapture(c) => c.id,
            _ => panic!(),
        };
        state.records.insert(id, record);
    }
    let context = ImportContextV1 {
        receipt_id: id(200),
        source: capture.capture.source.clone(),
        captured_at: capture.capture.original_capture_time.clone(),
        received_at: capture.receipt.received_time.clone(),
        coverage: vec![],
        row_ids: vec![],
    };
    let profile = RelationshipCorrectionProfileV1 {
        schema_version: 1,
        source_record: "paystub-1".into(),
        source_revision: "original".into(),
        corrections: vec![RelationshipAmountCorrectionV1 {
            facet: "gross".into(),
            expected_amount: "5000".into(),
            amount: "5200".into(),
        }],
    };
    let batch = currents_acquisition::identity::RelationshipSourceParser::correct(
        &capture.capture,
        None,
        serde_json::to_vec(&profile).unwrap(),
        context.clone(),
        id(201),
    )
    .unwrap();
    let corrected = ImportEvidence::new_relationship_parse(batch.clone()).unwrap();
    let plan = corrected.plan(&state).unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    assert_eq!(batch.revision.row.source_revision, "original");
    assert_eq!(batch.revision.row.facets[0].amount, "5200");
    assert_eq!(serde_json::to_vec(&capture.capture).unwrap(), wire);
    assert_eq!(capture.capture.rows[0].facets[0].amount, "5000");
    for case in 0..8 {
        let mut forged = batch.clone();
        match case {
            0 => forged.contents[0].bytes.push(b' '),
            1 => forged.revision.row.facets[0].amount = "9999".into(),
            2 => forged.revision.row.facets[0].unit = "EUR".into(),
            3 => forged.revision.row.source_revision = "fabricated".into(),
            4 => forged.receipt.parser_version = "unsupported".into(),
            5 => forged.revision.correction.corrections[0].expected_amount = "4999".into(),
            6 => forged.revision.prior_parse = Some(id(999)),
            _ => forged.revision.capture = id(999),
        }
        assert!(
            ImportEvidence::new_relationship_parse(forged)
                .and_then(|e| e.plan(&state))
                .is_err(),
            "case {case}"
        );
    }
    for case in 0..5 {
        let mut p = profile.clone();
        match case {
            0 => p.corrections[0].expected_amount = "4999".into(),
            1 => p.corrections[0].amount = "NaN".into(),
            2 => p.corrections.push(p.corrections[0].clone()),
            3 => p.corrections[0].facet = "unknown".into(),
            _ => p.schema_version = 2,
        }
        assert!(
            currents_acquisition::identity::RelationshipSourceParser::correct(
                &capture.capture,
                None,
                serde_json::to_vec(&p).unwrap(),
                context.clone(),
                id(201)
            )
            .is_err()
        );
    }
}
