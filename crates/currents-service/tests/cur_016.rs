//! CUR-016 corrections against an actual committed and replayed accounting prefix.
use currents_domain::segments::{
    SegmentEngine, SegmentPreviewV1, SegmentReplacementV1, TransferJoinV1,
};
use currents_service as service_api;
use currents_types::*;
#[path = "../../currents-domain/tests/support/associations.rs"]
#[allow(dead_code)]
mod association_fixtures;
#[path = "../../currents-domain/tests/support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "support/lifecycle.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/financial_service.rs"]
mod support_financial;
#[path = "../../currents-domain/tests/support/transfers.rs"]
mod transfers;
use support_financial::*;

#[tokio::test]
async fn cur_016_2_replayed_retirement_blocks_correction() {
    use currents_domain::correction::{CorrectionEngine, CorrectionRequestV1};
    use currents_protocol::OperationV1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("correction-guard.db");
    let (mut f, _) = bootstrap(&path).await;
    declare_template(&mut f, &fixture::monetary_state()).await;
    let mut requests = Vec::new();
    for (n, day) in [
        (200, "2026-11-01"),
        (220, "2026-11-02"),
        (240, "2026-11-03"),
    ] {
        publish_legacy(&mut f, n, "1", day).await;
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
        refresh_financial(&mut f, None).await;
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
    let deduped = refresh_financial(&mut f, None).await;
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
        r.remapping.push(OrganizationRecordV1 {
            id: fixture::id(9600),
            version: 2,
            kind: OrganizationKind::Note,
            targets: vec![],
            parent: None,
            text: "Requires a leg correction".into(),
            allocations: vec![],
            supersedes: Some(fixture::id(9603)),
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
    declare_template(&mut f, &fixture::monetary_state()).await;
    publish_legacy(&mut f, 200, "1", "2026-11-01").await;
    let activity = fixture::posted(legacy_binding(200), 300, 30);
    let original_id = activity.financial.as_ref().unwrap().recognition_id;
    submit(&mut f, 30200, OperationV1::AdoptActivity(activity.clone())).await;
    refresh_financial(&mut f, None).await;
    publish_legacy(&mut f, 220, "1", "2026-11-01").await;
    let unrelated = fixture::posted(legacy_binding(220), 320, 30);
    let unrelated_id = unrelated.financial.as_ref().unwrap().recognition_id;
    submit(&mut f, 30220, OperationV1::AdoptActivity(unrelated)).await;
    let old_basis = refresh_financial(&mut f, None).await;
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
    let current = refresh_financial(&mut f, None).await;
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

const ACCOUNT_A: u32 = 10;
const ACCOUNT_B: u32 = 20;
const EQUITY: u32 = 50;
const EXPENSE: u32 = 30;
const DIGEST: Digest = Digest([77; 32]);

fn account(n: u32) -> AccountId {
    AccountId(fixture::id(n).0)
}

/// The binding of evidence retained at genesis as observation `n`.
fn retained(n: u32, value: i128) -> SourceFacetBindingV1 {
    SourceFacetBindingV1 {
        observation: fixture::id(n),
        parse_product: fixture::id(n + 1),
        facet: fixture::id(900),
        amount: fixture::amount(value),
    }
}

/// A posted draft on account `on` with its own value and booking dates.
fn dated_draft(
    source: SourceFacetBindingV1,
    n: u32,
    on: u32,
    counter: u32,
    effective: &str,
    booking: &str,
) -> ActivityRequestV1 {
    let mut draft = fixture::posted(source, n, counter);
    let (effective, booking) = (fixture::date(effective), fixture::date(booking));
    let leg = &mut draft.legs[0].leg;
    leg.account_id = account(on);
    leg.effective = effective.clone();
    leg.posting = booking.clone();
    let terms = draft.financial.as_mut().unwrap();
    terms.bookkeeping_date = booking.clone();
    terms.postings[0].account_id = account(on);
    for posting in &mut terms.postings {
        posting.effective = effective.clone();
        posting.posting = booking.clone();
    }
    draft
}

/// A posted opening of account `on`'s component at `cutoff`, naming `segment`.
fn opening_draft(
    source: SourceFacetBindingV1,
    n: u32,
    on: u32,
    segment: u32,
    cutoff: &str,
) -> ActivityRequestV1 {
    let mut draft = dated_draft(source.clone(), n, on, EQUITY, cutoff, cutoff);
    draft.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: fixture::id(segment),
        component_id: fixture::id(on + 1),
        amount: source.amount,
        cutoff: fixture::date(cutoff),
        evidence: source,
        included_activity: vec![],
    });
    draft
}

/// A replacement of segment 707 on account `on`: an opening from evidence
/// `base` (draft `base + 10`, successor `base + 17`) and one backfill to the
/// expense account from evidence `base + 100` (draft `base + 110`).
fn replacement_request(
    on: u32,
    base: u32,
    opening: (i128, &str),
    backfill: (i128, &str, &str),
) -> SegmentReplacementV1 {
    SegmentReplacementV1 {
        prior_segment: fixture::id(707),
        opening: opening_draft(
            retained(base, opening.0),
            base + 10,
            on,
            base + 17,
            opening.1,
        ),
        backfill: vec![dated_draft(
            retained(base + 100, backfill.0),
            base + 110,
            on,
            EXPENSE,
            backfill.1,
            backfill.2,
        )],
        approved_accounts: vec![account(on), account(EQUITY), account(EXPENSE)],
        transfers: vec![],
        reemissions: vec![],
    }
}

fn ready_plan(
    state: &DomainState,
    request: &SegmentReplacementV1,
) -> (DecisionPlan, AccountingBatchV1) {
    let SegmentPreviewV1::Ready(plan) = SegmentEngine::replace(state, request).unwrap() else {
        panic!("the replacement must reconcile with the old opening");
    };
    let frozen = currents_accounting::AccountingAdapter::freeze(&plan.accounting, state).unwrap();
    (*plan, frozen)
}

/// The commit input for `plan` under command `command`, naming every emitted
/// record in the result.
fn segment_commit(
    plan: &DecisionPlan,
    frozen: &AccountingBatchV1,
    command: u32,
    control: currents_storage::CommitControl,
) -> currents_storage::PreparedCommit {
    let records = plan
        .records
        .iter()
        .map(|r| {
            serde_json::from_value(serde_json::to_value(r).unwrap()["value"]["id"].clone()).unwrap()
        })
        .collect();
    currents_storage::PreparedCommit {
        evidence: vec![],
        result: CommandResultV1 {
            command_id: CommandId(fixture::id(command).0),
            digest: DIGEST,
            state: CommandState::Committed,
            commit: None,
            effects: EffectsV1 {
                records,
                accounts: vec![],
                amounts: vec![],
                invalidations: plan.invalidations.clone(),
            },
            health: vec![],
        },
        decision_plan: plan.clone(),
        frozen_batch: frozen.clone(),
        prior_head: frozen.preconditions.through,
        decisions: vec![],
        manifest_id: fixture::id(command + 1),
        control: std::sync::Arc::new(control),
    }
}

/// Account A anchored at 3,300 USD at the end of 2026-11-01 through the real
/// encrypted service, with retained evidence for the replacement in the domain
/// test `cur_016_1_replacement_proves_old_boundary`: a 3,400 USD balance at
/// 2026-10-30 and a -100 USD movement on 2026-10-31.
async fn anchored_a(path: &std::path::Path) -> (support::Fixture, DomainState) {
    anchored_with(
        path,
        &[
            (3000, 3400, "2026-10-30", ObservationKind::Balance),
            (3100, -100, "2026-10-31", ObservationKind::Movement),
        ],
    )
    .await
}

/// Account A anchored as in `anchored_a`, retaining `evidence` as
/// `(observation, amount, date, kind)` for a later replacement.
async fn anchored_with(
    path: &std::path::Path,
    evidence: &[(u32, i128, &str, ObservationKind)],
) -> (support::Fixture, DomainState) {
    let evidence: Vec<_> = [(2900, 3300, "2026-11-01", ObservationKind::Balance)]
        .into_iter()
        .chain(evidence.iter().copied())
        .collect();
    let texts: Vec<_> = evidence
        .iter()
        .map(|(n, value, day, _)| format!("Synthetic evidence {n}: {value} USD on {day}"))
        .collect();
    let retained_evidence: Vec<_> = evidence
        .iter()
        .zip(&texts)
        .map(|(&(n, value, day, kind), text)| RetainedEvidence {
            n,
            value,
            day,
            kind,
            text,
        })
        .collect();
    let (mut f, _) = bootstrap_with_evidence(path, &retained_evidence).await;
    declare_template(&mut f, &fixture::monetary_state()).await;
    let opening = opening_draft(retained(2900, 3300), 700, ACCOUNT_A, 707, "2026-11-01");
    submit(
        &mut f,
        50_001,
        currents_protocol::OperationV1::AdoptActivity(opening),
    )
    .await;
    let old = refresh_financial(&mut f, None).await;
    (f, old)
}

fn a_replacement() -> SegmentReplacementV1 {
    replacement_request(
        ACCOUNT_A,
        3000,
        (3400, "2026-10-30"),
        (-100, "2026-10-31", "2026-10-31"),
    )
}

/// Backfill movements in the large replacement, and the first observation of
/// their retained evidence. Movement `i` is observation `LARGE + 20 * i`.
const LARGE_BACKFILL: u32 = 1_198;
const LARGE: u32 = 200_000;

/// Account A anchored as in `anchored_a`, with retained evidence for a
/// replacement of 1,198 movements of -1 USD on 2026-10-31 and a 4,498 USD
/// balance at 2026-10-30.
async fn anchored_large(path: &std::path::Path) -> (support::Fixture, DomainState) {
    let opening = 3300 + i128::from(LARGE_BACKFILL);
    let evidence: Vec<_> = [(3000, opening, "2026-10-30", ObservationKind::Balance)]
        .into_iter()
        .chain(
            (0..LARGE_BACKFILL)
                .map(|i| (LARGE + 20 * i, -1, "2026-10-31", ObservationKind::Movement)),
        )
        .collect();
    anchored_with(path, &evidence).await
}

fn large_replacement() -> SegmentReplacementV1 {
    let mut request = a_replacement();
    request.opening = opening_draft(
        retained(3000, 3300 + i128::from(LARGE_BACKFILL)),
        3010,
        ACCOUNT_A,
        3017,
        "2026-10-30",
    );
    request.backfill = (0..LARGE_BACKFILL)
        .map(|i| {
            let n = LARGE + 20 * i;
            dated_draft(
                retained(n, -1),
                n + 10,
                ACCOUNT_A,
                EXPENSE,
                "2026-10-31",
                "2026-10-31",
            )
        })
        .collect();
    request
}

/// The head, domain records and frozen native entries of a replayed state.
type Replayed = (
    CommitSeq,
    Digest,
    std::collections::BTreeMap<RecordId, AuthoritativeRecordV1>,
    std::collections::BTreeMap<RecognitionId, Vec<FrozenEntryV1>>,
    std::collections::BTreeSet<RecognitionId>,
);
fn replayed(state: &DomainState) -> Replayed {
    let index = state.accounting.as_ref().unwrap();
    (
        state.through,
        state.domain_digest,
        state.records.clone(),
        index.recognitions.clone(),
        index.reversed.clone(),
    )
}

/// Replays retained history through `through` into an empty encrypted
/// destination, as recovery does, and returns the replayed state.
async fn replay_through(
    source: &currents_storage::HistoryReader,
    through: CommitSeq,
    destination: &currents_storage::EncryptedStore,
) -> DomainState {
    let empty = destination.writer().audited_prefix().await.unwrap();
    assert_eq!(empty.through, CommitSeq(0));
    let mut state = DomainState {
        accounting: Some(AccountingIndexV1::empty(empty.native_head)),
        ..DomainState::default()
    };
    let mut stream = source.stream_prefix(through).await.unwrap();
    while let Some(commit) = stream.next().await.unwrap() {
        let mut next = state.clone();
        currents_domain::ReplayEngine::apply(&mut next, &commit).unwrap();
        next.accounting = Some(
            source
                .rebuild_commit(destination, &state, &commit)
                .await
                .unwrap(),
        );
        currents_domain::recognition::RecognitionEngine::audit(&next).unwrap();
        currents_domain::associations::IdentityEngine::audit(&next).unwrap();
        state = next;
    }
    assert_eq!(state.through, through);
    assert_eq!(
        &destination.writer().audited_prefix().await.unwrap(),
        &state.accounting.as_ref().unwrap().prefix
    );
    state
}

/// The segment on `component` that no successor names.
fn active_segment(state: &DomainState, component: RecordId) -> Option<&AccountingSegmentV1> {
    let segments: Vec<_> = state
        .records
        .values()
        .filter_map(|r| match r {
            AuthoritativeRecordV1::AccountingSegment(s) if s.component == component => Some(s),
            _ => None,
        })
        .collect();
    let mut tips = segments
        .iter()
        .filter(|s| !segments.iter().any(|n| n.prior_segment == Some(s.id)));
    let tip = tips.next().copied();
    assert!(
        tips.next().is_none(),
        "one component has one active segment"
    );
    tip
}

/// The cutoff of the opening that `segment` names.
fn segment_cutoff(state: &DomainState, segment: &AccountingSegmentV1) -> TemporalValue {
    state
        .records
        .values()
        .find_map(|r| match r {
            AuthoritativeRecordV1::ActivityAdoption(a)
                if a.recognition == Some(segment.opening) =>
            {
                a.opening.as_ref().map(|o| o.cutoff.clone())
            }
            _ => None,
        })
        .unwrap()
}

/// Recognized (not reversed) native postings on `on`, with their value dates.
fn recognized_postings(state: &DomainState, on: AccountId) -> Vec<(String, Amount)> {
    let index = state.accounting.as_ref().unwrap();
    index
        .recognitions
        .iter()
        .filter(|(id, _)| !index.reversed.contains(id))
        .flat_map(|(_, entries)| entries)
        .flat_map(|entry| {
            entry
                .postings
                .iter()
                .filter(|p| p.original.account_id == on)
                .map(|p| (entry.value_date.clone(), p.original.amount))
        })
        .collect()
}

fn total(postings: impl IntoIterator<Item = (String, Amount)>) -> Amount {
    postings
        .into_iter()
        .try_fold(fixture::amount(0), |sum, (_, amount)| {
            sum.checked_add(&amount)
        })
        .unwrap()
        .normalized()
        .unwrap()
}

/// Submits through the verified host path and expects a refusal before any
/// native operation, with both heads unchanged and no settled command.
async fn refused(
    path: &std::path::Path,
    request: &SegmentReplacementV1,
    prepared: currents_storage::PreparedCommit,
) {
    let store = open(path).await;
    let before = store.writer().audited_prefix().await.unwrap();
    let control = prepared.control.clone();
    let command = prepared.result.command_id;
    let outcome =
        currents_service::synthetic_commit_verified_segment(store.writer(), request, prepared)
            .await;
    assert!(matches!(
        outcome,
        Err(currents_storage::CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(control.completed_operations(), 0);
    assert_eq!(store.writer().audited_prefix().await.unwrap(), before);
    assert!(
        store
            .writer()
            .command_result(command, DIGEST)
            .await
            .unwrap()
            .is_none()
    );
    store.close().await.unwrap();
}

#[tokio::test]
async fn cur_016_segment_commit_reopens_and_rebuilds() {
    use currents_service::{ReplayCoordinator, synthetic_commit_verified_segment};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segment-commit.db");
    let (mut f, old) = anchored_a(&path).await;
    let request = a_replacement();
    let (plan, frozen) = ready_plan(&old, &request);
    assert_eq!(frozen.operations.len(), 3);

    f.service.close().await.unwrap();
    let store = open(&path).await;
    let before = store.writer().audited_prefix().await.unwrap();
    assert_eq!(&before, &old.accounting.as_ref().unwrap().prefix);
    let receipt = synthetic_commit_verified_segment(
        store.writer(),
        &request,
        segment_commit(&plan, &frozen, 51_000, Default::default()),
    )
    .await
    .expect("a ready replacement commits through the verified writer path");
    let after = store.writer().audited_prefix().await.unwrap();
    assert_eq!(receipt.commit, CommitSeq(before.through.0 + 1));
    assert_eq!(after.through, receipt.commit);
    assert_eq!(after.native_head, receipt.native_head);
    assert_ne!(after.native_head, before.native_head);
    let committed = store.history().audit_report().await.unwrap();
    store.close().await.unwrap();

    // Close and reopen: recovery replays the replacement from retained history.
    f.service = f.reopen().await;
    let reopened = refresh_financial(&mut f, None).await;
    assert_eq!(&reopened.accounting.as_ref().unwrap().prefix, &after);
    for (id, record) in &old.records {
        assert_eq!(reopened.records.get(id), Some(record));
    }
    for record in &plan.records {
        assert!(reopened.records.values().any(|r| r == record));
    }
    let successor = active_segment(&reopened, fixture::id(ACCOUNT_A + 1)).unwrap();
    assert_eq!(successor.id, fixture::id(3017));
    assert_eq!(successor.prior_segment, Some(fixture::id(707)));
    assert_eq!(
        segment_cutoff(&reopened, successor),
        fixture::date("2026-10-30")
    );
    let index = reopened.accounting.as_ref().unwrap();
    assert!(index.reversed.contains(&RecognitionId(fixture::id(703).0)));
    assert_eq!(
        total(recognized_postings(&reopened, account(ACCOUNT_A))),
        fixture::amount(3300)
    );
    f.service.close().await.unwrap();

    // A fresh rebuild into a new encrypted file, then a replay of that file.
    let store = open(&path).await;
    assert_eq!(store.history().audit_report().await.unwrap(), committed);
    let rebuilt_path = dir.path().join("segment-rebuild.db");
    let report = ReplayCoordinator::rebuild(store.history(), open(&rebuilt_path).await)
        .await
        .expect("replay re-admits the segment replacement");
    store.close().await.unwrap();
    assert_eq!(report, committed);
    let rebuilt = open(&rebuilt_path).await;
    assert_eq!(rebuilt.writer().audited_prefix().await.unwrap(), after);
    let witness = open(&dir.path().join("segment-witness.db")).await;
    let replayed_rebuild = replay_through(&rebuilt.history(), receipt.commit, &witness).await;
    witness.close().await.unwrap();
    rebuilt.close().await.unwrap();
    assert_eq!(replayed(&replayed_rebuild), replayed(&reopened));
}

#[tokio::test]
async fn cur_016_segment_commit_rejects_stale_or_altered_plans() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segment-reject.db");
    let (mut f, old) = anchored_a(&path).await;
    let request = a_replacement();
    let (plan, frozen) = ready_plan(&old, &request);
    f.service.close().await.unwrap();

    // A balanced frozen batch that differs from the one derived again.
    let mut wrong = plan.accounting.clone();
    let AccountingIntent::Correction(correction) = &mut wrong else {
        panic!("a replacement retires the old opening");
    };
    let postings = &mut correction.replacement[1].postings;
    postings[0].amount.coefficient -= 1;
    postings[1].amount.coefficient += 1;
    let wrong_frozen = currents_accounting::AccountingAdapter::freeze(&wrong, &old)
        .expect("balanced wrong economics pass native shape checks");
    assert_eq!(wrong_frozen.preconditions, frozen.preconditions);
    assert_ne!(wrong_frozen, frozen);
    refused(
        &path,
        &request,
        segment_commit(&plan, &wrong_frozen, 51_010, Default::default()),
    )
    .await;

    // Plans whose records changed after preview, with the original batch.
    let mut changed_amount = plan.clone();
    let mut changed_segment = plan.clone();
    for record in &mut changed_amount.records {
        if let AuthoritativeRecordV1::Recognition(r) = record
            && r.id == RecognitionId(fixture::id(3113).0)
        {
            r.postings[0].amount.coefficient -= 1;
            r.postings[1].amount.coefficient += 1;
        }
    }
    for record in &mut changed_segment.records {
        if let AuthoritativeRecordV1::AccountingSegment(s) = record {
            s.approved_accounts.pop();
        }
    }
    for (n, changed) in [(51_020, changed_amount), (51_030, changed_segment)] {
        assert_ne!(changed, plan);
        refused(
            &path,
            &request,
            segment_commit(&changed, &frozen, n, Default::default()),
        )
        .await;
    }

    // A plan prepared at an older head. An unrelated import advances the
    // head; the same replacement is still ready there.
    f.service = f.reopen().await;
    refresh_financial(&mut f, None).await;
    publish_legacy(&mut f, 220, "1", "2026-11-03").await;
    let newer = refresh_financial(&mut f, None).await;
    assert_eq!(newer.through, CommitSeq(old.through.0 + 1));
    ready_plan(&newer, &request);
    f.service.close().await.unwrap();
    refused(
        &path,
        &request,
        segment_commit(&plan, &frozen, 51_040, Default::default()),
    )
    .await;
}

#[tokio::test]
async fn cur_016_segment_cancel_and_retry() {
    use currents_accounting::test_fixtures::{ClosureFault, fault_control};
    use currents_service::synthetic_commit_verified_segment;
    use currents_storage::CommitFailure;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segment-retry.db");
    let (f, old) = anchored_a(&path).await;
    let request = a_replacement();
    let (plan, frozen) = ready_plan(&old, &request);
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let writer = store.writer();
    let before = writer.audited_prefix().await.unwrap();

    // Cancel after native entries start and before COMMIT submission.
    let prepared = segment_commit(
        &plan,
        &frozen,
        52_000,
        fault_control(ClosureFault::PauseBeforeCommit),
    );
    let control = prepared.control.clone();
    let cancel = async {
        while control.completed_operations() == 0 {
            tokio::task::yield_now().await;
        }
        assert!(control.request_cancel());
    };
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(
            synthetic_commit_verified_segment(writer, &request, prepared),
            cancel
        )
    })
    .await
    .expect("synthetic cancellation checkpoint not reached");
    assert!(matches!(
        outcome,
        Err(CommitFailure::DefinitelyRolledBack(EngineError {
            class: ErrorClass::Canceled,
            ..
        }))
    ));
    assert_eq!(writer.audited_prefix().await.unwrap(), before);
    store.close().await.unwrap();
    let store = open(&path).await;
    let writer = store.writer();
    assert_eq!(writer.audited_prefix().await.unwrap(), before);
    let command = CommandId(fixture::id(52_000).0);
    assert!(
        writer
            .command_result(command, DIGEST)
            .await
            .unwrap()
            .is_none()
    );

    // The identical command commits once; a second submission resolves to it.
    let identical = || segment_commit(&plan, &frozen, 52_000, Default::default());
    let first = synthetic_commit_verified_segment(writer, &request, identical())
        .await
        .expect("the identical command commits after cancellation");
    let retry = synthetic_commit_verified_segment(writer, &request, identical())
        .await
        .expect("an identical retry resolves the settled commit");
    assert_eq!(first.commit, CommitSeq(before.through.0 + 1));
    assert_eq!(
        (retry.commit, retry.domain_digest, &retry.native_head),
        (first.commit, first.domain_digest, &first.native_head)
    );
    let committed = writer.audited_prefix().await.unwrap();
    assert_eq!(committed.through, first.commit);

    // The same command ID with different content: other draft identities over
    // the same evidence form another ready plan at the same prior.
    let mut other = a_replacement();
    other.opening = opening_draft(retained(3000, 3400), 3510, ACCOUNT_A, 3517, "2026-10-30");
    other.backfill[0] = dated_draft(
        retained(3100, -100),
        3610,
        ACCOUNT_A,
        EXPENSE,
        "2026-10-31",
        "2026-10-31",
    );
    let (other_plan, other_frozen) = ready_plan(&old, &other);
    assert_ne!(other_plan, plan);
    let reused = || segment_commit(&other_plan, &other_frozen, 52_000, Default::default());
    assert!(matches!(
        synthetic_commit_verified_segment(writer, &other, reused()).await,
        Err(CommitFailure::DefinitelyRolledBack(_))
    ));
    assert!(matches!(
        writer.commit(reused()).await,
        Err(CommitFailure::DefinitelyRolledBack(_))
    ));
    let mut other_digest = identical();
    other_digest.result.digest = Digest([78; 32]);
    assert!(matches!(
        synthetic_commit_verified_segment(writer, &request, other_digest).await,
        Err(CommitFailure::DefinitelyRolledBack(_))
    ));
    assert_eq!(writer.audited_prefix().await.unwrap(), committed);
    store.close().await.unwrap();
}

#[tokio::test]
async fn cur_016_4_1200_operation_replacement_is_atomic() {
    use currents_accounting::test_fixtures::{ClosureFault, fault_control, release_control};
    use currents_service::{ReplayCoordinator, synthetic_commit_verified_segment};
    use currents_storage::CommitFailure;
    // Each full read of this history costs seconds, so the test reads it
    // only where a check needs it.
    let started = std::time::Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("segment-large.db");
    let (f, old) = anchored_large(&path).await;
    let request = large_replacement();
    let (plan, frozen) = ready_plan(&old, &request);
    // The old opening's reversal, the new opening and 1,198 backfills.
    assert_eq!(frozen.operations.len(), 1200);
    f.service.close().await.unwrap();
    let identical = |control| segment_commit(&plan, &frozen, 53_000, control);
    let store = open(&path).await;
    let (writer, history) = (store.writer(), store.history());
    let prior = history.audit_report().await.unwrap();

    // Cancellation after the final native batch and before COMMIT.
    let prepared = identical(fault_control(ClosureFault::PauseAfter1200));
    let control = prepared.control.clone();
    let cancel = async {
        while control.completed_operations() < 1200 {
            tokio::task::yield_now().await;
        }
        assert!(control.request_cancel());
    };
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        tokio::join!(
            synthetic_commit_verified_segment(writer, &request, prepared),
            cancel
        )
    })
    .await
    .expect("the final batch was not reached");
    assert!(matches!(
        outcome,
        Err(CommitFailure::DefinitelyRolledBack(EngineError {
            class: ErrorClass::Canceled,
            ..
        }))
    ));
    let command = CommandId(fixture::id(53_000).0);
    assert!(
        writer
            .command_result(command, DIGEST)
            .await
            .unwrap()
            .is_none()
    );

    // The identical command commits once. After its first native batch of
    // 600, a separate reader sees the prior domain and native heads: the
    // cancellation left them, and the open commit does not show.
    let prepared = identical(fault_control(ClosureFault::PauseAfter600));
    let control = prepared.control.clone();
    let observe = async {
        while control.completed_operations() < 600 {
            tokio::task::yield_now().await;
        }
        assert_eq!(control.completed_operations(), 600);
        assert_eq!(history.audit_report().await.unwrap(), prior);
        release_control(&control);
    };
    let (receipt, ()) = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        tokio::join!(
            synthetic_commit_verified_segment(writer, &request, prepared),
            observe
        )
    })
    .await
    .expect("the first native batch did not finish");
    let receipt = receipt.expect("the identical command commits after the cancellation");
    assert_eq!(control.completed_operations(), 1200);
    assert_eq!(receipt.commit, CommitSeq(prior.through.0 + 1));
    let committed = history.audit_report().await.unwrap();
    assert_eq!(
        (committed.through, &committed.native_head),
        (receipt.commit, &receipt.native_head)
    );
    assert_ne!(committed.native_head, prior.native_head);
    store.close().await.unwrap();

    // A fresh rebuild matches the committed native head, native leaves and
    // ordered postings.
    let store = open(&path).await;
    let rebuilt = ReplayCoordinator::rebuild(
        store.history(),
        open(&dir.path().join("segment-large-rebuild.db")).await,
    )
    .await
    .expect("replay re-admits the large replacement");
    store.close().await.unwrap();
    assert_eq!(rebuilt, committed);
    eprintln!(
        "cur_016_4_1200_operation_replacement_is_atomic ran in {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn cur_016_3_2024_backfill_after_2026_declaration() {
    use currents_protocol::{OperationV1, TrustedClock};
    use currents_service::synthetic_commit_verified_segment;
    let journey: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/journeys/s4/date-floor.json"
    ))
    .unwrap();
    let text = |pointer: &str| journey.pointer(pointer).unwrap().as_str().unwrap();
    let value = |pointer: &str| i128::from(journey.pointer(pointer).unwrap().as_i64().unwrap());
    assert_eq!(text("/account"), "B");
    let b = account(ACCOUNT_B);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("date-floor.db");
    let evidence = [
        RetainedEvidence {
            n: 2900,
            value: value("/opening/amount"),
            day: text("/opening/cutoff"),
            kind: ObservationKind::Balance,
            text: "Synthetic account B balance at the 2026 opening",
        },
        RetainedEvidence {
            n: 3000,
            value: value("/replacement/amount"),
            day: text("/replacement/cutoff"),
            kind: ObservationKind::Balance,
            text: "Synthetic account B balance at the 2024 opening",
        },
        RetainedEvidence {
            n: 3100,
            value: value("/backfill/amount"),
            day: text("/backfill/effective"),
            kind: ObservationKind::Movement,
            text: "Synthetic account B movement in 2024",
        },
    ];
    let (mut f, _) = bootstrap_with_evidence(&path, &evidence).await;
    let declared_at: RecordedInstant =
        serde_json::from_value(journey["declared_at"].clone()).unwrap();
    f.clock
        .advance(declared_at.0.unix_timestamp() - f.clock.now().unix_timestamp());
    declare_template(&mut f, &fixture::monetary_state()).await;
    let mut declaration = fixture::catalog();
    declaration.revision_id = fixture::id(ACCOUNT_B + 2);
    declaration.catalog.account_id = b;
    declaration.catalog.component_id = fixture::id(ACCOUNT_B + 1);
    declaration.catalog.display_name = "Synthetic account B".into();
    declaration.ownership.account_id = b;
    submit(
        &mut f,
        11_100,
        OperationV1::SetAccountCatalogComplete(declaration),
    )
    .await;
    refresh_financial(&mut f, None).await;
    let opening = opening_draft(
        retained(2900, value("/opening/amount")),
        700,
        ACCOUNT_B,
        707,
        text("/opening/cutoff"),
    );
    submit(&mut f, 53_000, OperationV1::AdoptActivity(opening)).await;
    let old = refresh_financial(&mut f, None).await;
    assert!(
        old.records.values().any(|r| matches!(r,
            AuthoritativeRecordV1::Decision(d)
                if d.adopted_result.contains(&fixture::id(ACCOUNT_B + 2))
                    && d.recorded_time == declared_at)),
        "account B is declared at the journey's 2026 instant"
    );

    let request = replacement_request(
        ACCOUNT_B,
        3000,
        (value("/replacement/amount"), text("/replacement/cutoff")),
        (
            value("/backfill/amount"),
            text("/backfill/effective"),
            text("/backfill/booking"),
        ),
    );
    let (plan, frozen) = ready_plan(&old, &request);
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let receipt = synthetic_commit_verified_segment(
        store.writer(),
        &request,
        segment_commit(&plan, &frozen, 53_010, Default::default()),
    )
    .await
    .expect("the native date floor admits 2024 entries after a 2026 declaration");
    assert_eq!(receipt.commit, CommitSeq(old.through.0 + 1));
    let source = store.history();
    let component = fixture::id(ACCOUNT_B + 1);

    // History through the commit before the replacement keeps the old belief.
    let earlier = open(&dir.path().join("before-replacement.db")).await;
    let before = replay_through(&source, old.through, &earlier).await;
    earlier.close().await.unwrap();
    assert_eq!(before.records, old.records);
    let segment = active_segment(&before, component).unwrap();
    assert_eq!(segment.id, fixture::id(707));
    assert_eq!(
        segment_cutoff(&before, segment),
        fixture::date(text("/opening/cutoff"))
    );
    assert!(
        recognized_postings(&before, b)
            .iter()
            .all(|(day, _)| !day.starts_with("2024")),
        "B has no recognized posting dated in 2024 before the replacement"
    );

    // History through the replacement shows the corrected belief.
    let later = open(&dir.path().join("through-replacement.db")).await;
    let after = replay_through(&source, receipt.commit, &later).await;
    later.close().await.unwrap();
    store.close().await.unwrap();
    let segment = active_segment(&after, component).unwrap();
    assert_eq!(segment.id, fixture::id(3017));
    assert_eq!(
        segment_cutoff(&after, segment),
        fixture::date(text("/replacement/cutoff"))
    );
    let as_of = text("/expected/as_of");
    assert_eq!(
        total(
            recognized_postings(&after, b)
                .into_iter()
                .filter(|(day, _)| day.as_str() <= as_of)
        ),
        fixture::amount(value("/expected/balance"))
    );
    let index = after.accounting.as_ref().unwrap();
    let [backfill] = index.recognitions[&RecognitionId(fixture::id(3113).0)].as_slice() else {
        panic!("one native backfill entry");
    };
    assert_eq!(backfill.value_date, text("/backfill/effective"));
    assert_eq!(backfill.booking_date, text("/backfill/booking"));
}

const COUNTER: u32 = 60;
const CLEARING: u32 = 80;

/// Account A's replacement from `a_replacement`'s evidence, except that the
/// 2026-10-31 send of 1,000 USD to clearing joins the transfer family in
/// which counterpart C's receipt (recognition 1610) is already recognized.
fn cross_account_request(state: &DomainState) -> SegmentReplacementV1 {
    let mut request = replacement_request(
        ACCOUNT_A,
        3000,
        (4300, "2026-10-30"),
        (-1000, "2026-10-31", "2026-10-31"),
    );
    request.backfill[0].financial.as_mut().unwrap().postings[1].account_id = account(CLEARING);
    request.approved_accounts = vec![
        account(ACCOUNT_A),
        account(EQUITY),
        account(CLEARING),
        account(COUNTER),
    ];
    request.transfers = vec![TransferJoinV1 {
        revision: fixture::id(3500),
        prior_revision: fixture::id(1650),
        leg: LegId(fixture::id(3111).0),
        source: transfers::row_fact(fixture::id(1501), "send-a"),
    }];
    request.reemissions = vec![transfers::reemission(state, 1610, 3600)];
    request
}

#[tokio::test]
async fn cur_016_1_encrypted_cross_account_commit() {
    use currents_protocol::OperationV1;
    use currents_service::{ReplayCoordinator, synthetic_commit_verified_segment};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cross-account.db");
    let evidence = [
        RetainedEvidence {
            n: 2900,
            value: 3300,
            day: "2026-11-01",
            kind: ObservationKind::Balance,
            text: "Synthetic account A balance at the old opening",
        },
        RetainedEvidence {
            n: 3000,
            value: 4300,
            day: "2026-10-30",
            kind: ObservationKind::Balance,
            text: "Synthetic account A balance at the new opening",
        },
        RetainedEvidence {
            n: 3100,
            value: -1000,
            day: "2026-10-31",
            kind: ObservationKind::Movement,
            text: "Synthetic account A send to account C",
        },
    ];
    let (mut f, _) = bootstrap_with_evidence(&path, &evidence).await;
    let (template, _) = association_fixtures::transfer_request();
    declare_template(&mut f, &template).await;
    let opening = opening_draft(retained(2900, 3300), 700, ACCOUNT_A, 707, "2026-11-01");
    submit(&mut f, 54_000, OperationV1::AdoptActivity(opening)).await;
    refresh_financial(&mut f, None).await;
    let rows = transfers::cross_account_rows();
    publish_relationship_rows(&mut f, 1500, rows.clone()).await;
    let receipt = transfers::clearing_family(fixture::id(1501), &rows, 1600, &[(COUNTER, 1)]);
    submit(&mut f, 54_001, OperationV1::AssociateLegsComplete(receipt)).await;
    let old = refresh_financial(&mut f, None).await;

    let request = cross_account_request(&old);
    let (plan, frozen) = ready_plan(&old, &request);
    assert_eq!(frozen.operations.len(), 5);
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let before = store.writer().audited_prefix().await.unwrap();
    let receipt = synthetic_commit_verified_segment(
        store.writer(),
        &request,
        segment_commit(&plan, &frozen, 54_010, Default::default()),
    )
    .await
    .expect("a cross-account replacement commits through the verified writer path");
    let after = store.writer().audited_prefix().await.unwrap();
    assert_eq!(receipt.commit, CommitSeq(before.through.0 + 1));
    assert_eq!(after.native_head, receipt.native_head);
    let committed = store.history().audit_report().await.unwrap();
    store.close().await.unwrap();

    // Reopen: recovery replays the replacement and both audits pass.
    f.service = f.reopen().await;
    let reopened = refresh_financial(&mut f, None).await;
    assert_eq!(&reopened.accounting.as_ref().unwrap().prefix, &after);
    for (id, record) in &old.records {
        assert_eq!(reopened.records.get(id), Some(record));
    }
    currents_domain::associations::IdentityEngine::audit(&reopened).unwrap();
    let by_date = |state: &DomainState, n: u32| {
        let mut postings = recognized_postings(state, account(n));
        postings.sort_by(|a, b| a.0.cmp(&b.0));
        postings
    };
    assert_eq!(by_date(&reopened, COUNTER), by_date(&old, COUNTER));
    assert_eq!(
        total(recognized_postings(&reopened, account(ACCOUNT_A))),
        fixture::amount(3300)
    );
    assert_eq!(
        total(recognized_postings(&reopened, account(CLEARING))),
        fixture::amount(0)
    );
    f.service.close().await.unwrap();

    // A fresh rebuild into a new encrypted file reaches the same native head.
    let store = open(&path).await;
    let rebuilt_path = dir.path().join("cross-account-rebuild.db");
    let report = ReplayCoordinator::rebuild(store.history(), open(&rebuilt_path).await)
        .await
        .expect("replay re-admits the cross-account replacement");
    store.close().await.unwrap();
    assert_eq!(report, committed);
    let rebuilt = open(&rebuilt_path).await;
    assert_eq!(rebuilt.writer().audited_prefix().await.unwrap(), after);
    rebuilt.close().await.unwrap();
}

// Claims on account A: K1 covers 2026-10-01 to 2026-10-31 with status
// Proposed, K2 covers 2026-11-05 to 2026-11-30. Both have empty explicit
// dependency lists.
const K1: u32 = 4001;
const K2: u32 = 4002;

fn claim_on_a(n: u32, start: &str, end: &str) -> AuthoritativeRecordV1 {
    let fixtures: Vec<AuthoritativeRecordV1> =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    let Some(AuthoritativeRecordV1::ReconciliationClaim(mut claim)) = fixtures
        .into_iter()
        .find(|r| matches!(r, AuthoritativeRecordV1::ReconciliationClaim(_)))
    else {
        panic!("claim fixture missing");
    };
    let civil = |day: &str| {
        let TemporalValue::Date { civil, .. } = fixture::date(day) else {
            panic!("civil date")
        };
        Some(civil)
    };
    claim.id = fixture::id(n);
    claim.scope = vec![account(ACCOUNT_A)];
    claim.cutoffs = DateScope {
        start: civil(start),
        end: civil(end),
        basis: DateBasis::Effective,
    };
    claim.status = ReconciliationStatus::Proposed;
    claim.basis.dataset_id = DatasetId(support::id(1000).0);
    AuthoritativeRecordV1::ReconciliationClaim(claim)
}

/// A history where K1 and K2 commit after A's anchor, then the replacement
/// `a_replacement` commits through the verified writer path.
struct Invalidated {
    path: std::path::PathBuf,
    /// The state at the claims' commit.
    claimed: DomainState,
    replaced: CommitSeq,
}
async fn invalidated_a(dir: &std::path::Path) -> Invalidated {
    use currents_service::synthetic_commit_verified_segment;
    let path = dir.join("claims.db");
    let (mut f, old) = anchored_a(&path).await;
    f.service.close().await.unwrap();

    // The claims commit with no accounting effect, as their owner will.
    let plan = DecisionPlan {
        records: vec![
            claim_on_a(K1, "2026-10-01", "2026-10-31"),
            claim_on_a(K2, "2026-11-05", "2026-11-30"),
        ],
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "Synthetic reconciliation claims".into(),
        }),
    };
    let frozen = currents_accounting::AccountingAdapter::freeze(&plan.accounting, &old).unwrap();
    let store = open(&path).await;
    store
        .writer()
        .commit(segment_commit(&plan, &frozen, 52_000, Default::default()))
        .await
        .unwrap();
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let claimed = refresh_financial(&mut f, None).await;
    assert_eq!(claimed.through, CommitSeq(old.through.0 + 1));
    f.service.close().await.unwrap();

    let request = a_replacement();
    let (plan, frozen) = ready_plan(&claimed, &request);
    let store = open(&path).await;
    let receipt = synthetic_commit_verified_segment(
        store.writer(),
        &request,
        segment_commit(&plan, &frozen, 52_010, Default::default()),
    )
    .await
    .expect("a replacement with claims in its scope commits");
    store.close().await.unwrap();
    Invalidated {
        path,
        claimed,
        replaced: receipt.commit,
    }
}

#[tokio::test]
async fn cur_016_backfill_invalidates_claim_in_scope() {
    let dir = tempfile::tempdir().unwrap();
    let history = invalidated_a(dir.path()).await;
    let store = open(&history.path).await;
    let commit = store
        .history()
        .read_prefix(history.replaced)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let invalidations = &commit.command_result.effects.invalidations;
    assert!(invalidations.contains(&fixture::id(K1)));
    assert!(!invalidations.contains(&fixture::id(K2)));

    let witness = open(&dir.path().join("through-claims.db")).await;
    let at_claims = replay_through(&store.history(), history.claimed.through, &witness).await;
    witness.close().await.unwrap();
    let witness = open(&dir.path().join("through-replacement.db")).await;
    let replaced = replay_through(&store.history(), history.replaced, &witness).await;
    witness.close().await.unwrap();
    store.close().await.unwrap();

    assert!(!at_claims.claim_invalidated(fixture::id(K1)));
    assert!(replaced.claim_invalidated(fixture::id(K1)));
    assert!(!replaced.claim_invalidated(fixture::id(K2)));
    for kept in [K1, K2] {
        let key = fixture::id(kept);
        assert_eq!(
            serde_json::to_vec(&replaced.records[&key]).unwrap(),
            serde_json::to_vec(&history.claimed.records[&key]).unwrap()
        );
    }
}

#[tokio::test]
async fn cur_016_replay_checks_invalidations() {
    use currents_service::ReplayCoordinator;
    let dir = tempfile::tempdir().unwrap();
    let history = invalidated_a(dir.path()).await;

    // A fresh rebuild reports the same invalidated claims.
    let store = open(&history.path).await;
    let witness = open(&dir.path().join("source-witness.db")).await;
    let source = replay_through(&store.history(), history.replaced, &witness).await;
    witness.close().await.unwrap();
    let rebuilt_path = dir.path().join("rebuilt.db");
    ReplayCoordinator::rebuild(store.history(), open(&rebuilt_path).await)
        .await
        .expect("replay re-admits the invalidations");
    let rebuilt = open(&rebuilt_path).await;
    let witness = open(&dir.path().join("rebuilt-witness.db")).await;
    let again = replay_through(&rebuilt.history(), history.replaced, &witness).await;
    witness.close().await.unwrap();
    rebuilt.close().await.unwrap();
    assert!(source.invalidated.contains(&fixture::id(K1)));
    assert_eq!(again.invalidated, source.invalidated);
    for claim in [K1, K2] {
        let key = fixture::id(claim);
        assert_eq!(again.claim_status(key), source.claim_status(key));
        assert_eq!(again.claim_invalidated(key), source.claim_invalidated(key));
    }

    // Replay rejects a commit whose recorded invalidations add or omit a claim.
    let stored = store
        .history()
        .read_prefix(history.replaced)
        .await
        .unwrap()
        .pop()
        .unwrap();
    store.close().await.unwrap();
    let mut added = stored.clone();
    added
        .command_result
        .effects
        .invalidations
        .push(fixture::id(K2));
    let mut omitted = stored.clone();
    omitted
        .command_result
        .effects
        .invalidations
        .retain(|id| *id != fixture::id(K1));
    let mut exact = history.claimed.clone();
    currents_domain::ReplayEngine::apply(&mut exact, &stored).unwrap();
    for tampered in [added, omitted] {
        let mut prior = history.claimed.clone();
        assert!(currents_domain::ReplayEngine::apply(&mut prior, &tampered).is_err());
        assert_eq!(prior.through, history.claimed.through);
        assert_eq!(prior.invalidated, history.claimed.invalidated);
    }
}

const UNRESOLVED: u32 = 40;
const WAGES: u32 = 45;

#[tokio::test]
async fn cur_016_reinterpretation_encrypted_commit() {
    use currents_domain::correction::{CorrectionEngine, CorrectionRequestV1};
    use currents_protocol::OperationV1;
    use currents_service::{ReplayCoordinator, synthetic_commit_verified_correction};
    let journey: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/journeys/s5/reinterpretation.json"
    ))
    .unwrap();
    assert_eq!(journey["account"], "A");
    let day = journey["deposit"]["date"].as_str().unwrap();
    let deposited = i128::from(journey["deposit"]["amount"].as_i64().unwrap());
    let (a, unresolved, wages) = (account(ACCOUNT_A), account(UNRESOLVED), account(WAGES));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reinterpretation.db");
    let evidence = [RetainedEvidence {
        n: 3100,
        value: deposited,
        day,
        kind: ObservationKind::Movement,
        text: "Synthetic unexplained deposit to account A",
    }];
    let (mut f, _) = bootstrap_with_evidence(&path, &evidence).await;
    declare_template(&mut f, &fixture::monetary_state()).await;
    let mut income = fixture::catalog();
    income.revision_id = fixture::id(WAGES + 2);
    income.catalog.component_id = fixture::id(WAGES + 1);
    income.catalog.account_id = wages;
    income.catalog.account_class = AccountClass::Income;
    income.catalog.normal_sign = NormalSign::Credit;
    income.catalog.display_name = "Synthetic wages".into();
    income.ownership.account_id = wages;
    income.origin = AccountOriginV1::Internal(InternalAccountV1 {
        role: ControlRoleV1::IncomeFlow,
        created_at: fixture::date("2026-01-01"),
    });
    submit(
        &mut f,
        11_200,
        OperationV1::SetAccountCatalogComplete(income),
    )
    .await;
    refresh_financial(&mut f, None).await;
    let mut deposit = dated_draft(
        retained(3100, deposited),
        3110,
        ACCOUNT_A,
        UNRESOLVED,
        day,
        day,
    );
    deposit.financial.as_mut().unwrap().unresolved_account = Some(unresolved);
    let original = deposit.financial.as_ref().unwrap().recognition_id;
    let adoption = deposit.adoption_id;
    let governing = deposit.governing_policy.clone();
    submit(&mut f, 53_200, OperationV1::AdoptActivity(deposit)).await;
    let old = refresh_financial(&mut f, None).await;

    let Some(AuthoritativeRecordV1::Recognition(prior)) = old.records.get(&RecordId(original.0))
    else {
        panic!("the deposit's recognition is retained");
    };
    let mut replacement = prior.clone();
    replacement.id = RecognitionId(fixture::id(9300).0);
    for (posting, n) in replacement.postings.iter_mut().zip(9301..) {
        posting.id = fixture::id(n);
    }
    replacement.postings[1].account_id = wages;
    let request = CorrectionRequestV1 {
        active_recognition: original,
        replacement: vec![replacement.clone()],
        remapping: vec![],
        reason: "The unexplained deposit is wages".into(),
    };
    let revision = fixture::id(9310);
    let plan = CorrectionEngine::reinterpret(&old, revision, &request, &governing, None).unwrap();
    let frozen = currents_accounting::AccountingAdapter::freeze(&plan.accounting, &old).unwrap();
    f.service.close().await.unwrap();
    let store = open(&path).await;
    let receipt = synthetic_commit_verified_correction(
        store.writer(),
        segment_commit(&plan, &frozen, 53_210, Default::default()),
    )
    .await
    .expect("the encrypted writer commits the reinterpretation");
    assert_eq!(receipt.commit, CommitSeq(old.through.0 + 1));
    let source = store.history();

    // History through the commit before the correction keeps the old belief.
    let earlier = open(&dir.path().join("before-reinterpretation.db")).await;
    let before = replay_through(&source, old.through, &earlier).await;
    earlier.close().await.unwrap();
    assert!(
        !before
            .accounting
            .as_ref()
            .unwrap()
            .reversed
            .contains(&original)
    );
    assert_eq!(
        total(recognized_postings(&before, unresolved)),
        fixture::amount(-deposited)
    );
    assert_eq!(
        total(recognized_postings(&before, wages)),
        fixture::amount(0)
    );

    // History through the correction shows the new interpretation; A's
    // postings keep their amounts and dates.
    let later = open(&dir.path().join("through-reinterpretation.db")).await;
    let after = replay_through(&source, receipt.commit, &later).await;
    later.close().await.unwrap();
    assert!(
        after
            .accounting
            .as_ref()
            .unwrap()
            .reversed
            .contains(&original)
    );
    let sorted = |state: &DomainState| {
        let mut postings = recognized_postings(state, a);
        postings.sort_by(|x, y| x.0.cmp(&y.0));
        postings
    };
    assert_eq!(sorted(&after), sorted(&before));
    assert_eq!(
        total(recognized_postings(&after, unresolved)),
        fixture::amount(0)
    );
    assert_eq!(
        total(recognized_postings(&after, wages)),
        fixture::amount(-deposited)
    );

    // A fresh rebuild and a reopened store reach the same native head.
    let rebuilt = open(&dir.path().join("rebuild.db")).await;
    let report = ReplayCoordinator::rebuild(source, rebuilt).await.unwrap();
    assert_eq!(report.through, receipt.commit);
    assert_eq!(report.native_head, receipt.native_head);
    store.close().await.unwrap();
    f.service = f.reopen().await;
    let current = refresh_financial(&mut f, None).await;
    let index = current.accounting.as_ref().unwrap();
    assert_eq!(index.prefix.native_head, receipt.native_head);
    for key in [adoption, RecordId(original.0)] {
        assert_eq!(current.records.get(&key), old.records.get(&key));
    }
    assert!(matches!(
        current.records.get(&revision),
        Some(AuthoritativeRecordV1::RecognitionCorrectionRevision(r))
            if r.schema_version == 2
                && r.remap_disposition == RecognitionCorrectionRemapV1::ReinterpretedStableLegs(
                    StableLegsReinterpretationV1 {
                        governing_policy: governing.clone(),
                        unresolved_account: None,
                        changed_accounts: vec![unresolved, wages],
                    }
                )
    ));
}
