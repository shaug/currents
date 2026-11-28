//! Shared real encrypted service setup for synthetic financial integration tests.
use super::{fixture, support};
use currents_accounting::AccountingAdapter;
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_service::ImportEvidence;
use currents_storage::{EncryptedStore, HistoryAppend};
use currents_types::*;
use std::path::Path;
use support::Fixture;

pub(super) struct RelationshipPlanner {
    pub(super) state: DomainState,
    pub(super) evidence: Option<ImportEvidence>,
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
        let mut accounts = Vec::new();
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
            // A policy revision changes no accounting. The preview names the
            // account of each component whose position changes at a basis
            // the request's changes adopt (ruling G8 on #27).
            currents_protocol::OperationV1::SetDatasetPolicy(request) => {
                use currents_domain::policy::PolicyEngine;
                for basis in adopted_bases(request) {
                    for component in PolicyEngine::affected(&self.state, request, basis)? {
                        accounts.push(
                            currents_domain::catalog::CatalogEngine::current(
                                &self.state,
                                component,
                            )?
                            .catalog
                            .account_id,
                        );
                    }
                }
                PolicyEngine::set(&self.state, request)?
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
                    currents_protocol::OperationV1::SetDatasetPolicy(_) => DecisionKind::Policy,
                    _ => DecisionKind::Interpretation,
                },
                fields: vec![match &body.operation {
                    currents_protocol::OperationV1::SetDatasetPolicy(_) => "dataset-policy".into(),
                    _ => "relationship-source".into(),
                }],
            },
            decision_id: support::id(body.command_id.0.as_u128() + 200_000),
            manifest_id: support::id(body.command_id.0.as_u128() + 300_000),
            policy: PolicyReferenceV1 {
                id: policy.id,
                version: policy.version,
            },
        })
    }
    /// Position queries and explanations answer from the planner's state.
    /// The position engine refuses a basis other than its commit.
    fn query(
        &self,
        query: &currents_protocol::QueryV1,
        _: &AccountingPrefixV1,
    ) -> Result<currents_protocol::QueryResponseV1, EngineError> {
        use currents_domain::position::PositionEngine;
        use currents_protocol::{QueryKindV1, QueryOutputV1};
        let output = match query.kind {
            QueryKindV1::QueryPosition => QueryOutputV1::Position(PositionEngine::query(
                &self.state,
                &query.basis,
                &AccountScope {
                    accounts: query.accounts.clone(),
                    components: query.records.clone(),
                },
            )?),
            QueryKindV1::Explain => {
                let ([component], []) = (query.records.as_slice(), query.accounts.as_slice())
                else {
                    return Err(EngineError::invalid());
                };
                QueryOutputV1::Explanation(PositionEngine::explain(
                    &self.state,
                    &query.basis,
                    *component,
                )?)
            }
            _ => return Err(EngineError::version()),
        };
        Ok(currents_protocol::QueryResponseV1 {
            processed_head: self.state.through,
            basis: query.basis.clone(),
            health: vec![Health::Healthy],
            output,
        })
    }
}

/// Each distinct basis a policy request's changes adopt.
fn adopted_bases(request: &SetDatasetPolicyV1) -> Vec<&ReadBasis> {
    let mut bases: Vec<&ReadBasis> = Vec::new();
    for basis in [
        request.ownership_perimeter.as_ref().map(|c| &c.basis),
        request.source_priority.as_ref().map(|c| &c.basis),
        request.freshness_thresholds.as_ref().map(|c| &c.basis),
        request.timezone.as_ref().map(|c| &c.basis),
        request.reporting_currency.as_ref().map(|c| &c.basis),
    ]
    .into_iter()
    .flatten()
    {
        if !bases.contains(&basis) {
            bases.push(basis);
        }
    }
    bases
}

pub(super) async fn open(path: &Path) -> EncryptedStore {
    EncryptedStore::open(path, DatasetKey::new([0x10; 32]), CipherProfileV1)
        .await
        .unwrap()
}
/// Installs a planner for the current verified state. The service keeps that
/// state current after its own commits, so this replays only after a reopen.
pub(super) async fn refresh_financial(
    f: &mut Fixture,
    evidence: Option<ImportEvidence>,
) -> DomainState {
    let state = match f.service.verified_projection() {
        Ok(state) => state,
        // A newly opened service has no verified prefix until a full replay.
        Err(error) if error.class == ErrorClass::ResultUnknown => {
            support::recover_service(&f.service).await;
            f.service.verified_projection().unwrap()
        }
        Err(error) => panic!("verified projection did not follow the last commit: {error:?}"),
    };
    // Keep the session rotation that the old per-refresh restart performed.
    f.host.reconnect();
    f.planner.set(std::sync::Arc::new(RelationshipPlanner {
        state: state.clone(),
        evidence,
    }));
    state
}
pub(super) async fn submit(
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

pub(super) async fn bootstrap(path: &Path) -> (Fixture, NativeHead) {
    bootstrap_with_balance(path, false).await
}
pub(super) async fn bootstrap_with_balance(path: &Path, balance: bool) -> (Fixture, NativeHead) {
    let opening = [RetainedEvidence {
        n: 250,
        value: -96,
        day: "2026-11-01",
        kind: ObservationKind::Balance,
        text: "synthetic opening balance",
    }];
    bootstrap_with_evidence(path, if balance { &opening } else { &[] }).await
}
/// One synthetic observation with a dated facet and retained content.
pub(super) struct RetainedEvidence<'a> {
    pub(super) n: u32,
    pub(super) value: i128,
    pub(super) day: &'a str,
    pub(super) kind: ObservationKind,
    pub(super) text: &'a str,
}
/// The genesis commit also retains each observation, its parse product and its
/// content. Observations with the same text share one content and the import
/// receipt of the first of them. Trusted synthetic history, as in CUR013; the
/// file adapter only emits movements.
pub(super) async fn bootstrap_with_evidence(
    path: &Path,
    evidence: &[RetainedEvidence<'_>],
) -> (Fixture, NativeHead) {
    bootstrap_genesis(path, evidence, |_, _, _| {}).await
}
/// The genesis commit, with `shape` applied to each observation and its parse
/// product before its receipt takes the observation's source.
pub(super) async fn bootstrap_genesis(
    path: &Path,
    evidence: &[RetainedEvidence<'_>],
    shape: impl Fn(u32, &mut ObservationVersionV1, &mut AddressedParseProductV1),
) -> (Fixture, NativeHead) {
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
    let mut digests = std::collections::BTreeMap::new();
    for item in evidence {
        let n = item.n;
        fixture::evidence(&mut fixture, n, item.value, None);
        let bytes = item.text;
        let first = !digests.contains_key(bytes);
        if first {
            let content = store.stage(bytes.as_bytes()).await.unwrap();
            digests.insert(bytes, content.digest());
            staged.push(content);
        }
        let digest = digests[bytes];
        let Some(AuthoritativeRecordV1::ObservationVersion(mut o)) =
            fixture.records.remove(&fixture::id(n))
        else {
            panic!()
        };
        let Some(AuthoritativeRecordV1::AddressedParseProduct(mut p)) =
            fixture.records.remove(&fixture::id(n + 1))
        else {
            panic!()
        };
        o.content_digest = digest;
        o.sanitized_representation = bytes.into();
        o.semantic_kind = item.kind;
        o.temporal = fixture::date(item.day);
        p.product.facets[0].kind = item.kind;
        p.product.facets[0].temporal = fixture::date(item.day);
        shape(n, &mut o, &mut p);
        let receipt = AuthoritativeRecordV1::ImportReceipt(ImportReceiptV1 {
            id: fixture::id(n + 2),
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
                digest,
                byte_count: CommitSeq(bytes.len() as u64),
                media_type: "text/plain".into(),
            }],
        });
        if first {
            records.push(receipt);
        }
        records.push(AuthoritativeRecordV1::ObservationVersion(o));
        records.push(AuthoritativeRecordV1::AddressedParseProduct(p));
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
    refresh_financial(&mut f, None).await;
    (f, empty)
}

pub(super) fn template_catalog_commands(
    template: &DomainState,
) -> impl Iterator<Item = (u128, currents_protocol::OperationV1)> + '_ {
    use currents_protocol::OperationV1;
    template
        .records
        .values()
        .filter_map(|record| {
            let AuthoritativeRecordV1::AccountCatalogRevision(r) = record else {
                return None;
            };
            let Some(AuthoritativeRecordV1::AccountComponent(root)) =
                template.records.get(&r.component_id)
            else {
                panic!()
            };
            Some(OperationV1::SetAccountCatalogComplete(CatalogRequestV1 {
                schema_version: r.schema_version,
                revision_id: r.id,
                prior_revision: r.prior_revision,
                catalog: r.catalog.clone(),
                ownership: root.ownership.clone(),
                origin: r.origin.clone(),
                overlaps: vec![],
            }))
        })
        .zip(11000..)
        .map(|(operation, serial)| (serial, operation))
}

pub(super) async fn declare_template(f: &mut Fixture, template: &DomainState) {
    for (serial, operation) in template_catalog_commands(template) {
        submit(f, serial, operation).await;
        refresh_financial(f, None).await;
    }
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
pub(super) async fn publish_legacy(f: &mut Fixture, n: u32, version: &str, effective: &str) {
    let evidence = ImportEvidence::new(legacy_capture(n, version, effective)).unwrap();
    let import = evidence.request();
    refresh_financial(f, Some(evidence)).await;
    submit(
        f,
        20000 + u128::from(n) + if version == "1" { 0 } else { 50000 },
        currents_protocol::OperationV1::PublishImport(import),
    )
    .await;
    refresh_financial(f, None).await;
}
pub(super) fn legacy_binding(n: u32) -> SourceFacetBindingV1 {
    SourceFacetBindingV1 {
        observation: fixture::id(n),
        parse_product: fixture::id(n + 1),
        facet: fixture::id(900),
        amount: fixture::amount(-96),
    }
}
/// Publishes `rows` as a retained relationship capture, receipt `n`.
pub(super) async fn publish_relationship_rows(
    f: &mut Fixture,
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
    refresh_financial(f, Some(evidence)).await;
    submit(
        f,
        12000 + u128::from(n),
        currents_protocol::OperationV1::PublishImport(import),
    )
    .await;
    refresh_financial(f, None).await;
}
