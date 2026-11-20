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
        synthetic_commit_verified_correction(writer, malformed).await,
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
            synthetic_commit_verified_correction(writer, prepare(control.clone())),
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
    let commit =
        synthetic_commit_verified_correction(writer, prepare(Arc::new(CommitControl::default())))
            .await
            .expect("C04 retry must atomically commit both native and typed authority");
    let retry =
        synthetic_commit_verified_correction(writer, prepare(Arc::new(CommitControl::default())))
            .await
            .expect("identical retry must resolve the existing correction");
    assert_eq!(retry.commit, commit.commit);
    assert_eq!(retry.domain_digest, commit.domain_digest);
    assert_eq!(retry.native_head, commit.native_head);
    let persisted = writer.audited_prefix().await.unwrap();
    let mut altered_retry = prepare(Arc::new(CommitControl::default()));
    altered_retry
        .result
        .effects
        .amounts
        .push(fixture::amount(1));
    let retry_control = altered_retry.control.clone();
    assert!(matches!(
        synthetic_commit_verified_correction(writer, altered_retry).await,
        Err(CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(retry_control.completed_operations(), 0);
    assert_eq!(writer.audited_prefix().await.unwrap(), persisted);

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

/// Retain a second read-only synthetic balance assertion using the same
/// encrypted evidence shape as the established bootstrap fixture.
async fn retain_segment_assertion(
    store: &currents_storage::EncryptedStore,
    serial: u32,
    coefficient: i128,
    date: &str,
    kind: ObservationKind,
) -> SourceFacetBindingV1 {
    let mut scratch = DomainState::default();
    let source = fixture::evidence(&mut scratch, serial, coefficient, None);
    let day = fixture::date(date);
    let mut observation = scratch.records.remove(&source.observation).unwrap();
    let mut product = scratch.records.remove(&source.parse_product).unwrap();
    let AuthoritativeRecordV1::ObservationVersion(o) = &mut observation else {
        panic!()
    };
    let AuthoritativeRecordV1::AddressedParseProduct(p) = &mut product else {
        panic!()
    };
    let text = format!("Synthetic segment assertion: {coefficient} USD on {date}");
    let content = store.stage(text.as_bytes()).await.unwrap();
    o.temporal = day.clone();
    o.semantic_kind = kind;
    o.original_ids = vec![format!("standalone-segment-evidence-{serial}")];
    o.sanitized_representation = text.clone();
    o.content_digest = content.digest();
    p.product.facets[0].kind = kind;
    p.product.facets[0].temporal = day;
    let receipt = ImportReceiptV1 {
        id: fixture::id(serial + 2),
        source: o.source.clone(),
        adapter_version: "cur-016-synthetic-balance/1".into(),
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
            byte_count: CommitSeq(text.len() as u64),
            media_type: "text/plain".into(),
        }],
    };
    let writer = store.writer();
    let prefix = writer.audited_prefix().await.unwrap();
    writer
        .append(currents_storage::HistoryAppend {
            prior_head: prefix.through,
            records: vec![
                AuthoritativeRecordV1::ImportReceipt(receipt),
                observation,
                product,
            ],
            decisions: vec![],
            staged: vec![content],
            native: None,
            result: CommandResultV1 {
                command_id: CommandId(fixture::id(50_001 + serial).0),
                digest: Digest([55; 32]),
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
        writer.audited_prefix().await.unwrap().native_head,
        prefix.native_head
    );
    source
}

#[tokio::test]
async fn cur_016_standalone_segment_commit_reopens_and_rebuilds_native_authority() {
    use currents_accounting::AccountingAdapter;
    use currents_domain::segments::{SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1};
    use currents_protocol::OperationV1;
    use currents_service::{ReplayCoordinator, synthetic_commit_verified_segment};
    use currents_storage::{CommitControl, PreparedCommit};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("standalone-segment.db");
    let (mut f, _) = bootstrap_with_balance(&path, true).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    let initial = legacy_binding(250);
    let prior_segment = fixture::id(707);
    let component = fixture::id(11);
    let mut old_draft = fixture::posted(initial.clone(), 700, 50);
    let terms = old_draft.financial.as_mut().unwrap();
    let old_opening = terms.recognition_id;
    terms.opening = Some(OpeningBasisV1 {
        segment_id: prior_segment,
        component_id: component,
        amount: initial.amount,
        cutoff: fixture::date("2026-11-01"),
        evidence: initial,
        included_activity: vec![],
    });
    submit(&mut f, 50_002, OperationV1::AdoptActivity(old_draft)).await;
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let balance =
        retain_segment_assertion(&store, 810, 96, "2026-10-30", ObservationKind::Balance).await;
    let movement =
        retain_segment_assertion(&store, 820, -192, "2026-10-31", ObservationKind::Movement).await;
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let old = refresh_financial(&mut f, &path, None).await;

    let dated = |source, serial, counter, day| {
        let mut draft = fixture::posted(source, serial, counter);
        let date = fixture::date(day);
        draft.legs[0].leg.effective = date.clone();
        draft.legs[0].leg.posting = date.clone();
        let terms = draft.financial.as_mut().unwrap();
        terms.bookkeeping_date = date.clone();
        for posting in &mut terms.postings {
            posting.effective = date.clone();
            posting.posting = date.clone();
        }
        draft
    };
    let new_segment = fixture::id(907);
    let mut opening = dated(balance.clone(), 900, 50, "2026-10-30");
    opening.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: new_segment,
        component_id: component,
        amount: balance.amount,
        cutoff: fixture::date("2026-10-30"),
        evidence: balance,
        included_activity: vec![],
    });
    let new_opening = RecognitionId(fixture::id(903).0);
    let backfill = RecognitionId(fixture::id(923).0);
    let proposal = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component,
            prior_segment: Some(prior_segment),
            opening: new_opening,
            included_activity: vec![backfill],
            replacement_closure: vec![old_opening],
            approved_accounts: [10, 30, 50]
                .into_iter()
                .map(|n| AccountId(fixture::id(n).0))
                .collect(),
            residual_disposition: SourceValue::Absent,
        },
        opening,
        backfill: vec![dated(movement, 920, 30, "2026-10-31")],
        reemissions: vec![],
    };
    let SegmentPreviewV1::Ready(plan) = SegmentEngine::replace(&old, &proposal).unwrap() else {
        panic!("96 - 192 must equal the old -96 boundary");
    };
    let frozen = AccountingAdapter::freeze(&plan.accounting, &old).unwrap();
    assert_eq!(frozen.operations.len(), 3);
    let effects = EffectsV1 {
        records: plan
            .records
            .iter()
            .map(|record| match record {
                AuthoritativeRecordV1::FinancialEvent(e) => e.id,
                AuthoritativeRecordV1::FinancialLeg(l) => RecordId(l.id.0),
                AuthoritativeRecordV1::ActivityAdoption(a) => a.id,
                AuthoritativeRecordV1::Recognition(r) => RecordId(r.id.0),
                AuthoritativeRecordV1::AccountingSegment(s) => s.id,
                _ => panic!("unexpected record for standalone segment"),
            })
            .collect(),
        accounts: vec![],
        amounts: vec![],
        invalidations: plan.invalidations.clone(),
    };
    let make_prepared = || PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id: CommandId(fixture::id(50_003).0),
            digest: Digest([56; 32]),
            state: CommandState::Committed,
            commit: None,
            effects: effects.clone(),
            health: vec![],
        },
        decision_plan: plan.as_ref().clone(),
        frozen_batch: frozen.clone(),
        prior_head: old.through,
        decisions: vec![],
        manifest_id: fixture::id(50_004),
        control: Arc::new(CommitControl::default()),
    };
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let writer = store.writer();
    let before = writer.audited_prefix().await.unwrap();
    // Forged local state stamps itself with the true authority tip. Both
    // drafts still reconcile, but their amounts were never retained on disk.
    let mut forged_prior = old.clone();
    let mut forged_proposal = proposal.clone();
    let adjust_retained_source =
        |state: &mut DomainState, mut source: SourceFacetBindingV1, delta| {
            let adjusted = source.amount.checked_add(&fixture::amount(delta)).unwrap();
            let AuthoritativeRecordV1::AddressedParseProduct(product) =
                state.records.get_mut(&source.parse_product).unwrap()
            else {
                panic!("fixture parse product missing");
            };
            product.product.facets[0].amount = SourceValue::Known(adjusted);
            source.amount = adjusted;
            source
        };
    let changed_opening = adjust_retained_source(
        &mut forged_prior,
        proposal.opening.legs[0].sources[0].clone(),
        10,
    );
    let mut forged_opening = dated(changed_opening.clone(), 900, 50, "2026-10-30");
    let mut forged_basis = proposal
        .opening
        .financial
        .as_ref()
        .unwrap()
        .opening
        .clone()
        .unwrap();
    forged_basis.amount = changed_opening.amount;
    forged_basis.evidence = changed_opening;
    forged_opening.financial.as_mut().unwrap().opening = Some(forged_basis);
    forged_proposal.opening = forged_opening;
    let changed_movement = adjust_retained_source(
        &mut forged_prior,
        proposal.backfill[0].legs[0].sources[0].clone(),
        -10,
    );
    forged_proposal.backfill = vec![dated(changed_movement, 920, 30, "2026-10-31")];
    assert_eq!(forged_prior.through, old.through);
    assert_eq!(forged_prior.domain_digest, old.domain_digest);
    assert_eq!(forged_prior.accounting.as_ref().unwrap().prefix, before);
    let SegmentPreviewV1::Ready(forged_plan) =
        SegmentEngine::replace(&forged_prior, &forged_proposal).unwrap()
    else {
        panic!("forged amounts deliberately still reconcile");
    };
    let mut forged = make_prepared();
    forged.frozen_batch =
        AccountingAdapter::freeze(&forged_plan.accounting, &forged_prior).unwrap();
    forged.decision_plan = *forged_plan;
    let forged_control = forged.control.clone();
    let outcome = synthetic_commit_verified_segment(writer, &forged_proposal, forged).await;
    if outcome.is_ok() {
        let witness = open(&dir.path().join("forged-replay.db")).await;
        assert!(
            ReplayCoordinator::rebuild(store.history(), witness)
                .await
                .is_err()
        );
    }
    assert!(
        matches!(
            outcome,
            Err(currents_storage::CommitFailure::DefinitelyRolledBack(_))
        ),
        "a caller cannot authenticate edited retained evidence by copying the correct prefix stamp"
    );
    assert_eq!(forged_control.completed_operations(), 0);
    assert_eq!(writer.audited_prefix().await.unwrap(), before);

    let mut wrong_prefix = make_prepared();
    wrong_prefix.frozen_batch.preconditions.domain_digest = Digest([57; 32]);
    let wrong_prefix_control = wrong_prefix.control.clone();
    assert!(matches!(
        synthetic_commit_verified_segment(writer, &proposal, wrong_prefix).await,
        Err(currents_storage::CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(wrong_prefix_control.completed_operations(), 0);
    assert_eq!(writer.audited_prefix().await.unwrap(), before);

    let mut wrong_intent = plan.accounting.clone();
    let AccountingIntent::Corrections(corrections) = &mut wrong_intent else {
        panic!();
    };
    let postings = &mut corrections[0].replacement[1].postings;
    postings[0].amount.coefficient -= 1;
    postings[1].amount.coefficient += 1;
    let mut malformed = make_prepared();
    malformed.frozen_batch = AccountingAdapter::freeze(&wrong_intent, &old)
        .expect("balanced but wrong C05 payload is natively freezeable");
    let malformed_control = malformed.control.clone();
    assert!(matches!(
        synthetic_commit_verified_segment(writer, &proposal, malformed).await,
        Err(currents_storage::CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(malformed_control.completed_operations(), 0);
    assert_eq!(writer.audited_prefix().await.unwrap(), before);
    assert!(
        writer
            .command_result(CommandId(fixture::id(50_003).0), Digest([56; 32]))
            .await
            .unwrap()
            .is_none()
    );

    let receipt = synthetic_commit_verified_segment(writer, &proposal, make_prepared())
        .await
        .expect("trusted C08/C05 segment preflight must reach the real C04 commit");
    assert_eq!(receipt.commit, CommitSeq(before.through.0 + 1));
    assert_ne!(receipt.native_head, before.native_head);
    let committed_prefix = writer.audited_prefix().await.unwrap();
    assert!(matches!(
        synthetic_commit_verified_segment(writer, &proposal, make_prepared()).await,
        Err(currents_storage::CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(writer.audited_prefix().await.unwrap(), committed_prefix);

    store.close().await.unwrap();

    let store = open(&path).await;
    let reopened = store.writer().audited_prefix().await.unwrap();
    assert_eq!(reopened.through, receipt.commit);
    assert_eq!(reopened.native_head, receipt.native_head);
    let fresh = open(&dir.path().join("standalone-segment-rebuild.db")).await;
    let report = ReplayCoordinator::rebuild(store.history(), fresh)
        .await
        .unwrap();
    assert_eq!(report.through, receipt.commit);
    assert_eq!(report.native_head, reopened.native_head);
    store.close().await.unwrap();

    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, &path, None).await;
    for (id, record) in &old.records {
        assert_eq!(current.records.get(id), Some(record));
    }
    let Some(AuthoritativeRecordV1::AccountingSegment(active)) = current.records.get(&new_segment)
    else {
        panic!("reopened projection lost the new segment");
    };
    assert_eq!(active.prior_segment, Some(prior_segment));
    assert_eq!(active.opening, new_opening);
    assert_eq!(active.included_activity, vec![backfill]);
    let index = current.accounting.as_ref().unwrap();
    assert!(index.reversed.contains(&old_opening));
    for id in [new_opening, backfill] {
        assert!(index.recognitions.contains_key(&id));
        assert!(!index.reversed.contains(&id));
    }
}

#[path = "../../currents-domain/tests/support/associations.rs"]
#[allow(dead_code)]
mod transfer_fixture;

#[tokio::test]
async fn cur_016_1_encrypted_cross_account_segment_commits_full_transfer_family() {
    use currents_accounting::AccountingAdapter;
    use currents_domain::segments::{
        SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1, SegmentReemissionV1,
    };
    use currents_protocol::OperationV1;
    use currents_service::{ImportEvidence, ReplayCoordinator, synthetic_commit_verified_segment};
    use currents_storage::{CommitControl, PreparedCommit};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cross-account-segment.db");
    let (mut f, _) = bootstrap_with_balance(&path, true).await;
    let (template, mut transfer) = transfer_fixture::transfer_request();
    declare_template(&mut f, &path, &template).await;

    let initial = legacy_binding(250);
    let prior_segment = fixture::id(707);
    let component = fixture::id(11);
    let mut original_opening = fixture::posted(initial.clone(), 700, 50);
    let terms = original_opening.financial.as_mut().unwrap();
    let old_opening = terms.recognition_id;
    terms.opening = Some(OpeningBasisV1 {
        segment_id: prior_segment,
        component_id: component,
        amount: initial.amount,
        cutoff: fixture::date("2026-11-01"),
        evidence: initial,
        included_activity: vec![],
    });
    submit(&mut f, 51_001, OperationV1::AdoptActivity(original_opening)).await;
    refresh_financial(&mut f, &path, None).await;

    // Same fixture adapter and capture IDs as the real C15 transfer route.
    let rows = transfer_fixture::boundary_rows("2026-11-03", false);
    let capture = currents_acquisition::identity::RelationshipSourceParser::capture(
        currents_acquisition::capture::BoundedEvidence::new(
            "relationship.json",
            serde_json::to_vec(&rows).unwrap(),
            ImportContextV1 {
                receipt_id: fixture::id(1500),
                source: SourceIdentityV1 {
                    source: "synthetic".into(),
                    connection_id: fixture::id(900),
                    device_id: None,
                },
                captured_at: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
                received_at: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
                coverage: vec![],
                row_ids: vec![],
            },
        )
        .unwrap(),
        fixture::id(1501),
    )
    .unwrap();
    let evidence = ImportEvidence::new_relationship(capture).unwrap();
    let import = evidence.request();
    refresh_financial(&mut f, &path, Some(evidence)).await;
    submit(&mut f, 51_002, OperationV1::PublishImport(import)).await;
    refresh_financial(&mut f, &path, None).await;
    transfer_fixture::association_date(&mut transfer, "2026-11-03");
    submit(
        &mut f,
        51_003,
        OperationV1::AssociateLegsComplete(transfer.clone()),
    )
    .await;
    f.service.close().await.unwrap();

    let store = open(&path).await;
    let balance =
        retain_segment_assertion(&store, 810, 96, "2026-10-30", ObservationKind::Balance).await;
    let movement =
        retain_segment_assertion(&store, 820, -192, "2026-10-31", ObservationKind::Movement).await;
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let before_dependents = refresh_financial(&mut f, &path, None).await;
    let affected_claim = fixture::id(51_101);
    let unchanged_claim = fixture::id(51_102);
    let retained_note = fixture::id(51_103);
    let claim = |id, day| {
        let TemporalValue::Date { civil, .. } = fixture::date(day) else {
            panic!("fixture date")
        };
        AuthoritativeRecordV1::ReconciliationClaim(ReconciliationClaimV1 {
            id,
            comparison_type: "Activity".into(),
            scope: vec![AccountId(fixture::id(10).0)],
            cutoffs: DateScope {
                start: Some(civil),
                end: Some(civil),
                basis: DateBasis::Effective,
            },
            evidence_dependencies: vec![],
            decision_dependencies: vec![],
            dependency_digest: Digest([91; 32]),
            matched_allocations: vec![],
            differences: vec![],
            exclusions: vec![],
            status: ReconciliationStatus::Matched,
            reason: "Bounded synthetic segment comparison".into(),
        })
    };
    let note = AuthoritativeRecordV1::Organization(OrganizationRecordV1 {
        id: retained_note,
        version: 1,
        kind: OrganizationKind::Note,
        targets: vec![RecordId(transfer.revision.legs[0].0)],
        parent: None,
        text: "Stable transfer leg note".into(),
        allocations: vec![],
        supersedes: None,
    });
    let accounting = AccountingIntent::NoEffect(NoEffectV1 {
        reason: "retain synthetic segment dependents".into(),
    });
    let frozen_batch = AccountingAdapter::freeze(&accounting, &before_dependents).unwrap();
    f.service.close().await.unwrap();
    let store = open(&path).await;
    store
        .writer()
        .commit(PreparedCommit {
            evidence: vec![],
            decision_plan: DecisionPlan {
                records: vec![
                    claim(affected_claim, "2026-10-31"),
                    claim(unchanged_claim, "2026-11-03"),
                    note.clone(),
                ],
                invalidations: vec![],
                accounting,
            },
            frozen_batch,
            prior_head: before_dependents.through,
            decisions: vec![],
            result: CommandResultV1 {
                command_id: CommandId(fixture::id(51_104).0),
                digest: Digest([92; 32]),
                state: CommandState::Committed,
                commit: None,
                effects: EffectsV1 {
                    records: vec![affected_claim, unchanged_claim, retained_note],
                    accounts: vec![],
                    amounts: vec![],
                    invalidations: vec![],
                },
                health: vec![],
            },
            manifest_id: fixture::id(51_105),
            control: Arc::new(CommitControl::default()),
        })
        .await
        .unwrap();
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let old = refresh_financial(&mut f, &path, None).await;
    assert_eq!(old.records[&retained_note], note);
    assert_eq!(
        old.reconciliation_status(affected_claim),
        Some(ReconciliationStatus::Matched)
    );
    let dated = |source, serial, counter, day| {
        let mut draft = fixture::posted(source, serial, counter);
        let at = fixture::date(day);
        draft.legs[0].leg.effective = at.clone();
        draft.legs[0].leg.posting = at.clone();
        let terms = draft.financial.as_mut().unwrap();
        terms.bookkeeping_date = at.clone();
        for posting in &mut terms.postings {
            posting.effective = at.clone();
            posting.posting = at.clone();
        }
        draft
    };
    let target_segment = fixture::id(907);
    let mut opening = dated(balance.clone(), 900, 50, "2026-10-30");
    opening.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: target_segment,
        component_id: component,
        amount: balance.amount,
        cutoff: fixture::date("2026-10-30"),
        evidence: balance,
        included_activity: vec![],
    });
    let covered: Vec<_> = std::iter::once(old_opening)
        .chain(transfer.revision.recognitions.iter().copied())
        .collect();
    let reemissions = transfer
        .revision
        .recognitions
        .iter()
        .enumerate()
        .map(|(i, original)| {
            let fresh = 1070 + 10 * u32::try_from(i).unwrap();
            let AuthoritativeRecordV1::Recognition(r) = &old.records[&RecordId(original.0)] else {
                panic!("original native recognition not retained");
            };
            SegmentReemissionV1 {
                original: *original,
                replacement: RecognitionId(fixture::id(fresh).0),
                posting_ids: r
                    .postings
                    .iter()
                    .enumerate()
                    .map(|(j, _)| fixture::id(fresh + 1 + u32::try_from(j).unwrap()))
                    .collect(),
            }
        })
        .collect();
    let proposal = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component,
            prior_segment: Some(prior_segment),
            opening: RecognitionId(fixture::id(903).0),
            included_activity: vec![RecognitionId(fixture::id(923).0)],
            replacement_closure: covered.clone(),
            approved_accounts: [10, 30, 50]
                .into_iter()
                .map(|n| AccountId(fixture::id(n).0))
                .collect(),
            residual_disposition: SourceValue::Absent,
        },
        opening,
        backfill: vec![dated(movement, 920, 30, "2026-10-31")],
        reemissions,
    };
    let authority = fixture::id(941);
    let SegmentPreviewV1::Ready(plan) =
        SegmentEngine::replace_owned(&old, &proposal, authority).unwrap()
    else {
        panic!("the cross-account owner's earlier balance reconciles");
    };
    assert!(plan.invalidations.contains(&affected_claim));
    assert!(!plan.invalidations.contains(&unchanged_claim));
    assert!(!plan.invalidations.contains(&retained_note));
    let mut unsupported_note = old.clone();
    let Some(AuthoritativeRecordV1::Recognition(old_anchor)) =
        old.records.get(&RecordId(old_opening.0))
    else {
        panic!("prior anchor remains authenticated")
    };
    let Some(AuthoritativeRecordV1::Organization(changed_note)) =
        unsupported_note.records.get_mut(&retained_note)
    else {
        panic!("note remains authenticated")
    };
    changed_note.targets = old_anchor
        .leg_ids
        .iter()
        .map(|leg| RecordId(leg.0))
        .collect();
    assert!(!changed_note.targets.is_empty());
    assert_eq!(
        SegmentEngine::replace_owned(&unsupported_note, &proposal, authority)
            .unwrap_err()
            .class,
        EngineError::version().class,
        "a note targeting retired old-anchor legs may not be silently detached or remapped"
    );
    let frozen = AccountingAdapter::freeze(&plan.accounting, &old).unwrap();
    assert_eq!(frozen.operations.len(), 11);
    let effect_records: Vec<_> = plan
        .records
        .iter()
        .map(|record| match record {
            AuthoritativeRecordV1::FinancialEvent(e) => e.id,
            AuthoritativeRecordV1::FinancialLeg(l) => RecordId(l.id.0),
            AuthoritativeRecordV1::ActivityAdoption(a) => a.id,
            AuthoritativeRecordV1::Recognition(r) => RecordId(r.id.0),
            AuthoritativeRecordV1::AccountingSegment(s) => s.id,
            AuthoritativeRecordV1::SegmentReplacementAuthority(a) => a.id,
            _ => panic!("unexpected record in owned segment plan"),
        })
        .collect();
    let Some(AuthoritativeRecordV1::SegmentReplacementAuthority(owner)) = plan.records.last()
    else {
        panic!("the affected accounts belong to the durable owner");
    };
    let make_prepared = || PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id: CommandId(fixture::id(51_004).0),
            digest: Digest([61; 32]),
            state: CommandState::Committed,
            commit: None,
            effects: EffectsV1 {
                records: effect_records.clone(),
                accounts: owner.affected_accounts.clone(),
                amounts: vec![],
                invalidations: plan.invalidations.clone(),
            },
            health: vec![],
        },
        decision_plan: plan.as_ref().clone(),
        frozen_batch: frozen.clone(),
        prior_head: old.through,
        decisions: vec![],
        manifest_id: fixture::id(51_005),
        control: Arc::new(CommitControl::default()),
    };

    f.service.close().await.unwrap();
    let store = open(&path).await;
    let writer = store.writer();
    let prior = writer.audited_prefix().await.unwrap();
    let mut incomplete = proposal.clone();
    let removed = incomplete.reemissions.remove(1).original;
    incomplete
        .request
        .replacement_closure
        .retain(|id| *id != removed);
    let rejected = make_prepared();
    let control = rejected.control.clone();
    assert!(
        synthetic_commit_verified_segment(writer, &incomplete, rejected)
            .await
            .is_err()
    );
    assert_eq!(control.completed_operations(), 0);
    assert_eq!(writer.audited_prefix().await.unwrap(), prior);

    let mut forged_affected = make_prepared();
    let Some(AuthoritativeRecordV1::SegmentReplacementAuthority(forged_owner)) =
        forged_affected.decision_plan.records.last_mut()
    else {
        panic!("durable owner missing");
    };
    forged_owner.affected_accounts.pop();
    let control = forged_affected.control.clone();
    assert!(
        synthetic_commit_verified_segment(writer, &proposal, forged_affected)
            .await
            .is_err()
    );
    assert_eq!(control.completed_operations(), 0);
    assert_eq!(writer.audited_prefix().await.unwrap(), prior);
    assert!(
        writer
            .command_result(CommandId(fixture::id(51_004).0), Digest([61; 32]))
            .await
            .unwrap()
            .is_none()
    );

    let mut forged_invalidations = make_prepared();
    forged_invalidations
        .decision_plan
        .invalidations
        .retain(|id| *id != affected_claim);
    forged_invalidations.result.effects.invalidations =
        forged_invalidations.decision_plan.invalidations.clone();
    assert!(
        synthetic_commit_verified_segment(writer, &proposal, forged_invalidations)
            .await
            .is_err()
    );
    assert_eq!(writer.audited_prefix().await.unwrap(), prior);

    let receipt = synthetic_commit_verified_segment(writer, &proposal, make_prepared())
        .await
        .expect("same-writer authority must commit all eleven cross-account operations together");
    assert_eq!(receipt.commit, CommitSeq(old.through.0 + 1));
    assert_ne!(receipt.native_head, prior.native_head);
    store.close().await.unwrap();
    let store = open(&path).await;
    let reopened = store.writer().audited_prefix().await.unwrap();
    assert_eq!(reopened.native_head, receipt.native_head);
    assert_eq!(reopened.domain_digest, receipt.domain_digest);
    let history = store.history();
    history
        .verify_checkpoint(old.through, old.domain_digest)
        .await
        .unwrap();
    let committed = history
        .read_prefix(receipt.commit)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let mut forged = committed.clone();
    forged
        .command_result
        .effects
        .invalidations
        .retain(|id| *id != affected_claim);
    forged
        .command_result
        .effects
        .invalidations
        .push(unchanged_claim);
    let mut unchanged = old.clone();
    assert!(currents_domain::ReplayEngine::apply(&mut unchanged, &forged).is_err());
    assert_eq!(unchanged.through, old.through);
    assert_eq!(
        unchanged.reconciliation_status(affected_claim),
        Some(ReconciliationStatus::Matched)
    );
    let Some(AuthoritativeRecordV1::FrozenAccountingManifest(manifest)) = committed.records.last()
    else {
        panic!("actual authority commit must retain its exact native closure");
    };
    assert_eq!(
        manifest.result_expectations.native_operation_count,
        CommitSeq(11)
    );
    assert_eq!(receipt.result.effects.accounts, owner.affected_accounts);
    let destination = open(&dir.path().join("fresh-cross-account-rebuild.db")).await;
    let report = ReplayCoordinator::rebuild(store.history(), destination)
        .await
        .unwrap();
    assert_eq!(report.through, receipt.commit);
    assert_eq!(report.native_head, receipt.native_head);
    store.close().await.unwrap();

    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, &path, None).await;
    assert_eq!(
        old.reconciliation_status(affected_claim),
        Some(ReconciliationStatus::Matched)
    );
    assert_eq!(
        current.reconciliation_status(affected_claim),
        Some(ReconciliationStatus::Invalidated)
    );
    assert_eq!(
        current.reconciliation_status(unchanged_claim),
        Some(ReconciliationStatus::Matched)
    );
    assert_eq!(current.records[&retained_note], note);
    for (id, archived) in &old.records {
        assert_eq!(current.records.get(id), Some(archived));
    }
    assert_eq!(
        current.records.get(&transfer.revision.id),
        Some(&AuthoritativeRecordV1::AssociationRevision(
            transfer.revision.clone()
        ))
    );
    let Some(AuthoritativeRecordV1::SegmentReplacementAuthority(authority)) =
        current.records.get(&authority)
    else {
        panic!("rebuild must retain explicit current counterpart authority");
    };
    assert_eq!(authority.segment, target_segment);
    assert_eq!(
        authority.affected_accounts,
        [10, 30, 50, 60, 70, 80]
            .into_iter()
            .map(|n| AccountId(fixture::id(n).0))
            .collect::<Vec<_>>()
    );
    let before = old.accounting.as_ref().unwrap();
    let after = current.accounting.as_ref().unwrap();
    for id in covered {
        assert!(!before.reversed.contains(&id));
        assert!(after.reversed.contains(&id));
    }
    for mapping in &proposal.reemissions {
        let [old] = before.recognitions[&mapping.original].as_slice() else {
            panic!()
        };
        let [new] = after.recognitions[&mapping.replacement].as_slice() else {
            panic!()
        };
        assert_eq!(new.booking_date, old.booking_date);
        assert_eq!(new.value_date, old.value_date);
        assert_eq!(new.postings.len(), old.postings.len());
        for ((before, after), new_id) in old
            .postings
            .iter()
            .zip(&new.postings)
            .zip(&mapping.posting_ids)
        {
            let mut preserved = before.clone();
            preserved.original.id = *new_id;
            assert_eq!(after, &preserved);
        }
    }
    currents_domain::associations::IdentityEngine::audit(&current).unwrap();
}

/// Narrow acceptance-witness report from verified authoritative native rows.
/// Not the general CUR-017 position/query surface.
fn historical_balance_for_witness(
    state: &DomainState,
    basis: &ReadBasis,
    account: AccountId,
) -> SourceValue<Amount> {
    assert_eq!(state.through, basis.through_commit);
    assert_eq!(basis.effective_scope.basis, DateBasis::Effective);
    let index = state.accounting.as_ref().unwrap();
    assert_eq!(index.prefix.through, basis.through_commit);
    let end = basis.effective_scope.end.unwrap().to_string();
    let mut uses_anchor = false;
    let mut balance = fixture::amount(0);
    for (id, entries) in &index.recognitions {
        if index.reversed.contains(id) {
            continue;
        }
        let AuthoritativeRecordV1::Recognition(authority) = &state.records[&RecordId(id.0)] else {
            panic!("frozen recognition is missing typed authority");
        };
        for entry in entries.iter().filter(|e| e.value_date <= end) {
            for posting in entry
                .postings
                .iter()
                .filter(|p| p.original.account_id == account)
            {
                if authority.kind == RecognitionKind::Opening {
                    assert!(!uses_anchor, "this fixture has one active opening on A");
                    uses_anchor = true;
                }
                balance = balance.checked_add(&posting.original.amount).unwrap();
            }
        }
    }
    if uses_anchor {
        SourceValue::Known(balance.normalized().unwrap())
    } else {
        SourceValue::Unknown
    }
}

#[tokio::test]
async fn cur_016_3_2024_date_floor_preserves_old_basis_and_repairs_current_as_of() {
    use currents_accounting::AccountingAdapter;
    use currents_domain::segments::{SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1};
    use currents_domain::{ReplayEngine, recognition::RecognitionEngine};
    use currents_protocol::{OperationV1, TrustedClock};
    use currents_service::{ReplayCoordinator, synthetic_commit_verified_segment};
    use currents_storage::{CommitControl, PreparedCommit};
    use std::sync::Arc;

    let journey: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/journeys/s4/date-floor.json"
    ))
    .unwrap();
    let text = |pointer: &str| journey.pointer(pointer).unwrap().as_str().unwrap();
    let amount = |pointer: &str| journey.pointer(pointer).unwrap().as_i64().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("historical-date-floor.db");
    let (mut f, _) = bootstrap_with_balance(&path, true).await;
    let now: RecordedInstant = serde_json::from_value(journey["declaration_at"].clone()).unwrap();
    f.clock
        .advance(now.0.unix_timestamp() - f.clock.now().unix_timestamp());
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    let account = AccountId(fixture::id(10).0);
    let component = fixture::id(11);
    let prior_segment = fixture::id(707);
    let initial = legacy_binding(250);
    let mut opening_2026 = fixture::posted(initial.clone(), 700, 50);
    let terms = opening_2026.financial.as_mut().unwrap();
    let old_opening = terms.recognition_id;
    terms.opening = Some(OpeningBasisV1 {
        segment_id: prior_segment,
        component_id: component,
        amount: initial.amount,
        cutoff: fixture::date(text("/prior_cutoff")),
        evidence: initial,
        included_activity: vec![],
    });
    submit(&mut f, 52_001, OperationV1::AdoptActivity(opening_2026)).await;
    f.service.close().await.unwrap();

    let store = open(&path).await;
    let balance = retain_segment_assertion(
        &store,
        810,
        amount("/anchor/amount").into(),
        text("/anchor/date"),
        ObservationKind::Balance,
    )
    .await;
    let movement = retain_segment_assertion(
        &store,
        820,
        amount("/movement/amount").into(),
        text("/movement/effective"),
        ObservationKind::Movement,
    )
    .await;
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let old = refresh_financial(&mut f, &path, None).await;
    let declared = old
        .records
        .values()
        .find_map(|record| match record {
            AuthoritativeRecordV1::AccountCatalogRevision(r)
                if r.account_id == account && r.prior_revision.is_none() =>
            {
                Some(r.id)
            }
            _ => None,
        })
        .unwrap();
    assert!(
        old.records.values().any(|record| matches!(record,
            AuthoritativeRecordV1::Decision(d)
                if d.adopted_result.contains(&declared) && d.recorded_time.0.year() == 2026
        )),
        "real account declaration decision must be recorded in 2026"
    );
    let TemporalValue::Date { civil: as_of, .. } = fixture::date("2024-10-31") else {
        panic!()
    };
    let old_basis = ReadBasis {
        dataset_id: DatasetId(support::id(1000).0),
        through_commit: old.through,
        effective_scope: DateScope {
            start: None,
            end: Some(as_of),
            basis: DateBasis::Effective,
        },
        evaluation_instant: now.0,
        query_semantics_version: 1,
        selection_policy_version: 1,
        unit_catalog_version: 1,
    };
    assert_eq!(
        historical_balance_for_witness(&old, &old_basis, account),
        SourceValue::Unknown
    );

    let dated = |source, serial, counter, effective, booking| {
        let mut draft = fixture::posted(source, serial, counter);
        let effective = fixture::date(effective);
        let booking = fixture::date(booking);
        draft.legs[0].leg.effective = effective.clone();
        draft.legs[0].leg.posting = booking.clone();
        let terms = draft.financial.as_mut().unwrap();
        terms.bookkeeping_date = booking.clone();
        for posting in &mut terms.postings {
            posting.effective = effective.clone();
            posting.posting = booking.clone();
        }
        draft
    };
    let mut earlier = dated(
        balance.clone(),
        900,
        50,
        text("/anchor/date"),
        text("/anchor/date"),
    );
    earlier.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: fixture::id(907),
        component_id: component,
        amount: balance.amount,
        cutoff: fixture::date(text("/anchor/date")),
        evidence: balance,
        included_activity: vec![],
    });
    let backfill = RecognitionId(fixture::id(923).0);
    let proposal = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component,
            prior_segment: Some(prior_segment),
            opening: RecognitionId(fixture::id(903).0),
            included_activity: vec![backfill],
            replacement_closure: vec![old_opening],
            approved_accounts: [10, 30, 50]
                .into_iter()
                .map(|n| AccountId(fixture::id(n).0))
                .collect(),
            residual_disposition: SourceValue::Absent,
        },
        opening: earlier,
        backfill: vec![dated(
            movement,
            920,
            30,
            text("/movement/effective"),
            text("/movement/booking"),
        )],
        reemissions: vec![],
    };
    let SegmentPreviewV1::Ready(plan) = SegmentEngine::replace(&old, &proposal).unwrap() else {
        panic!("legitimate 2024 opening and movement must reconcile");
    };
    let frozen = AccountingAdapter::freeze(&plan.accounting, &old).unwrap();
    assert_eq!(frozen.operations.len(), 3);
    assert!(
        frozen
            .operations
            .iter()
            .all(|op| matches!(op, FrozenNativeOperationV1::Entry(_)))
    );

    assert!(
        frozen
            .operations
            .iter()
            .any(|op| matches!(op, FrozenNativeOperationV1::Entry(e)
        if e.recognition_id == backfill && e.value_date == "2024-10-31"
            && e.booking_date == "2024-11-02"))
    );
    let effects = EffectsV1 {
        records: plan
            .records
            .iter()
            .map(|r| match r {
                AuthoritativeRecordV1::FinancialEvent(e) => e.id,
                AuthoritativeRecordV1::FinancialLeg(l) => RecordId(l.id.0),
                AuthoritativeRecordV1::ActivityAdoption(a) => a.id,
                AuthoritativeRecordV1::Recognition(r) => RecordId(r.id.0),
                AuthoritativeRecordV1::AccountingSegment(s) => s.id,
                _ => panic!("unexpected historical plan record"),
            })
            .collect(),
        accounts: vec![],
        amounts: vec![],
        invalidations: plan.invalidations.clone(),
    };
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let writer = store.writer();
    let committed = synthetic_commit_verified_segment(
        writer,
        &proposal,
        PreparedCommit {
            evidence: vec![],
            result: CommandResultV1 {
                command_id: CommandId(fixture::id(52_002).0),
                digest: Digest([71; 32]),
                state: CommandState::Committed,
                commit: None,
                effects,
                health: vec![],
            },
            decision_plan: *plan,
            frozen_batch: frozen,
            prior_head: old.through,
            decisions: vec![],
            manifest_id: fixture::id(52_003),
            control: Arc::new(CommitControl::default()),
        },
    )
    .await
    .expect("native account floor allows older dates after a later domain declaration");

    // Independently reconstruct the old encrypted prefix *after* the accepted
    // backfill, not merely reread the pre-write in-memory state.
    let source = store.history();
    source
        .verify_checkpoint(old.through, old.domain_digest)
        .await
        .unwrap();
    let old_rebuild = open(&dir.path().join("prior-as-of.db")).await;
    let genesis = old_rebuild.writer().audited_prefix().await.unwrap();
    let mut historical = DomainState {
        accounting: Some(AccountingIndexV1::empty(genesis.native_head)),
        ..DomainState::default()
    };
    let mut stream = source
        .stream_prefix(old_basis.through_commit)
        .await
        .unwrap();
    while let Some(commit) = stream.next().await.unwrap() {
        let mut next = historical.clone();
        ReplayEngine::apply(&mut next, &commit).unwrap();
        next.accounting = Some(
            source
                .rebuild_commit(&old_rebuild, &historical, &commit)
                .await
                .unwrap(),
        );
        RecognitionEngine::audit(&next).unwrap();
        historical = next;
    }
    drop(stream);
    assert_eq!(
        old_rebuild.writer().audited_prefix().await.unwrap(),
        old.accounting.as_ref().unwrap().prefix
    );
    assert_eq!(historical.records, old.records);
    assert_eq!(
        historical_balance_for_witness(&historical, &old_basis, account),
        SourceValue::Unknown
    );
    old_rebuild.close().await.unwrap();
    let current_rebuild = open(&dir.path().join("current-as-of.db")).await;
    let report = ReplayCoordinator::rebuild(source, current_rebuild)
        .await
        .unwrap();
    assert_eq!(report.native_head, committed.native_head);
    store.close().await.unwrap();

    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, &path, None).await;
    let current_basis = ReadBasis {
        through_commit: committed.commit,
        ..old_basis.clone()
    };
    assert_eq!(current_basis.effective_scope, old_basis.effective_scope);
    assert_eq!(
        historical_balance_for_witness(&current, &current_basis, account),
        SourceValue::Known(fixture::amount(-96))
    );
    let index = current.accounting.as_ref().unwrap();
    let [entry] = index.recognitions[&backfill].as_slice() else {
        panic!()
    };
    assert_eq!(entry.value_date, "2024-10-31");
    assert_eq!(entry.booking_date, "2024-11-02");
    assert!(!index.reversed.contains(&backfill));
    for posting in &entry.postings {
        assert_eq!(posting.original.effective, fixture::date("2024-10-31"));
        assert_eq!(posting.original.posting, fixture::date("2024-11-02"));
    }
    assert_eq!(current.records.get(&declared), old.records.get(&declared));
}

#[tokio::test]
async fn cur_016_4_real_1200_operation_replacement_is_atomic_at_c04_midpoint() {
    use currents_accounting::{
        AccountingAdapter,
        test_fixtures::{ClosureFault, fault_control},
    };
    use currents_acquisition::{capture::BoundedEvidence, parse::FileParser};
    use currents_domain::segments::{SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1};
    use currents_protocol::OperationV1;
    use currents_service::{ImportEvidence, ReplayCoordinator, synthetic_commit_verified_segment};
    use currents_storage::{CommitControl, CommitFailure, PreparedCommit};
    use std::{sync::Arc, time::Duration};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("planner-1200.db");
    let (mut f, _) = bootstrap_with_balance(&path, true).await;
    declare_template(&mut f, &path, &fixture::monetary_state()).await;
    let initial = legacy_binding(250);
    let prior_segment = fixture::id(707);
    let component = fixture::id(11);
    let mut old_opening = fixture::posted(initial.clone(), 700, 50);
    let terms = old_opening.financial.as_mut().unwrap();
    let root = terms.recognition_id;
    terms.opening = Some(OpeningBasisV1 {
        segment_id: prior_segment,
        component_id: component,
        amount: initial.amount,
        cutoff: fixture::date("2026-11-01"),
        evidence: initial,
        included_activity: vec![],
    });
    submit(&mut f, 55_001, OperationV1::AdoptActivity(old_opening)).await;
    refresh_financial(&mut f, &path, None).await;

    let mut csv = String::from("original_id,amount,unit,effective,description\n");
    let mut row_ids = Vec::new();
    let mut movements = Vec::new();
    for i in 0..1198 {
        use std::fmt::Write;
        writeln!(
            &mut csv,
            "bulk-{i},-1,USD,2026-10-31,synthetic-closed-movement"
        )
        .unwrap();
        let serial = 100_000 + 4 * i;
        let row = ImportRowIdsV1 {
            observation: fixture::id(serial),
            product: fixture::id(serial + 1),
            facet: fixture::id(serial + 2),
        };
        movements.push(SourceFacetBindingV1 {
            observation: row.observation,
            parse_product: row.product,
            facet: row.facet,
            amount: fixture::amount(-1),
        });
        row_ids.push(row);
    }
    let capture = FileParser::parse(
        BoundedEvidence::new(
            "bulk.csv",
            csv.into_bytes(),
            ImportContextV1 {
                receipt_id: fixture::id(55_010),
                source: SourceIdentityV1 {
                    source: "synthetic".into(),
                    connection_id: fixture::id(102),
                    device_id: None,
                },
                captured_at: serde_json::from_str("\"2026-11-01T12:00:00Z\"").unwrap(),
                received_at: serde_json::from_str("\"2026-11-02T12:00:00Z\"").unwrap(),
                coverage: vec![],
                row_ids,
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
    submit(&mut f, 55_002, OperationV1::PublishImport(import)).await;
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let balance =
        retain_segment_assertion(&store, 810, 1102, "2026-10-30", ObservationKind::Balance).await;
    store.close().await.unwrap();
    f.service = f.reopen().await;
    // Only one independent initial replay is needed. We do not submit
    // another protocol command, so don't reinstall the fixture planner.
    support::recover_service(&f.service).await;
    let old = f.service.verified_projection().unwrap();

    let dated = |source, serial, counter, day| {
        let mut draft = fixture::posted(source, serial, counter);
        let date = fixture::date(day);
        draft.legs[0].leg.effective = date.clone();
        draft.legs[0].leg.posting = date.clone();
        let terms = draft.financial.as_mut().unwrap();
        terms.bookkeeping_date = date.clone();
        for posting in &mut terms.postings {
            posting.effective = date.clone();
            posting.posting = date.clone();
        }
        draft
    };
    let mut opening = dated(balance.clone(), 900, 50, "2026-10-30");
    opening.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: fixture::id(907),
        component_id: component,
        amount: balance.amount,
        cutoff: fixture::date("2026-10-30"),
        evidence: balance,
        included_activity: vec![],
    });
    let backfill: Vec<_> = movements
        .into_iter()
        .enumerate()
        .map(|(i, movement)| {
            dated(
                movement,
                1_000_000 + 10 * u32::try_from(i).unwrap(),
                30,
                "2026-10-31",
            )
        })
        .collect();
    let proposal = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component,
            prior_segment: Some(prior_segment),
            opening: opening.financial.as_ref().unwrap().recognition_id,
            included_activity: backfill
                .iter()
                .map(|r| r.financial.as_ref().unwrap().recognition_id)
                .collect(),
            replacement_closure: vec![root],
            approved_accounts: [10, 30, 50]
                .into_iter()
                .map(|n| AccountId(fixture::id(n).0))
                .collect(),
            residual_disposition: SourceValue::Absent,
        },
        opening,
        backfill,
        reemissions: vec![],
    };
    let SegmentPreviewV1::Ready(plan) = SegmentEngine::replace(&old, &proposal).unwrap() else {
        panic!("1102 minus 1198 must reconcile to the real retained -96 anchor");
    };
    let frozen = AccountingAdapter::freeze(&plan.accounting, &old).unwrap();
    assert_eq!(frozen.operations.len(), 1200);
    eprintln!("authenticated retained prefix planned and froze 1200 operations");
    let effects = EffectsV1 {
        records: plan
            .records
            .iter()
            .map(|r| match r {
                AuthoritativeRecordV1::FinancialEvent(e) => e.id,
                AuthoritativeRecordV1::FinancialLeg(l) => RecordId(l.id.0),
                AuthoritativeRecordV1::ActivityAdoption(a) => a.id,
                AuthoritativeRecordV1::Recognition(r) => RecordId(r.id.0),
                AuthoritativeRecordV1::AccountingSegment(s) => s.id,
                _ => panic!("unexpected typed bulk segment record"),
            })
            .collect(),
        accounts: vec![],
        amounts: vec![],
        invalidations: plan.invalidations.clone(),
    };
    let command = CommandId(fixture::id(55_003).0);
    let digest = Digest([79; 32]);
    let prepare = |control| PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id: command,
            digest,
            state: CommandState::Committed,
            commit: None,
            effects: effects.clone(),
            health: vec![],
        },
        decision_plan: plan.as_ref().clone(),
        frozen_batch: frozen.clone(),
        prior_head: old.through,
        decisions: vec![],
        manifest_id: fixture::id(55_004),
        control,
    };

    f.service.close().await.unwrap();
    let store = open(&path).await;
    let writer = store.writer();
    let before = writer.audited_prefix().await.unwrap();
    let control = Arc::new(fault_control(ClosureFault::PauseAfter600));
    let observe = async {
        while control.completed_operations() < 600 {
            tokio::task::yield_now().await;
        }
        assert_eq!(control.completed_operations(), 600);
        let still_old = store.history().audit_report().await.unwrap();
        assert_eq!(still_old.through, before.through);
        assert_eq!(still_old.native_head, before.native_head);
        assert!(control.request_cancel());
        eprintln!("at 600 applied operations, independent reader still saw the old head");
    };
    let attempt = synthetic_commit_verified_segment(writer, &proposal, prepare(control.clone()));
    let result = tokio::time::timeout(Duration::from_secs(240), async {
        tokio::pin!(attempt);
        tokio::select! {
            result = &mut attempt => result,
            () = observe => attempt.await,
        }
    })
    .await
    .expect("planner-generated C04 midpoint must be reached");
    assert!(
        matches!(
            result,
            Err(CommitFailure::DefinitelyRolledBack(EngineError {
                class: ErrorClass::Canceled,
                ..
            }))
        ),
        "the real writer must cancel after the first 600 operations, got {:?}",
        result.as_ref().err()
    );
    assert_eq!(control.completed_operations(), 600);
    assert_eq!(writer.audited_prefix().await.unwrap(), before);
    assert!(
        writer
            .command_result(command, digest)
            .await
            .unwrap()
            .is_none()
    );
    store.close().await.unwrap();

    let store = open(&path).await;
    assert_eq!(store.writer().audited_prefix().await.unwrap(), before);
    let committed = synthetic_commit_verified_segment(
        store.writer(),
        &proposal,
        prepare(Arc::new(CommitControl::default())),
    )
    .await
    .expect("complete 1200-op replacement must commit once after rolled-back attempt");
    assert_eq!(committed.commit, CommitSeq(before.through.0 + 1));
    let history = store.history();
    let last = store
        .writer()
        .command_result(command, digest)
        .await
        .unwrap()
        .unwrap();
    let Some(AuthoritativeRecordV1::FrozenAccountingManifest(manifest)) = last.records.last()
    else {
        panic!("committed manifest missing");
    };
    assert_eq!(
        manifest.result_expectations.native_operation_count,
        CommitSeq(1200)
    );
    let fresh = open(&dir.path().join("bulk-fresh-rebuild.db")).await;
    let rebuilt = ReplayCoordinator::rebuild(history, fresh).await.unwrap();
    assert_eq!(rebuilt.native_head, committed.native_head);
}
