//! Separate CUR-016_2 acceptance of append-only compensation and new refunds.
use currents_accounting::AccountingAdapter;
use currents_domain::correction::{CorrectionEngine, CorrectionRequestV1};
use currents_protocol::OperationV1;
use currents_service as service_api;
use currents_service::{ReplayCoordinator, synthetic_commit_verified_correction};
use currents_storage::{CommitControl, PreparedCommit};
use currents_types::*;
use std::sync::Arc;

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

fn fresh_correction(
    state: &DomainState,
    original: RecognitionId,
    serial: u32,
) -> CorrectionRequestV1 {
    let AuthoritativeRecordV1::Recognition(original) = &state.records[&RecordId(original.0)] else {
        panic!("original was not retained");
    };
    let mut rematerialized = original.clone();
    rematerialized.id = RecognitionId(fixture::id(serial).0);
    for (ordinal, posting) in rematerialized.postings.iter_mut().enumerate() {
        posting.id = fixture::id(serial + 1 + u32::try_from(ordinal).unwrap());
    }
    CorrectionRequestV1 {
        active_recognition: original.id,
        replacement: vec![rematerialized],
        remapping: vec![],
        reason: "Append another exact identity compensation for the one active recognition".into(),
    }
}

fn prepare(
    state: &DomainState,
    request: &CorrectionRequestV1,
    serial: u32,
) -> Result<PreparedCommit, EngineError> {
    let revision = fixture::id(serial);
    let plan = CorrectionEngine::correct_durable(state, revision, request)?;
    let frozen_batch = AccountingAdapter::freeze(&plan.accounting, state)?;
    Ok(PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id: CommandId(fixture::id(serial + 1).0),
            digest: Digest([71; 32]),
            state: CommandState::Committed,
            commit: None,
            effects: EffectsV1 {
                records: vec![revision, RecordId(request.replacement[0].id.0)],
                accounts: vec![],
                amounts: vec![],
                invalidations: plan.invalidations.clone(),
            },
            health: vec![],
        },
        decision_plan: plan,
        frozen_batch,
        prior_head: state.through,
        decisions: vec![],
        manifest_id: fixture::id(serial + 2),
        control: Arc::new(CommitControl::default()),
    })
}

#[tokio::test]
async fn cur_016_2_live_tip_can_be_corrected_again_without_a_supersession_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("correction-chain.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    publish_legacy(&mut f, &path, 200, "1", "2026-11-01").await;
    let purchase = fixture::posted(legacy_binding(200), 300, 30);
    let root = purchase.financial.as_ref().unwrap().recognition_id;
    let adoption = purchase.adoption_id;
    submit(&mut f, 53_001, OperationV1::AdoptActivity(purchase)).await;
    let initial = refresh_financial(&mut f, &path, None).await;

    let first = fresh_correction(&initial, root, 9400);
    let middle = first.replacement[0].id;
    f.service.close().await.unwrap();
    let store = open(&path).await;
    synthetic_commit_verified_correction(
        store.writer(),
        prepare(&initial, &first, 53_010).unwrap(),
    )
    .await
    .unwrap();
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let once = refresh_financial(&mut f, &path, None).await;
    assert!(once.accounting.as_ref().unwrap().reversed.contains(&root));
    assert!(!once.accounting.as_ref().unwrap().reversed.contains(&middle));

    // Cycling back to a retained recognition ID must not create an old owner.
    let mut cycle = fresh_correction(&once, middle, 9500);
    cycle.replacement[0].id = root;
    assert!(prepare(&once, &cycle, 53_020).is_err());

    let second = fresh_correction(&once, middle, 9500);
    let tip = second.replacement[0].id;
    let second = prepare(&once, &second, 53_030)
        .expect("a valid active successor is not an obsolete root and can be corrected again");
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let committed = synthetic_commit_verified_correction(store.writer(), second)
        .await
        .unwrap();
    assert_eq!(committed.commit, CommitSeq(once.through.0 + 1));
    store
        .history()
        .verify_checkpoint(initial.through, initial.domain_digest)
        .await
        .unwrap();
    store
        .history()
        .verify_checkpoint(once.through, once.domain_digest)
        .await
        .unwrap();
    let fresh = open(&dir.path().join("chain-rebuild.db")).await;
    let report = ReplayCoordinator::rebuild(store.history(), fresh)
        .await
        .unwrap();
    assert_eq!(report.native_head, committed.native_head);
    store.close().await.unwrap();

    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, &path, None).await;
    assert_eq!(
        current.records.get(&adoption),
        initial.records.get(&adoption)
    );
    for (id, historical) in &once.records {
        assert_eq!(current.records.get(id), Some(historical));
    }
    let index = current.accounting.as_ref().unwrap();
    assert!(index.reversed.contains(&root));
    assert!(index.reversed.contains(&middle));
    assert!(!index.reversed.contains(&tip));
    for (old, next, owner) in [(root, middle, 53_010), (middle, tip, 53_030)] {
        assert!(matches!(current.records.get(&fixture::id(owner)),
            Some(AuthoritativeRecordV1::RecognitionCorrectionRevision(r))
                if r.prior_recognition == old && r.replacement_recognition == next));
    }
    let mut cycle = fresh_correction(&current, tip, 9600);
    cycle.replacement[0].id = root;
    assert!(prepare(&current, &cycle, 53_040).is_err());
    use currents_domain::recognition::RecognitionEngine;
    RecognitionEngine::audit(&current).unwrap();

    for (from, into) in [(tip, root), (root, tip)] {
        // Fabricate a coherent-looking edge in a copied projection. This must
        // fail graph audit independently of the fresh-ID preview guard.
        let mut forged = current.clone();
        let key = fixture::id(53_050);
        forged.records.insert(
            key,
            AuthoritativeRecordV1::RecognitionCorrectionRevision(RecognitionCorrectionRevisionV1 {
                id: key,
                schema_version: 1,
                prior_recognition: from,
                replacement_recognition: into,
                reason: "synthetic attempted cycle or fork".into(),
                remap_disposition: RecognitionCorrectionRemapV1::UnchangedStableLegs,
            }),
        );
        assert!(RecognitionEngine::audit(&forged).is_err());
    }
    let mut orphaned = current.clone();
    orphaned.records.remove(&adoption);
    assert!(RecognitionEngine::audit(&orphaned).is_err());

    let mut altered = current.clone();
    let Some(AuthoritativeRecordV1::Recognition(revision)) =
        altered.records.get_mut(&RecordId(middle.0))
    else {
        panic!("known archived correction revision disappeared");
    };
    revision.postings[0].amount.coefficient -= 1;
    revision.postings[1].amount.coefficient += 1;
    let frozen = &mut altered
        .accounting
        .as_mut()
        .unwrap()
        .recognitions
        .get_mut(&middle)
        .unwrap()[0];
    for (posting, typed) in frozen.postings.iter_mut().zip(&revision.postings) {
        posting.original.amount = typed.amount.normalized().unwrap();
        posting.native_coefficient = typed
            .amount
            .checked_rescale(18)
            .unwrap()
            .coefficient
            .to_string();
    }
    assert!(RecognitionEngine::audit(&altered).is_err());

    let mut no_live_tip = current.clone();
    no_live_tip
        .accounting
        .as_mut()
        .unwrap()
        .reversed
        .insert(tip);
    assert!(RecognitionEngine::audit(&no_live_tip).is_err());

    let mut historical_dependency = current.clone();
    historical_dependency.records.insert(
        fixture::id(53_060),
        AuthoritativeRecordV1::Review(ReviewRecordV1 {
            id: fixture::id(53_060),
            targets: vec![RecordId(root.0)],
            disposition: ReviewDisposition::NeedsEvidence,
            reason: "archived dependency cannot be silently remapped".into(),
        }),
    );
    let late = fresh_correction(&historical_dependency, tip, 9700);
    assert!(prepare(&historical_dependency, &late, 53_070).is_err());
}

#[tokio::test]
async fn cur_016_2_refund_is_a_new_economic_event_not_a_native_reversal() {
    use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
    use currents_service::ImportEvidence;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("refund.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    publish_legacy(&mut f, &path, 200, "1", "2026-11-01").await;
    let purchase = fixture::posted(legacy_binding(200), 300, 30);
    let purchase_id = purchase.financial.as_ref().unwrap().recognition_id;
    let purchase_event = purchase.event.id;
    submit(&mut f, 54_001, OperationV1::AdoptActivity(purchase)).await;
    let before = refresh_financial(&mut f, &path, None).await;

    let serial = 220;
    let capture = FileParser::parse(
        BoundedEvidence::new(
            "refund.csv",
            include_bytes!("../../../fixtures/journeys/s5/refund.csv").to_vec(),
            ImportContextV1 {
                receipt_id: fixture::id(serial + 2),
                source: SourceIdentityV1 {
                    source: "synthetic".into(),
                    connection_id: fixture::id(102),
                    device_id: None,
                },
                captured_at: serde_json::from_str("\"2026-11-07T16:01:00Z\"").unwrap(),
                received_at: serde_json::from_str("\"2026-11-07T16:01:01Z\"").unwrap(),
                coverage: vec![],
                row_ids: vec![ImportRowIdsV1 {
                    observation: fixture::id(serial),
                    product: fixture::id(serial + 1),
                    facet: fixture::id(900),
                }],
            },
        )
        .unwrap(),
        ParserProfileV1 {
            schema_version: 1,
            format: ImportFormatV1::SyntheticCsv,
            parser_version: "1".into(),
            unit: UnitDefinition {
                id: UnitId(fixture::id(1).0),
                exponent: 2,
                native_code: "USD".into(),
                kind: UnitKind::Currency,
            },
        },
    )
    .unwrap();
    let evidence = ImportEvidence::new(capture).unwrap();
    let import = evidence.request();
    refresh_financial(&mut f, &path, Some(evidence)).await;
    submit(&mut f, 54_002, OperationV1::PublishImport(import)).await;
    refresh_financial(&mut f, &path, None).await;
    let mut refund = fixture::posted(
        SourceFacetBindingV1 {
            amount: fixture::amount(96),
            ..legacy_binding(serial)
        },
        320,
        30,
    );
    let day = fixture::date("2026-11-07");
    refund.legs[0].leg.effective = day.clone();
    refund.legs[0].leg.posting = day.clone();
    let terms = refund.financial.as_mut().unwrap();
    let refund_id = terms.recognition_id;
    terms.bookkeeping_date = day.clone();
    for posting in &mut terms.postings {
        posting.effective = day.clone();
        posting.posting = day.clone();
    }
    let refund_event = refund.event.id;
    assert_ne!(refund_event, purchase_event);
    submit(&mut f, 54_003, OperationV1::AdoptActivity(refund)).await;
    let after = refresh_financial(&mut f, &path, None).await;
    assert_eq!(
        after.records.get(&purchase_event),
        before.records.get(&purchase_event)
    );
    for id in [purchase_id, refund_id] {
        assert!(!after.accounting.as_ref().unwrap().reversed.contains(&id));
    }
    let native = after.accounting.as_ref().unwrap();
    let [charge] = native.recognitions[&purchase_id].as_slice() else {
        panic!()
    };
    let [refund] = native.recognitions[&refund_id].as_slice() else {
        panic!()
    };
    assert!(refund.reverses.is_none());
    assert_eq!(charge.value_date, "2026-11-01");
    assert_eq!(refund.value_date, "2026-11-07");
    assert_eq!(refund.booking_date, "2026-11-07");
    assert_eq!(charge.postings[0].original.amount, fixture::amount(-96));
    assert_eq!(refund.postings[0].original.amount, fixture::amount(96));
    assert!(after.records.contains_key(&refund_event));
}

#[tokio::test]
async fn cur_016_2_correction_preserves_note_and_invalidates_only_matching_predicate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dependent-correction.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    publish_legacy(&mut f, &path, 200, "1", "2026-11-01").await;
    let purchase = fixture::posted(legacy_binding(200), 300, 30);
    let source_leg = RecordId(purchase.legs[0].leg.id.0);
    let original = purchase.financial.as_ref().unwrap().recognition_id;
    submit(&mut f, 56_001, OperationV1::AdoptActivity(purchase)).await;
    let before = refresh_financial(&mut f, &path, None).await;
    let date = |civil: &str| {
        let TemporalValue::Date { civil, .. } = fixture::date(civil) else {
            panic!("fixture civil date");
        };
        civil
    };
    let claim = |serial, start, end| {
        AuthoritativeRecordV1::ReconciliationClaim(ReconciliationClaimV1 {
            id: fixture::id(serial),
            comparison_type: "Activity".into(),
            scope: vec![AccountId(fixture::id(10).0)],
            cutoffs: DateScope {
                start: Some(date(start)),
                end: Some(date(end)),
                basis: DateBasis::Effective,
            },
            evidence_dependencies: vec![],
            decision_dependencies: vec![],
            dependency_digest: Digest([87; 32]),
            matched_allocations: vec![],
            differences: vec![],
            exclusions: vec![],
            status: ReconciliationStatus::Matched,
            reason: "Retained synthetic comparison".into(),
        })
    };
    let affected_claim = fixture::id(56_100);
    let unrelated_claim = fixture::id(56_101);
    let note_id = fixture::id(56_102);
    let note = AuthoritativeRecordV1::Organization(OrganizationRecordV1 {
        id: note_id,
        version: 1,
        kind: OrganizationKind::Note,
        targets: vec![source_leg],
        parent: None,
        text: "Synthetic note attached to the stable economic leg".into(),
        allocations: vec![],
        supersedes: None,
    });
    let records = vec![
        claim(56_100, "2026-11-01", "2026-11-30"),
        claim(56_101, "2026-12-01", "2026-12-31"),
        note.clone(),
    ];
    let accounting = AccountingIntent::NoEffect(NoEffectV1 {
        reason: "retain synthetic dependent records".into(),
    });
    let frozen_batch = AccountingAdapter::freeze(&accounting, &before).unwrap();
    f.service.close().await.unwrap();
    let store = open(&path).await;
    store
        .writer()
        .commit(PreparedCommit {
            evidence: vec![],
            decision_plan: DecisionPlan {
                records,
                invalidations: vec![],
                accounting,
            },
            frozen_batch,
            prior_head: before.through,
            decisions: vec![],
            result: CommandResultV1 {
                command_id: CommandId(fixture::id(56_002).0),
                digest: Digest([88; 32]),
                state: CommandState::Committed,
                commit: None,
                effects: EffectsV1 {
                    records: vec![affected_claim, unrelated_claim, note_id],
                    accounts: vec![],
                    amounts: vec![],
                    invalidations: vec![],
                },
                health: vec![],
            },
            manifest_id: fixture::id(56_003),
            control: Arc::new(CommitControl::default()),
        })
        .await
        .unwrap();
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let retained = refresh_financial(&mut f, &path, None).await;
    assert_eq!(retained.records[&note_id], note);
    assert_eq!(
        retained.records[&affected_claim],
        claim(56_100, "2026-11-01", "2026-11-30")
    );
    assert_eq!(
        retained.records[&unrelated_claim],
        claim(56_101, "2026-12-01", "2026-12-31")
    );

    let request = fresh_correction(&retained, original, 56_110);
    // Differential shapes exercise the planner only; the positive baseline
    // above is retained encrypted authority, not a manually seeded projection.
    for field in 0..3 {
        let mut unsupported = retained.clone();
        let Some(AuthoritativeRecordV1::ReconciliationClaim(claim)) =
            unsupported.records.get_mut(&affected_claim)
        else {
            panic!("retained claim")
        };
        match field {
            0 => claim.evidence_dependencies.push(fixture::id(200)),
            1 => claim.decision_dependencies.push(fixture::id(201)),
            2 => claim.exclusions.push(source_leg),
            _ => unreachable!(),
        }
        assert_eq!(
            CorrectionEngine::correct_durable(&unsupported, fixture::id(56_120), &request)
                .unwrap_err()
                .class,
            EngineError::version().class,
            "unsupported claim dependency shape {field}"
        );
    }
    let prepared = prepare(&retained, &request, 56_120)
        .expect("bounded predicates must not prohibit a correction of their exact prior");
    assert!(
        prepared
            .decision_plan
            .invalidations
            .contains(&affected_claim)
    );
    assert!(
        !prepared
            .decision_plan
            .invalidations
            .contains(&unrelated_claim)
    );
    assert!(!prepared.decision_plan.invalidations.contains(&note_id));
    assert_eq!(
        retained.reconciliation_status(affected_claim),
        Some(ReconciliationStatus::Matched)
    );
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let mut forged = prepare(&retained, &request, 56_120).unwrap();
    forged
        .decision_plan
        .invalidations
        .retain(|id| *id != affected_claim);
    forged.result.effects.invalidations = forged.decision_plan.invalidations.clone();
    assert!(
        synthetic_commit_verified_correction(store.writer(), forged)
            .await
            .is_err()
    );
    assert_eq!(
        store.writer().audited_prefix().await.unwrap().through,
        retained.through
    );
    let committed = synthetic_commit_verified_correction(store.writer(), prepared)
        .await
        .unwrap();
    let exact = store
        .history()
        .read_prefix(committed.commit)
        .await
        .unwrap()
        .pop()
        .unwrap();
    for contaminate in [false, true] {
        let mut forged = exact.clone();
        forged
            .command_result
            .effects
            .invalidations
            .retain(|id| *id != affected_claim);
        if contaminate {
            forged
                .command_result
                .effects
                .invalidations
                .push(unrelated_claim);
        }
        let mut untouched = retained.clone();
        assert!(currents_domain::ReplayEngine::apply(&mut untouched, &forged).is_err());
        assert_eq!(untouched.through, retained.through);
        assert_eq!(
            untouched.reconciliation_status(affected_claim),
            Some(ReconciliationStatus::Matched)
        );
    }
    let fresh_path = dir.path().join("dependent-rebuild.db");
    let fresh = open(&fresh_path).await;
    let replay = ReplayCoordinator::rebuild(store.history(), fresh)
        .await
        .unwrap();
    assert_eq!(replay, store.history().audit_report().await.unwrap());
    store
        .history()
        .verify_checkpoint(retained.through, retained.domain_digest)
        .await
        .unwrap();
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, &path, None).await;
    assert_eq!(current.records[&note_id], note);
    assert_eq!(
        current.records[&affected_claim],
        retained.records[&affected_claim]
    );
    assert_eq!(
        retained.reconciliation_status(affected_claim),
        Some(ReconciliationStatus::Matched)
    );
    assert_eq!(
        current.reconciliation_status(affected_claim),
        Some(ReconciliationStatus::Invalidated)
    );
    assert_eq!(
        current.reconciliation_status(unrelated_claim),
        Some(ReconciliationStatus::Matched)
    );
}

// The legacy request cannot name the immutable revision. This fixture supplies
// every ID explicitly for one synthetic command rather than inventing a
// production rule for deriving them from the client request.
struct ExactSyntheticCorrectionPlanner {
    state: DomainState,
    command: CommandId,
    revision: RecordId,
    decision: RecordId,
    manifest: RecordId,
    policy: PolicyReferenceV1,
}

impl service_api::CommandPlanner for ExactSyntheticCorrectionPlanner {
    fn prepare(
        &self,
        body: &currents_protocol::CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<service_api::CommandPlan, EngineError> {
        let OperationV1::CorrectRecognition(request) = &body.operation else {
            return Err(EngineError::version());
        };
        if body.command_id != self.command
            || !self
                .state
                .dataset_policies
                .contains_key(&(self.policy.id, self.policy.version))
        {
            return Err(EngineError::invalid());
        }
        let request = CorrectionRequestV1 {
            active_recognition: request.active_recognition,
            replacement: request.replacement.clone(),
            remapping: request.remapping.clone(),
            reason: request.reason.clone(),
        };
        let plan = CorrectionEngine::correct_durable(&self.state, self.revision, &request)?;
        let frozen = AccountingAdapter::freeze(&plan.accounting, &self.state)?;
        service_api::verify_correction_preflight(&self.state, prefix, &plan, &frozen)?;
        let mut accounts = std::collections::BTreeSet::new();
        let mut amounts = Vec::new();
        for operation in &frozen.operations {
            let FrozenNativeOperationV1::Entry(entry) = operation else {
                return Err(EngineError::invalid());
            };
            for posting in &entry.postings {
                accounts.insert(posting.original.account_id);
                amounts.push(posting.original.amount);
            }
        }
        let accounts: Vec<_> = accounts.into_iter().collect();
        let effects = EffectsV1 {
            records: vec![self.revision, RecordId(request.replacement[0].id.0)],
            accounts: accounts.clone(),
            amounts,
            invalidations: plan.invalidations.clone(),
        };
        Ok(service_api::CommandPlan {
            import: None,
            decision_plan: plan,
            frozen_batch: frozen,
            effects,
            warnings: vec![],
            approval_scope: currents_protocol::ApprovalScopeV1 {
                accounts,
                operation: DecisionKind::Correction,
                fields: vec!["recognition".into()],
            },
            decision_id: self.decision,
            manifest_id: self.manifest,
            policy: self.policy.clone(),
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

// Synthetic host-only sidecar. Neither the legacy body nor the Service DTO
// is extended; the fixture refuses the real Service challenge unless both
// canonical commitments and the separately approved typed digest match.
struct CandidateApprovalHost {
    inner: Arc<support::Host>,
    body_digest: Digest,
    closure_digest: Digest,
    candidate_digest: Digest,
    challenge: std::sync::Mutex<Option<service_api::ApprovalChallenge>>,
    approval: std::sync::Mutex<Option<(uuid::Uuid, Digest, Digest, Digest)>>,
}

impl CandidateApprovalHost {
    fn proof(
        &self,
        fixture: &support::Fixture,
        approved_candidate: Digest,
    ) -> currents_protocol::AuthorizationEnvelope {
        let challenge = self.challenge.lock().unwrap().clone().unwrap();
        *self.approval.lock().unwrap() = Some((
            challenge.nonce,
            challenge.preview.digest,
            challenge.preview.closure_digest,
            approved_candidate,
        ));
        fixture.auth(Some(challenge.nonce.to_string()))
    }
}

impl service_api::HostGrants for CandidateApprovalHost {
    fn revoke_sessions(&self, d: DatasetId, i: InstanceId) {
        self.inner.revoke_sessions(d, i);
    }
    fn authenticate(
        &self,
        ctx: &currents_protocol::RequestContext,
        d: DatasetId,
        capability: Option<&str>,
        now: OffsetDateTime,
    ) -> Result<(), EngineError> {
        self.inner.authenticate(ctx, d, capability, now)
    }
    fn authorize_command(
        &self,
        ctx: &currents_protocol::RequestContext,
        b: &currents_protocol::CommandBodyV1,
    ) -> Result<(), EngineError> {
        self.inner.authorize_command(ctx, b)
    }
    fn authorize_effects(
        &self,
        ctx: &currents_protocol::RequestContext,
        p: &currents_protocol::PreviewV1,
    ) -> Result<(), EngineError> {
        self.inner.authorize_effects(ctx, p)
    }
    fn publish_challenge(&self, c: &service_api::ApprovalChallenge) -> Result<(), EngineError> {
        if c.preview.digest != self.body_digest || c.preview.closure_digest != self.closure_digest {
            return Err(EngineError::invalid());
        }
        self.inner.publish_challenge(c)?;
        *self.challenge.lock().unwrap() = Some(c.clone());
        Ok(())
    }
    fn verify_approval(
        &self,
        ctx: &currents_protocol::RequestContext,
        envelope: &currents_protocol::AuthorizationEnvelope,
        c: &service_api::ApprovalChallenge,
        now: OffsetDateTime,
    ) -> Result<(), EngineError> {
        self.inner.verify_approval(ctx, envelope, c, now)?;
        if *self.approval.lock().unwrap()
            != Some((
                c.nonce,
                self.body_digest,
                self.closure_digest,
                self.candidate_digest,
            ))
        {
            return Err(EngineError {
                class: ErrorClass::Unauthorized,
                affected_scope: vec![],
                next_action: SafeNextAction::Authenticate,
            });
        }
        Ok(())
    }
    fn authorize_result(
        &self,
        ctx: &currents_protocol::RequestContext,
        access: service_api::ResultAccess<'_>,
    ) -> Result<(), EngineError> {
        self.inner.authorize_result(ctx, access)
    }
    fn authorize_query(
        &self,
        ctx: &currents_protocol::RequestContext,
        q: &currents_protocol::QueryV1,
    ) -> Result<(), EngineError> {
        self.inner.authorize_query(ctx, q)
    }
}

#[tokio::test]
async fn cur_016_2_synthetic_service_approves_exact_correction_and_replays_owner_decision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("owner-approved-correction.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    publish_legacy(&mut f, &path, 200, "1", "2026-11-01").await;
    let purchase = fixture::posted(legacy_binding(200), 300, 30);
    let original = purchase.financial.as_ref().unwrap().recognition_id;
    let original_adoption = purchase.adoption_id;
    let governing_policy = purchase.governing_policy.clone();
    submit(&mut f, 58_001, OperationV1::AdoptActivity(purchase)).await;
    let before = refresh_financial(&mut f, &path, None).await;

    let correction = fresh_correction(&before, original, 58_020);
    let replacement = correction.replacement[0].id;
    let mut body = f.body(58_002);
    body.expected_head = before.through;
    body.operation = OperationV1::CorrectRecognition(currents_types::CorrectionRequestV1 {
        active_recognition: correction.active_recognition,
        replacement: correction.replacement.clone(),
        remapping: correction.remapping,
        reason: correction.reason,
    });
    f.service.close().await.unwrap();
    let candidate = service_api::SyntheticCorrectionCandidate {
        dataset: body.dataset_id,
        command: body.command_id,
        expected_basis: before.accounting.as_ref().unwrap().prefix.clone(),
        revision: fixture::id(58_010),
        active_recognition: original,
        replacement: match &body.operation {
            OperationV1::CorrectRecognition(request) => request.replacement[0].clone(),
            _ => panic!("legacy carrier"),
        },
        reason: match &body.operation {
            OperationV1::CorrectRecognition(request) => request.reason.clone(),
            _ => panic!("legacy carrier"),
        },
        governing_policy: governing_policy.clone(),
    };
    let store = open(&path).await;
    let typed = service_api::synthetic_preview_correction_candidate(store.writer(), &candidate)
        .await
        .unwrap();
    let mut alternate = candidate.clone();
    alternate.revision = fixture::id(58_013);
    let substituted =
        service_api::synthetic_preview_correction_candidate(store.writer(), &alternate)
            .await
            .unwrap();
    assert_ne!(substituted.candidate_digest, typed.candidate_digest);
    store.close().await.unwrap();
    use service_api::CommandPlanner as _;
    use sha2::{Digest as _, Sha256};
    let planner = Arc::new(ExactSyntheticCorrectionPlanner {
        state: before.clone(),
        command: body.command_id,
        revision: candidate.revision,
        decision: fixture::id(58_011),
        manifest: fixture::id(58_012),
        policy: typed.policy.clone(),
    });
    let plan = planner.prepare(&body, &typed.basis).unwrap();
    assert_eq!(plan.decision_plan, typed.plan);
    assert_eq!(plan.policy, typed.policy);
    let closure = currents_protocol::canonical_bytes_with_limits(
        &plan,
        currents_protocol::CanonicalLimits {
            max_bytes: 65 * 1_048_576,
            max_collection_items: 65_536,
        },
    )
    .unwrap();
    let host = Arc::new(CandidateApprovalHost {
        inner: f.host.clone(),
        body_digest: currents_protocol::command_digest(&body).unwrap(),
        closure_digest: Digest(Sha256::digest(closure).into()),
        candidate_digest: typed.candidate_digest,
        challenge: std::sync::Mutex::new(None),
        approval: std::sync::Mutex::new(None),
    });
    f.host.reconnect();
    f.service = service_api::Service::new(
        open(&path).await,
        body.dataset_id,
        f.ctx().instance_id,
        service_api::ServiceProfile::Synthetic,
        host.clone(),
        planner,
        f.clock.clone(),
    )
    .await
    .unwrap();
    support::recover_service(&f.service).await;
    let preview = f
        .service
        .preview(f.ctx(), body.clone())
        .expect("verified synthetic service preview must disclose a correction before approval");
    assert_eq!(preview.approval_scope.operation, DecisionKind::Correction);
    assert_eq!(
        preview.effects.accounts,
        [10, 30].map(|id| AccountId(fixture::id(id).0))
    );
    assert_eq!(
        preview.effects.records,
        [fixture::id(58_010), RecordId(replacement.0)]
    );
    assert_eq!(f.prefix().await, before.accounting.as_ref().unwrap().prefix);

    assert_eq!(
        f.service
            .submit(f.ctx(), body.clone(), f.auth(None))
            .await
            .unwrap_err()
            .class,
        ErrorClass::Unauthorized
    );
    let token_without_typed_binding = f.approve(&body);
    assert_eq!(
        f.service
            .submit(f.ctx(), body.clone(), token_without_typed_binding)
            .await
            .unwrap_err()
            .class,
        ErrorClass::Unauthorized,
        "legacy nonce and closure are not evidence that the exact typed candidate digest was approved"
    );
    assert_eq!(f.prefix().await, typed.basis);
    let substituted_proof = host.proof(&f, substituted.candidate_digest);
    assert_eq!(
        f.service
            .submit(f.ctx(), body.clone(), substituted_proof)
            .await
            .unwrap_err()
            .class,
        ErrorClass::Unauthorized,
        "valid digest for a different immutable revision cannot authorize the original"
    );
    assert_eq!(f.prefix().await, typed.basis);
    let mut edited = body.clone();
    let OperationV1::CorrectRecognition(request) = &mut edited.operation else {
        panic!()
    };
    request.reason.push_str(" altered after approval");
    let token = host.proof(&f, typed.candidate_digest);
    assert!(f.service.submit(f.ctx(), edited, token).await.is_err());
    let mut stale = body.clone();
    stale.expected_head = CommitSeq(before.through.0 - 1);
    assert_eq!(
        f.service.preview(f.ctx(), stale).unwrap_err().class,
        ErrorClass::StaleBasis
    );
    assert_eq!(f.prefix().await, before.accounting.as_ref().unwrap().prefix);

    let result = f
        .service
        .submit(
            f.ctx(),
            body.clone(),
            host.proof(&f, typed.candidate_digest),
        )
        .await
        .expect("host-approved synthetic correction");
    assert_eq!(result.state, CommandState::Committed);
    assert_eq!(result.commit, Some(CommitSeq(before.through.0 + 1)));
    assert_eq!(result.effects, preview.effects);
    f.service.close().await.unwrap();

    let store = open(&path).await;
    let commit = store
        .history()
        .read_prefix(result.commit.unwrap())
        .await
        .unwrap()
        .pop()
        .unwrap();
    let [decision] = commit.decisions.as_slice() else {
        panic!("real owner decision envelope")
    };
    assert_eq!(decision.decision_type, DecisionKind::Correction);
    assert_eq!(decision.authority, AuthorityKind::Owner);
    assert_eq!(decision.command_id, body.command_id);
    assert_eq!(decision.command_digest, preview.digest);
    assert_eq!(decision.adopted_result, preview.effects.records);
    assert_eq!(decision.policy, typed.policy);
    assert_eq!(
        commit.records[..commit.records.len() - 1],
        typed.plan.records
    );

    store
        .history()
        .verify_checkpoint(before.through, before.domain_digest)
        .await
        .unwrap();
    let fresh = open(&dir.path().join("approved-rebuild.db")).await;
    let replay = ReplayCoordinator::rebuild(store.history(), fresh)
        .await
        .unwrap();
    assert_eq!(replay, store.history().audit_report().await.unwrap());
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, &path, None).await;
    assert_eq!(
        current.records[&original_adoption],
        before.records[&original_adoption]
    );
    assert_eq!(
        current.records[&decision.id],
        AuthoritativeRecordV1::Decision(decision.clone())
    );
    let index = current.accounting.as_ref().unwrap();
    assert!(index.reversed.contains(&original));
    assert!(!index.reversed.contains(&replacement));
    assert!(index.recognitions.contains_key(&replacement));
}

#[tokio::test]
async fn cur_016_2_typed_candidate_pins_authenticated_policy_and_revision() {
    use service_api::{SyntheticCorrectionCandidate, synthetic_preview_correction_candidate};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed-correction-candidate.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    publish_legacy(&mut f, &path, 200, "1", "2026-11-01").await;
    let purchase = fixture::posted(legacy_binding(200), 300, 30);
    let original = purchase.financial.as_ref().unwrap().recognition_id;
    let adopted_policy = purchase.governing_policy.clone();
    let adoption = purchase.adoption_id;
    submit(&mut f, 59_001, OperationV1::AdoptActivity(purchase)).await;
    let before = refresh_financial(&mut f, &path, None).await;
    let mut later_policy =
        before.dataset_policies[&(adopted_policy.id, adopted_policy.version)].clone();
    later_policy.version += 1;
    let accounting = AccountingIntent::NoEffect(NoEffectV1 {
        reason: "retain later synthetic policy".into(),
    });
    let frozen = AccountingAdapter::freeze(&accounting, &before).unwrap();
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let writer = store.writer();
    writer
        .commit(PreparedCommit {
            evidence: vec![],
            decision_plan: DecisionPlan {
                records: vec![AuthoritativeRecordV1::DatasetPolicy(later_policy.clone())],
                invalidations: vec![],
                accounting,
            },
            frozen_batch: frozen,
            prior_head: before.through,
            decisions: vec![],
            result: CommandResultV1 {
                command_id: CommandId(fixture::id(59_004).0),
                digest: Digest([85; 32]),
                state: CommandState::Committed,
                commit: None,
                effects: EffectsV1 {
                    records: vec![later_policy.id],
                    accounts: vec![],
                    amounts: vec![],
                    invalidations: vec![],
                },
                health: vec![],
            },
            manifest_id: fixture::id(59_005),
            control: Arc::new(CommitControl::default()),
        })
        .await
        .unwrap();

    let current = writer.audited_prefix().await.unwrap();
    let domain_request = fresh_correction(&before, original, 59_020);
    let candidate = SyntheticCorrectionCandidate {
        dataset: DatasetId(support::id(1000).0),
        command: CommandId(fixture::id(59_002).0),
        expected_basis: current.clone(),
        revision: fixture::id(59_010),
        active_recognition: original,
        replacement: domain_request.replacement[0].clone(),
        reason: domain_request.reason.clone(),
        governing_policy: adopted_policy.clone(),
    };
    let preview = synthetic_preview_correction_candidate(writer, &candidate).await
        .expect("exact same-writer candidate must bind the adopted policy even after a newer policy is retained");
    let same = synthetic_preview_correction_candidate(writer, &candidate.clone())
        .await
        .unwrap();
    assert_eq!(preview, same);
    assert_eq!(
        preview.plan,
        CorrectionEngine::correct_durable(&before, candidate.revision, &domain_request).unwrap()
    );
    assert_eq!(preview.basis, current);
    assert_eq!(preview.policy, adopted_policy);
    let mut altered = candidate.clone();
    altered.revision = fixture::id(59_011);
    let second = synthetic_preview_correction_candidate(writer, &altered)
        .await
        .unwrap();
    assert_ne!(second.candidate_digest, preview.candidate_digest);
    assert_ne!(second.plan.records, preview.plan.records);

    let mut stale = candidate.clone();
    stale.expected_basis = before.accounting.as_ref().unwrap().prefix.clone();
    assert_eq!(
        synthetic_preview_correction_candidate(writer, &stale)
            .await
            .unwrap_err()
            .class,
        ErrorClass::StaleBasis
    );
    let mut forged_basis = candidate.clone();
    forged_basis.expected_basis.native_head.checkpoint = Digest([42; 32]);
    assert_eq!(
        synthetic_preview_correction_candidate(writer, &forged_basis)
            .await
            .unwrap_err()
            .class,
        ErrorClass::StaleBasis
    );
    let mut wrong = candidate.clone();
    wrong.governing_policy.version = later_policy.version;
    assert!(
        synthetic_preview_correction_candidate(writer, &wrong)
            .await
            .is_err()
    );
    let mut occupied = candidate.clone();
    occupied.revision = adoption;
    assert!(
        synthetic_preview_correction_candidate(writer, &occupied)
            .await
            .is_err()
    );
    let mut foreign = candidate.clone();
    foreign.dataset = DatasetId(fixture::id(59_012).0);
    assert!(
        synthetic_preview_correction_candidate(writer, &foreign)
            .await
            .is_err()
    );
    assert_eq!(writer.audited_prefix().await.unwrap(), current);
    store.close().await.unwrap();
    f.service = f.reopen().await;
    support::recover_service(&f.service).await;
    let mut body = f.body(59_002);
    body.expected_head = candidate.expected_basis.through;
    body.operation =
        OperationV1::CorrectRecognitionComplete(currents_protocol::CompleteCorrectionRequestV1 {
            schema_version: currents_protocol::ProtocolVersion,
            expected_basis: candidate.expected_basis,
            revision_id: candidate.revision,
            active_recognition: candidate.active_recognition,
            replacement: candidate.replacement,
            reason: candidate.reason,
            governing_policy: candidate.governing_policy,
        });
    assert_eq!(
        f.service.preview(f.ctx(), body).unwrap_err().class,
        ErrorClass::UnsupportedVersion,
        "structural negotiation must not activate the new operation in the unchanged planner"
    );
    assert_eq!(f.prefix().await, current);
}
