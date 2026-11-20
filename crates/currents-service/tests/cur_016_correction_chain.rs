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
