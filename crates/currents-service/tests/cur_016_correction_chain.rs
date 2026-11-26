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
    declare_template(&mut f, &fixture::monetary_state()).await;
    publish_legacy(&mut f, 200, "1", "2026-11-01").await;
    let purchase = fixture::posted(legacy_binding(200), 300, 30);
    let root = purchase.financial.as_ref().unwrap().recognition_id;
    let adoption = purchase.adoption_id;
    submit(&mut f, 53_001, OperationV1::AdoptActivity(purchase)).await;
    let initial = refresh_financial(&mut f, None).await;

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
    let once = refresh_financial(&mut f, None).await;
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
    let current = refresh_financial(&mut f, None).await;
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
    declare_template(&mut f, &fixture::monetary_state()).await;
    publish_legacy(&mut f, 200, "1", "2026-11-01").await;
    let purchase = fixture::posted(legacy_binding(200), 300, 30);
    let purchase_id = purchase.financial.as_ref().unwrap().recognition_id;
    let purchase_event = purchase.event.id;
    submit(&mut f, 54_001, OperationV1::AdoptActivity(purchase)).await;
    let before = refresh_financial(&mut f, None).await;

    let serial = 220;
    let capture = FileParser::parse(
        BoundedEvidence::new(
            "refund.csv",
            b"original_id,amount,unit,effective,description\nsynthetic-refund,96,USD,2026-11-07,separate refund\n".to_vec(),
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
        ).unwrap(),
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
    ).unwrap();
    let evidence = ImportEvidence::new(capture).unwrap();
    let import = evidence.request();
    refresh_financial(&mut f, Some(evidence)).await;
    submit(&mut f, 54_002, OperationV1::PublishImport(import)).await;
    refresh_financial(&mut f, None).await;
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
    let after = refresh_financial(&mut f, None).await;
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
