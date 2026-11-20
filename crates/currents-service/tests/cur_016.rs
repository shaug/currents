//! CUR-016 corrections against an actual committed and replayed accounting prefix.
use currents_service as service_api;
use currents_types::*;
#[path = "../../currents-domain/tests/support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "support/lifecycle.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/financial_service.rs"]
mod support_financial;
use support_financial::*;

#[tokio::test]
async fn cur_016_2_replayed_retirement_blocks_correction() {
    use currents_domain::correction::{CorrectionEngine, CorrectionRequestV1};
    use currents_protocol::OperationV1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("correction-guard.db");
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

    let retired = revision.retired_recognitions[0];
    let retained = revision.recognitions[0];
    let request_for = |id: RecognitionId| {
        let key = RecordId(id.0);
        let Some(AuthoritativeRecordV1::Recognition(original)) = old.records.get(&key) else {
            panic!("the original recognition must be authoritative before retirement");
        };
        assert_eq!(deduped.records.get(&key), old.records.get(&key));
        let mut replacement = original.clone();
        replacement.id = RecognitionId(fixture::id(9300).0);
        for (ordinal, posting) in replacement.postings.iter_mut().enumerate() {
            posting.id = fixture::id(9400 + u32::try_from(ordinal).unwrap());
        }
        CorrectionRequestV1 {
            active_recognition: id,
            replacement: vec![replacement],
            remapping: vec![],
            reason: "Correct the posting identities without changing economic activity".into(),
        }
    };
    let previously_active = request_for(retired);
    assert!(!old.accounting.as_ref().unwrap().reversed.contains(&retired));
    assert!(index.reversed.contains(&retired));

    let plan = CorrectionEngine::correct(&old, &previously_active).unwrap();
    assert!(matches!(
        plan.accounting,
        AccountingIntent::Correction(ref correction)
            if correction.active_recognition == retired
                && correction.replacement == previously_active.replacement
    ));
    let original_entries = &old.accounting.as_ref().unwrap().recognitions[&retired];
    let frozen = currents_accounting::AccountingAdapter::freeze(&plan.accounting, &old).unwrap();
    assert_eq!(frozen.operations.len(), original_entries.len() + 1);
    for (operation, original) in frozen.operations.iter().zip(original_entries) {
        let FrozenNativeOperationV1::Entry(reversal) = operation else {
            panic!("expected a whole native entry reversal");
        };
        assert_eq!(reversal.reverses.as_ref(), Some(&original.key));
        assert_eq!(reversal.booking_date, original.booking_date);
        assert_eq!(reversal.value_date, original.value_date);
        assert_eq!(reversal.postings.len(), original.postings.len());
        for (posting, prior) in reversal.postings.iter().zip(&original.postings) {
            assert_eq!(posting.original.id, prior.original.id);
            assert_eq!(posting.original.account_id, prior.original.account_id);
            let mut expected = prior.original.clone();
            expected.amount.coefficient = -expected.amount.coefficient;
            assert_eq!(posting.original, expected);
        }
    }
    let Some(FrozenNativeOperationV1::Entry(replacement)) = frozen.operations.last() else {
        panic!("expected the replacement after all reversals");
    };
    assert_eq!(
        replacement.recognition_id,
        previously_active.replacement[0].id
    );
    assert!(replacement.reverses.is_none());
    assert!(CorrectionEngine::correct(&deduped, &request_for(retained)).is_ok());

    assert_eq!(
        replacement.postings.len(),
        previously_active.replacement[0].postings.len()
    );
    for (frozen, expected) in replacement
        .postings
        .iter()
        .zip(&previously_active.replacement[0].postings)
    {
        let mut expected = expected.clone();
        expected.amount = expected.amount.normalized().unwrap();
        assert_eq!(frozen.original, expected);
    }

    let retained_request = request_for(retained);
    assert!(
        CorrectionEngine::correct_durable(&deduped, fixture::id(9699), &retained_request).is_err(),
        "a still-active recognition with association authority cannot use the durable standalone path"
    );
    let reject = |modify: fn(&mut CorrectionRequestV1)| {
        let mut request = retained_request.clone();
        modify(&mut request);
        assert!(CorrectionEngine::correct(&deduped, &request).is_err());
    };
    reject(|r| r.replacement.clear());
    reject(|r| r.replacement.push(r.replacement[0].clone()));
    reject(|r| {
        r.remapping.push(AssociationV1 {
            id: fixture::id(9600),
            kind: RelationshipKind::Duplicate,
            legs: vec![],
            allocations: vec![],
            survivor: None,
            corrections: vec![r.active_recognition],
            reason: "Requires explicit remapping support".into(),
        })
    });
    reject(|r| r.replacement[0].postings[0].account_id = AccountId(fixture::id(9601).0));
    reject(|r| r.replacement[0].postings[0].amount.coefficient += 1);
    reject(|r| r.replacement[0].postings[0].effective = fixture::date("2026-10-01"));
    reject(|r| r.replacement[0].lineage = fixture::id(9602));
    let mut wrong_prefix = deduped.clone();
    wrong_prefix
        .accounting
        .as_mut()
        .unwrap()
        .prefix
        .domain_digest = Digest([0; 32]);
    assert!(CorrectionEngine::correct(&wrong_prefix, &retained_request).is_err());

    let before = f.prefix().await;
    assert!(CorrectionEngine::correct(&deduped, &previously_active).is_err());
    let after = f.prefix().await;
    assert_eq!(after.through, before.through);
    assert_eq!(after.native_head, before.native_head);
}

#[tokio::test]
async fn cur_016_durable_correction_rebuilds_immutable_lineage() {
    use currents_accounting::AccountingAdapter;
    use currents_domain::correction::{CorrectionEngine, CorrectionRequestV1};
    use currents_protocol::OperationV1;
    use currents_service::ReplayCoordinator;
    use currents_storage::{CommitControl, PreparedCommit};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    publish_legacy(&mut f, &path, 200, "1", "2026-11-01").await;
    let activity = fixture::posted(legacy_binding(200), 300, 30);
    let original_id = activity.financial.as_ref().unwrap().recognition_id;
    submit(&mut f, 30200, OperationV1::AdoptActivity(activity.clone())).await;
    refresh_financial(&mut f, &path, None).await;
    publish_legacy(&mut f, &path, 220, "1", "2026-11-01").await;
    let unrelated = fixture::posted(legacy_binding(220), 320, 30);
    let unrelated_id = unrelated.financial.as_ref().unwrap().recognition_id;
    submit(&mut f, 30220, OperationV1::AdoptActivity(unrelated)).await;
    let old_basis = refresh_financial(&mut f, &path, None).await;
    let original_key = RecordId(original_id.0);
    let Some(AuthoritativeRecordV1::Recognition(original)) = old_basis.records.get(&original_key)
    else {
        panic!("expected a committed original recognition");
    };
    let mut replacement = original.clone();
    replacement.id = RecognitionId(fixture::id(9300).0);
    for (ordinal, posting) in replacement.postings.iter_mut().enumerate() {
        posting.id = fixture::id(9400 + u32::try_from(ordinal).unwrap());
    }
    let request = CorrectionRequestV1 {
        active_recognition: original_id,
        replacement: vec![replacement.clone()],
        remapping: vec![],
        reason: "Correct posting identities without changing the economic activity".into(),
    };
    let revision_id = fixture::id(9502);
    let plan = CorrectionEngine::correct_durable(&old_basis, revision_id, &request).unwrap();
    let frozen = AccountingAdapter::freeze(&plan.accounting, &old_basis).unwrap();
    assert_eq!(frozen.operations.len(), 2);
    let [
        FrozenNativeOperationV1::Entry(reversal),
        FrozenNativeOperationV1::Entry(emitted),
    ] = frozen.operations.as_slice()
    else {
        panic!("expected one whole reversal followed by one replacement");
    };
    assert_eq!(reversal.recognition_id, original_id);
    assert_eq!(emitted.recognition_id, replacement.id);
    assert_eq!(reversal.postings.len(), emitted.postings.len());
    for (reversed, adopted) in reversal.postings.iter().zip(&emitted.postings) {
        assert_eq!(reversed.original.account_id, adopted.original.account_id);
        assert_eq!(
            reversed.original.amount,
            adopted.original.amount.checked_neg().unwrap()
        );
    }

    f.service.close().await.unwrap();
    let source = open(&path).await;
    let trusted_tip = source.writer().audited_prefix().await.unwrap();
    currents_service::verify_correction_preflight(&old_basis, &trusted_tip, &plan, &frozen)
        .expect("exact authorized correction must pass preflight");

    let mut wrong_request = request.clone();
    wrong_request.replacement[0].postings[0].amount.coefficient -= 1;
    wrong_request.replacement[0].postings[1].amount.coefficient += 1;
    let wrong_frozen =
        AccountingAdapter::freeze(&AccountingIntent::Correction(wrong_request), &old_basis)
            .expect("balanced wrong economics must pass native shape checks");
    assert_eq!(wrong_frozen.preconditions, frozen.preconditions);
    assert_eq!(wrong_frozen.operations.len(), frozen.operations.len());
    assert_ne!(wrong_frozen, frozen);
    assert!(
        currents_service::verify_correction_preflight(
            &old_basis,
            &trusted_tip,
            &plan,
            &wrong_frozen,
        )
        .is_err(),
        "a self-consistent but unauthorized C05 payload must fail before writing"
    );
    assert_eq!(source.writer().audited_prefix().await.unwrap(), trusted_tip);

    let prepare = |control| PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id: CommandId(fixture::id(9500).0),
            digest: Digest([49; 32]),
            state: CommandState::Committed,
            commit: None,
            effects: EffectsV1 {
                records: vec![revision_id, RecordId(replacement.id.0)],
                accounts: vec![],
                amounts: vec![],
                invalidations: plan.invalidations.clone(),
            },
            health: vec![],
        },
        decision_plan: plan.clone(),
        frozen_batch: frozen.clone(),
        prior_head: old_basis.through,
        decisions: vec![],
        manifest_id: fixture::id(9501),
        control,
    };
    use currents_accounting::test_fixtures::{ClosureFault, fault_control};
    use currents_service::synthetic_commit_verified_correction;
    use currents_storage::CommitFailure;
    let writer = source.writer();
    let mut malformed = prepare(Arc::new(CommitControl::default()));
    malformed.frozen_batch = wrong_frozen;
    let malformed_control = malformed.control.clone();
    assert!(matches!(
        synthetic_commit_verified_correction(&writer, &old_basis, malformed).await,
        Err(CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(malformed_control.completed_operations(), 0);
    assert_eq!(writer.audited_prefix().await.unwrap(), trusted_tip);
    assert!(
        writer
            .command_result(CommandId(fixture::id(9500).0), Digest([49; 32]))
            .await
            .unwrap()
            .is_none()
    );
    let control = Arc::new(fault_control(ClosureFault::PauseBetweenCorrectionEntries));
    let cancel_between_entries = async {
        while control.completed_operations() == 0 {
            tokio::task::yield_now().await;
        }
        let completed = control.completed_operations();
        assert!(control.request_cancel());
        completed
    };
    let (outcome, completed) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(
            synthetic_commit_verified_correction(&writer, &old_basis, prepare(control.clone())),
            cancel_between_entries
        )
    })
    .await
    .expect("synthetic cancellation checkpoint not reached");
    assert_eq!(
        completed, 1,
        "fault must be after reversal, before replacement"
    );
    assert!(matches!(
        outcome,
        Err(CommitFailure::DefinitelyRolledBack(EngineError {
            class: ErrorClass::Canceled,
            ..
        }))
    ));
    assert_eq!(writer.audited_prefix().await.unwrap(), trusted_tip);
    source.close().await.unwrap();
    let source = open(&path).await;
    assert_eq!(source.writer().audited_prefix().await.unwrap(), trusted_tip);
    assert!(
        source
            .writer()
            .command_result(CommandId(fixture::id(9500).0), Digest([49; 32]))
            .await
            .unwrap()
            .is_none()
    );
    let writer = source.writer();
    let commit = synthetic_commit_verified_correction(
        &writer,
        &old_basis,
        prepare(Arc::new(CommitControl::default())),
    )
    .await
    .expect("C04 retry must atomically commit both native and typed authority");
    let retry = synthetic_commit_verified_correction(
        &writer,
        &old_basis,
        prepare(Arc::new(CommitControl::default())),
    )
    .await
    .expect("identical retry must resolve the existing correction");
    assert_eq!(retry.commit, commit.commit);
    assert_eq!(retry.domain_digest, commit.domain_digest);
    assert_eq!(retry.native_head, commit.native_head);
    assert_eq!(commit.commit, CommitSeq(old_basis.through.0 + 1));
    assert_ne!(
        commit.native_head,
        old_basis.accounting.as_ref().unwrap().prefix.native_head
    );
    let stored = source
        .history()
        .read_prefix(commit.commit)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let old_index = old_basis.accounting.as_ref().unwrap();
    let valid_batch =
        AccountingAdapter::freeze(&AccountingIntent::Correction(request.clone()), &old_basis)
            .unwrap();
    for mutation in 0..3 {
        let mut bad_batch = valid_batch.clone();
        match mutation {
            0 => bad_batch.operations.swap(0, 1),
            1 => {
                bad_batch.operations.remove(0);
            }
            _ => {
                bad_batch
                    .operations
                    .insert(0, bad_batch.operations[0].clone());
            }
        }
        assert!(
            AccountingAdapter::verify_correspondence(&stored, Some(&bad_batch), old_index).is_err()
        );
    }
    for malformed in 0..8 {
        let mut candidate = stored.clone();
        match malformed {
            0 => {
                candidate.records.remove(0);
            }
            1 => candidate.records.swap(0, 1),
            2 => {
                let AuthoritativeRecordV1::RecognitionCorrectionRevision(revision) =
                    &mut candidate.records[0]
                else {
                    panic!("correction owner missing")
                };
                revision.schema_version += 1;
            }
            3 => candidate.records.insert(0, candidate.records[0].clone()),
            4 => {
                let AuthoritativeRecordV1::RecognitionCorrectionRevision(revision) =
                    &mut candidate.records[0]
                else {
                    panic!("correction owner missing")
                };
                revision.prior_recognition = unrelated_id;
            }
            5..=7 => {
                let AuthoritativeRecordV1::Recognition(replacement) = &mut candidate.records[1]
                else {
                    panic!("replacement missing")
                };
                match malformed {
                    5 => {
                        replacement.evidence_allocations[0].observation =
                            legacy_binding(220).observation
                    }
                    6 => {
                        replacement.postings[0].amount.coefficient -= 1;
                        replacement.postings[1].amount.coefficient += 1;
                    }
                    _ => {
                        // Remains a coherent civil-date entry, but not the accepted original.
                        let day = fixture::date("2026-10-31");
                        replacement.bookkeeping_date = day.clone();
                        for p in &mut replacement.postings {
                            p.posting = day.clone();
                            p.effective = day.clone();
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
        let mut unchanged = old_basis.clone();
        assert!(currents_domain::ReplayEngine::apply(&mut unchanged, &candidate).is_err());
        assert_eq!(unchanged.through, old_basis.through);
        assert_eq!(unchanged.records, old_basis.records);
    }
    assert_eq!(
        source.writer().audited_prefix().await.unwrap().through,
        commit.commit
    );
    let destination = open(&dir.path().join("independent-rebuild.db")).await;
    let report = ReplayCoordinator::rebuild(source.history(), destination)
        .await
        .expect("C08 must re-admit the correction against its exact prior prefix");
    assert_eq!(report.through, commit.commit);
    assert_eq!(report.native_head, commit.native_head);
    source.close().await.unwrap();

    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, &path, None).await;
    for key in [activity.adoption_id, original_key] {
        assert_eq!(current.records.get(&key), old_basis.records.get(&key));
    }
    assert_eq!(
        current.records.get(&RecordId(replacement.id.0)),
        Some(&AuthoritativeRecordV1::Recognition(replacement.clone()))
    );
    assert!(matches!(
        current.records.get(&revision_id),
        Some(AuthoritativeRecordV1::RecognitionCorrectionRevision(revision))
            if revision.prior_recognition == original_id
                && revision.replacement_recognition == replacement.id
                && revision.remap_disposition == RecognitionCorrectionRemapV1::UnchangedStableLegs
    ));
    let index = current.accounting.as_ref().unwrap();
    assert!(index.reversed.contains(&original_id));
    assert!(CorrectionEngine::correct_durable(&current, fixture::id(9700), &request).is_err());
    assert!(index.recognitions.contains_key(&replacement.id));
    assert!(!index.reversed.contains(&replacement.id));
    assert!(
        !old_basis
            .accounting
            .as_ref()
            .unwrap()
            .reversed
            .contains(&original_id)
    );
}
