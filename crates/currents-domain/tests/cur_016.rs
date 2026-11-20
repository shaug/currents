//! CUR-016 preview uses exact C08-admitted records and the recomputed C05 index.
use currents_accounting::AccountingAdapter;
use currents_domain::{ReplayEngine, associations::IdentityEngine, recognition::RecognitionEngine};
use currents_types::*;
use sha2::{Digest as _, Sha256};

#[path = "support/associations.rs"]
#[allow(dead_code)]
mod associations;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;

use fixture::id;

/// Admit original activity through C08 and reconstruct the exact C05 sidecar
/// from the same canonical commit. This fixture does not model native I/O.
fn retain_frozen_plan(state: &mut DomainState, plan: DecisionPlan) {
    replay_frozen_plan(state, plan).expect("accepted original fixture must replay");
}

fn replay_frozen_plan(
    state: &mut DomainState,
    plan: DecisionPlan,
) -> Result<DomainCommitV1, EngineError> {
    let batch = AccountingAdapter::freeze(&plan.accounting, state).unwrap();
    replay_with_frozen_batch(state, plan, &batch)
}

fn replay_with_frozen_batch(
    state: &mut DomainState,
    plan: DecisionPlan,
    batch: &AccountingBatchV1,
) -> Result<DomainCommitV1, EngineError> {
    let seq = CommitSeq(state.through.0 + 1);
    let native_head_after = if batch.operations.is_empty() {
        state
            .accounting
            .as_ref()
            .unwrap()
            .prefix
            .native_head
            .clone()
    } else {
        NativeHead {
            semantics_version: 1,
            checkpoint: Digest([u8::try_from(seq.0).unwrap(); 32]),
        }
    };
    let mut records = plan.records;
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(
        AccountingAdapter::manifest(id(30_000 + u32::try_from(seq.0).unwrap()), batch).unwrap(),
    ));
    let mut commit = DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records,
        decisions: vec![],
        native_head_after,
        command_result: CommandResultV1 {
            command_id: CommandId(id(31_000 + u32::try_from(seq.0).unwrap()).0),
            digest: Digest([7; 32]),
            state: CommandState::Committed,
            commit: Some(seq),
            effects: EffectsV1 {
                records: vec![],
                accounts: vec![],
                amounts: vec![],
                invalidations: plan.invalidations,
            },
            health: vec![],
        },
        digest: Digest::default(),
    };
    commit.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&commit).unwrap()).into());
    let mut with_native = state.clone();
    AccountingAdapter::advance_index(
        &mut with_native,
        &commit,
        &AccountingAdapter::instruction_bytes(batch).unwrap(),
    )?;
    AccountingAdapter::verify_correspondence(
        &commit,
        Some(batch),
        with_native.accounting.as_ref().unwrap(),
    )?;
    ReplayEngine::apply(state, &commit)?;
    state.accounting = with_native.accounting;
    assert_eq!(
        state.domain_digest,
        state.accounting.as_ref().unwrap().prefix.domain_digest
    );
    Ok(commit)
}

fn retain_original_opening(
    state: &mut DomainState,
    include_legacy_movement: bool,
) -> AccountingSegmentV1 {
    let old_prefix = state.accounting.take().unwrap().prefix;
    let mut native = AccountingIndexV1::empty(old_prefix.native_head.clone());
    native.prefix = old_prefix;
    state.accounting = Some(native);
    let mut accounts = Vec::new();
    let mut units = Vec::new();
    for record in state.records.values() {
        match record {
            AuthoritativeRecordV1::AccountComponent(account) => accounts.push(account.clone()),
            AuthoritativeRecordV1::UnitCatalog(catalog) => units.extend(catalog.units.clone()),
            _ => {}
        }
    }
    let date_policy = state.records.remove(&id(103)).unwrap();
    retain_frozen_plan(
        state,
        DecisionPlan {
            records: vec![date_policy],
            invalidations: vec![],
            accounting: AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
                accounts,
                units,
            }),
        },
    );
    let mut opening = fixture::supported_opening(state);
    if !include_legacy_movement {
        opening
            .financial
            .as_mut()
            .unwrap()
            .opening
            .as_mut()
            .unwrap()
            .included_activity
            .clear();
    }
    let old_segment_id = opening
        .financial
        .as_ref()
        .unwrap()
        .opening
        .as_ref()
        .unwrap()
        .segment_id;
    let plan = RecognitionEngine::adopt(state, &opening).unwrap();
    retain_frozen_plan(state, plan);
    let verified = state.records.get(&old_segment_id).unwrap().clone();
    let AuthoritativeRecordV1::AccountingSegment(old_segment) = verified else {
        panic!("prior segment missing");
    };
    old_segment
}

fn original_segment(
    include_legacy_movement: bool,
) -> (DomainState, AccountingSegmentV1, Vec<RecognitionId>) {
    let (mut state, mut transfer) = associations::transfer_request();
    let old_segment = retain_original_opening(&mut state, include_legacy_movement);
    associations::association_date(&mut transfer, "2026-11-03");
    associations::retained_relationships(
        &mut state,
        1500,
        associations::boundary_rows("2026-11-03", false),
    );
    let plan = IdentityEngine::associate(&state, &transfer).unwrap();
    retain_frozen_plan(&mut state, plan);
    (state, old_segment, transfer.revision.recognitions)
}

#[test]
fn cur_016_original_segment_is_backed_by_frozen_c05_authority() {
    let (state, segment, transfer) = original_segment(false);
    let index = state.accounting.as_ref().unwrap();
    assert!(!transfer.is_empty());
    for original in std::iter::once(&segment.opening).chain(&transfer) {
        assert!(
            index
                .recognitions
                .get(original)
                .is_some_and(|ops| !ops.is_empty())
        );
        assert!(!index.reversed.contains(original));
    }
}

/// Generate synthetic immutable evidence off-prefix, then retain it without
/// admitting any of the proposed opening/backfill activity records.
fn retain_proposal_evidence(
    state: &mut DomainState,
    n: u32,
    amount: Amount,
    day: &str,
    kind: ObservationKind,
) -> SourceFacetBindingV1 {
    let mut scratch = DomainState::default();
    let mut source = fixture::evidence(&mut scratch, n, 1, None);
    source.amount = amount;
    let mut observation = scratch.records.remove(&source.observation).unwrap();
    let AuthoritativeRecordV1::ObservationVersion(record) = &mut observation else {
        panic!();
    };
    record.semantic_kind = kind;
    record.temporal = fixture::date(day);
    record.original_ids = vec![format!("segment-preview-{n}")];
    let mut product = scratch.records.remove(&source.parse_product).unwrap();
    let AuthoritativeRecordV1::AddressedParseProduct(record) = &mut product else {
        panic!();
    };
    record.product.facets[0].kind = kind;
    record.product.facets[0].amount = SourceValue::Known(amount);
    record.product.facets[0].temporal = fixture::date(day);
    retain_frozen_plan(
        state,
        DecisionPlan {
            records: vec![observation, product],
            invalidations: vec![],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Retain preview source assertions, not financial drafts".into(),
            }),
        },
    );
    source
}

fn dated_draft(
    source: SourceFacetBindingV1,
    serial: u32,
    counter: u32,
    day: &str,
) -> ActivityRequestV1 {
    let mut draft = fixture::posted(source, serial, counter);
    let date = fixture::date(day);
    draft.legs[0].leg.effective = date.clone();
    draft.legs[0].leg.posting = date.clone();
    let terms = draft.financial.as_mut().unwrap();
    terms.bookkeeping_date = date.clone();
    for posting in &mut terms.postings {
        posting.posting = date.clone();
        posting.effective = date.clone();
    }
    draft
}

#[test]
fn cur_016_1_cross_account_backfill() {
    use currents_domain::segments::{
        SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1, SegmentReemissionV1,
    };

    let (mut state, prior, transfer) = original_segment(false);
    let exact = retain_proposal_evidence(
        &mut state,
        810,
        fixture::amount(3400),
        "2026-10-30",
        ObservationKind::Balance,
    );
    let cent = Amount::new(1, 2, exact.amount.unit_id).unwrap();
    let discrepant = retain_proposal_evidence(
        &mut state,
        820,
        exact.amount.checked_add(&cent).unwrap(),
        "2026-10-30",
        ObservationKind::Balance,
    );
    let movement = retain_proposal_evidence(
        &mut state,
        830,
        fixture::amount(-100),
        "2026-10-31",
        ObservationKind::Movement,
    );
    let large_amount = fixture::amount(10000000000000000000000);
    assert!(large_amount.checked_rescale(2).is_ok());
    assert!(large_amount.checked_rescale(18).is_err());
    let large_open = retain_proposal_evidence(
        &mut state,
        840,
        large_amount,
        "2026-10-30",
        ObservationKind::Balance,
    );
    let large_movement_amount = fixture::amount(3300)
        .checked_add(&large_amount.checked_neg().unwrap())
        .unwrap();
    let large_movement = retain_proposal_evidence(
        &mut state,
        850,
        large_movement_amount,
        "2026-10-31",
        ObservationKind::Movement,
    );
    let opening_draft = |source: SourceFacetBindingV1| {
        let mut draft = dated_draft(source.clone(), 900, 50, "2026-10-30");
        draft.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
            segment_id: id(907),
            component_id: prior.component,
            amount: source.amount,
            cutoff: fixture::date("2026-10-30"),
            evidence: source,
            included_activity: vec![],
        });
        draft
    };
    let mut backfill = dated_draft(movement, 920, 30, "2026-10-31");
    // Distinct value and booking dates are supported, provided all posting
    // dates agree with the explicit bookkeeping date.
    let booked = fixture::date("2026-11-02");
    backfill.legs[0].leg.posting = booked.clone();
    let terms = backfill.financial.as_mut().unwrap();
    terms.bookkeeping_date = booked.clone();
    for posting in &mut terms.postings {
        posting.posting = booked.clone();
    }
    let reemissions: Vec<_> = transfer
        .iter()
        .enumerate()
        .map(|(offset, original)| {
            let fresh = 970 + 10 * u32::try_from(offset).unwrap();
            let AuthoritativeRecordV1::Recognition(prior) = &state.records[&RecordId(original.0)]
            else {
                panic!("retained original missing");
            };
            SegmentReemissionV1 {
                original: *original,
                replacement: RecognitionId(id(fresh).0),
                posting_ids: prior
                    .postings
                    .iter()
                    .enumerate()
                    .map(|(i, _)| id(fresh + 1 + u32::try_from(i).unwrap()))
                    .collect(),
            }
        })
        .collect();
    let mut request = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component: prior.component,
            prior_segment: Some(prior.id),
            opening: RecognitionId(id(903).0),
            included_activity: vec![RecognitionId(id(923).0)],
            replacement_closure: std::iter::once(prior.opening)
                .chain(transfer.iter().copied())
                .collect(),
            approved_accounts: [10, 30, 50]
                .into_iter()
                .map(|n| AccountId(id(n).0))
                .collect(),
            residual_disposition: SourceValue::Absent,
        },
        opening: opening_draft(exact),
        backfill: vec![backfill],
        reemissions,
    };
    let before_records = state.records.clone();
    let before_index = state.accounting.clone();
    let SegmentPreviewV1::Ready(plan) = SegmentEngine::replace(&state, &request).unwrap() else {
        panic!("3400 - 100 must agree with the 3300 prior boundary");
    };
    let frozen = AccountingAdapter::freeze(&plan.accounting, &state).unwrap();
    let mut durable_attempt = state.clone();
    assert!(
        replay_frozen_plan(&mut durable_attempt, plan.as_ref().clone()).is_err(),
        "cross-account closure remains a preview; it cannot activate a partial family"
    );
    assert_eq!(durable_attempt.records, before_records);
    assert_eq!(durable_attempt.through, state.through);

    let original_index = state.accounting.as_ref().unwrap();
    for old_id in &request.request.replacement_closure {
        let [original] = original_index.recognitions[old_id].as_slice() else {
            panic!("fixture must have exactly one frozen entry per recognition");
        };
        let [reversal] = frozen
            .operations
            .iter()
            .filter_map(|op| match op {
                FrozenNativeOperationV1::Entry(e) if e.reverses.as_ref() == Some(&original.key) => {
                    Some(e)
                }
                _ => None,
            })
            .collect::<Vec<_>>()[..]
        else {
            panic!("each affected full native recognition must reverse exactly once");
        };
        assert_eq!(reversal.recognition_id, *old_id);
        assert_eq!(reversal.booking_date, original.booking_date);
        assert_eq!(reversal.value_date, original.value_date);
        assert_eq!(reversal.postings.len(), original.postings.len());
        for (actual, original) in reversal.postings.iter().zip(&original.postings) {
            let mut inverse = original.original.clone();
            inverse.amount = inverse.amount.checked_neg().unwrap();
            assert_eq!(actual.original, inverse);
            assert_eq!(actual.unit, original.unit);
        }
    }
    for reemission in &request.reemissions {
        let [original] = original_index.recognitions[&reemission.original].as_slice() else {
            panic!("fixture must have exactly one retained transfer entry");
        };
        let [unchanged] = frozen
            .operations
            .iter()
            .filter_map(|op| match op {
                FrozenNativeOperationV1::Entry(e) if e.recognition_id == reemission.replacement => {
                    Some(e)
                }
                _ => None,
            })
            .collect::<Vec<_>>()[..]
        else {
            panic!("each retained counterpart, clearing and separate fee must reemit exactly once");
        };
        assert!(unchanged.reverses.is_none());
        assert_eq!(unchanged.booking_date, original.booking_date);
        assert_eq!(unchanged.value_date, original.value_date);
        assert_eq!(unchanged.postings.len(), original.postings.len());
        for ((actual, original), fresh) in unchanged
            .postings
            .iter()
            .zip(&original.postings)
            .zip(&reemission.posting_ids)
        {
            let mut retained = original.original.clone();
            retained.id = *fresh;
            assert_eq!(actual.original, retained);
            assert_eq!(actual.unit, original.unit);
        }
    }
    assert!(frozen.operations.iter().any(|op| matches!(op,
        FrozenNativeOperationV1::Entry(e)
            if e.recognition_id == request.request.included_activity[0]
                && e.booking_date == "2026-11-02"
                && e.value_date == "2026-10-31"
    )));
    assert_eq!(
        frozen.operations.len(),
        request.request.replacement_closure.len() + transfer.len() + 2
    );

    let mut residual = request.clone();
    residual.opening = opening_draft(discrepant);
    let SegmentPreviewV1::UnresolvedResidual(amount) =
        SegmentEngine::replace(&state, &residual).unwrap()
    else {
        panic!("one cent cannot yield a native plan or silent plug");
    };
    assert_eq!(amount, cent);
    let mut stale = state.clone();
    stale.accounting.as_mut().unwrap().prefix.domain_digest = Digest::default();
    assert!(SegmentEngine::replace(&stale, &request).is_err());

    let mut retired = state.clone();
    retired
        .accounting
        .as_mut()
        .unwrap()
        .reversed
        .insert(transfer[1]);
    assert!(SegmentEngine::replace(&retired, &request).is_err());

    let mut wrong_native_date = state.clone();
    wrong_native_date
        .accounting
        .as_mut()
        .unwrap()
        .recognitions
        .get_mut(&transfer[1])
        .unwrap()[0]
        .booking_date = "2026-11-04".into();
    assert!(SegmentEngine::replace(&wrong_native_date, &request).is_err());

    let mut duplicate_identity = request.clone();
    duplicate_identity.reemissions[0].posting_ids[0] = duplicate_identity
        .opening
        .financial
        .as_ref()
        .unwrap()
        .postings[0]
        .id;
    assert!(SegmentEngine::replace(&state, &duplicate_identity).is_err());

    let (mut legacy, _, _) = original_segment(true);
    retain_proposal_evidence(
        &mut legacy,
        810,
        fixture::amount(3400),
        "2026-10-30",
        ObservationKind::Balance,
    );
    retain_proposal_evidence(
        &mut legacy,
        830,
        fixture::amount(-100),
        "2026-10-31",
        ObservationKind::Movement,
    );
    assert_eq!(
        SegmentEngine::replace(&legacy, &request).unwrap_err().class,
        ErrorClass::UnsupportedVersion,
        "an equal aggregate must not mask unremapped movement evidence in the prior opening"
    );

    let mut wrong_backfill_booking = request.clone();
    wrong_backfill_booking.backfill[0]
        .financial
        .as_mut()
        .unwrap()
        .postings[0]
        .posting = fixture::date("2026-11-03");
    assert!(
        SegmentEngine::replace(&state, &wrong_backfill_booking).is_err(),
        "Ready must never carry fresh draft dates rejected by C05 freeze"
    );

    let mut year_zero_booking = request.clone();
    let terms = year_zero_booking.backfill[0].financial.as_mut().unwrap();
    let year_zero = fixture::date("0000-01-01");
    terms.bookkeeping_date = year_zero.clone();
    for posting in &mut terms.postings {
        posting.posting = year_zero.clone();
    }
    let result = SegmentEngine::replace(&state, &year_zero_booking);
    if let Ok(SegmentPreviewV1::Ready(plan)) = &result {
        assert!(AccountingAdapter::freeze(&plan.accounting, &state).is_err());
    }
    assert!(
        result.is_err(),
        "uniform booking dates outside C05's native years must not yield Ready"
    );

    let mut overflow = request.clone();
    overflow.opening = opening_draft(large_open);
    overflow.backfill = vec![dated_draft(large_movement, 920, 30, "2026-10-31")];
    assert!(
        SegmentEngine::replace(&state, &overflow).is_err(),
        "reconciled balanced values may still be unrepresentable at signed native scale 18"
    );

    let omitted_counterpart = transfer[1];
    request
        .request
        .replacement_closure
        .retain(|id| *id != omitted_counterpart);
    request
        .reemissions
        .retain(|reemit| reemit.original != omitted_counterpart);
    assert!(SegmentEngine::replace(&state, &request).is_err());
    assert_eq!(state.records, before_records);
    let before_index = before_index.unwrap();
    let after_index = state.accounting.as_ref().unwrap();
    assert_eq!(after_index.prefix, before_index.prefix);
    assert_eq!(after_index.recognitions, before_index.recognitions);
    assert_eq!(after_index.reversed, before_index.reversed);
}

#[test]
fn cur_016_standalone_segment_family_replays_without_overwriting_old_opening() {
    use currents_domain::segments::{SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1};

    // No transfer or association authority exists in this minimum durable case.
    let mut state = fixture::monetary_state();
    let prior = retain_original_opening(&mut state, false);
    let balance = retain_proposal_evidence(
        &mut state,
        810,
        fixture::amount(3400),
        "2026-10-30",
        ObservationKind::Balance,
    );
    let movement = retain_proposal_evidence(
        &mut state,
        830,
        fixture::amount(-100),
        "2026-10-31",
        ObservationKind::Movement,
    );
    let next_segment = id(907);
    let mut opening = dated_draft(balance.clone(), 900, 50, "2026-10-30");
    opening.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: next_segment,
        component_id: prior.component,
        amount: balance.amount,
        cutoff: fixture::date("2026-10-30"),
        evidence: balance,
        included_activity: vec![],
    });
    let new_opening = RecognitionId(id(903).0);
    let new_backfill = RecognitionId(id(923).0);
    let proposal = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component: prior.component,
            prior_segment: Some(prior.id),
            opening: new_opening,
            included_activity: vec![new_backfill],
            replacement_closure: vec![prior.opening],
            approved_accounts: [10, 30, 50]
                .into_iter()
                .map(|n| AccountId(id(n).0))
                .collect(),
            residual_disposition: SourceValue::Absent,
        },
        opening,
        backfill: vec![dated_draft(movement, 920, 30, "2026-10-31")],
        reemissions: vec![],
    };
    let SegmentPreviewV1::Ready(plan) = SegmentEngine::replace(&state, &proposal).unwrap() else {
        panic!("the standalone earlier anchor reconciles exactly");
    };
    let frozen = AccountingAdapter::freeze(&plan.accounting, &state).unwrap();
    assert_eq!(frozen.operations.len(), 3);
    let old_prefix = state.clone();
    let mut wrong_key = frozen.clone();
    let FrozenNativeOperationV1::Entry(reversal) = &mut wrong_key.operations[0] else {
        panic!("expected an original opening reversal");
    };
    reversal.reverses = Some("different-native-opening-key".into());
    wrong_key.decision_digest = AccountingAdapter::prebinding_digest(&wrong_key).unwrap();
    for operation in &mut wrong_key.operations {
        if let FrozenNativeOperationV1::Entry(entry) = operation {
            entry.generated_document = wrong_key.decision_digest;
        }
    }
    assert!(replay_with_frozen_batch(&mut state, plan.as_ref().clone(), &wrong_key).is_err());

    let mut wrong_component = plan.as_ref().clone();
    let other_component = wrong_component
        .records
        .iter_mut()
        .find_map(|record| match record {
            AuthoritativeRecordV1::AccountingSegment(segment) => Some(segment),
            _ => None,
        })
        .unwrap();
    other_component.component = id(31); // Real component, but not the original's component.
    assert!(state.records.contains_key(&id(31)));
    assert!(replay_frozen_plan(&mut state, wrong_component).is_err());

    let mut forged = (*plan).clone();
    let other = forged
        .records
        .iter_mut()
        .find_map(|record| match record {
            AuthoritativeRecordV1::AccountingSegment(segment) => Some(segment),
            _ => None,
        })
        .unwrap();
    other.prior_segment = Some(next_segment); // Self-link, not a new root.
    assert!(replay_frozen_plan(&mut state, forged).is_err());
    assert_eq!(state.through, old_prefix.through);
    assert_eq!(state.domain_digest, old_prefix.domain_digest);
    assert_eq!(state.records, old_prefix.records);
    assert_eq!(
        state.accounting.as_ref().unwrap().prefix,
        old_prefix.accounting.as_ref().unwrap().prefix,
    );

    let accepted = replay_frozen_plan(&mut state, *plan).expect(
        "C08 must replay an exact standalone segment-family successor, not a new initial opening",
    );
    assert_eq!(state.through, CommitSeq(old_prefix.through.0 + 1));
    for (key, record) in &old_prefix.records {
        assert_eq!(state.records.get(key), Some(record));
    }
    let Some(AuthoritativeRecordV1::AccountingSegment(successor)) =
        state.records.get(&next_segment)
    else {
        panic!("next immutable segment was not retained");
    };
    assert_eq!(successor.prior_segment, Some(prior.id));
    assert_eq!(successor.component, prior.component);
    assert_eq!(successor.opening, new_opening);
    assert_eq!(successor.included_activity, vec![new_backfill]);
    let native = state.accounting.as_ref().unwrap();
    assert!(native.reversed.contains(&prior.opening));
    assert!(native.recognitions.contains_key(&new_opening));
    assert!(!native.reversed.contains(&new_opening));
    assert!(
        !old_prefix
            .accounting
            .as_ref()
            .unwrap()
            .reversed
            .contains(&prior.opening)
    );
    RecognitionEngine::audit(&state)
        .expect("historical opening and active successor must both audit");

    // Domain-only replay guard: a second append with the same root may not
    // replace the accepted successor, even with an otherwise valid envelope.
    let mut second = accepted;
    second.seq = CommitSeq(state.through.0 + 1);
    second.previous_digest = state.domain_digest;
    second.command_result.commit = Some(second.seq);
    second.command_result.command_id = CommandId(id(31_999).0);
    second.digest = Digest::default();
    second.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&second).unwrap()).into());

    let before_fork = state.clone();
    assert!(ReplayEngine::apply(&mut state, &second).is_err());
    assert_eq!(state.records, before_fork.records);
    assert_eq!(state.through, before_fork.through);

    let next_source = retain_proposal_evidence(
        &mut state,
        860,
        fixture::amount(-42),
        "2026-11-01",
        ObservationKind::Movement,
    );
    let retained_draft = dated_draft(next_source, 1020, 30, "2026-11-01");
    assert!(
        RecognitionEngine::adopt(&state, &retained_draft).is_ok(),
        "only the active earlier cutoff governs later movements, not the archived root"
    );
}

#[test]
fn cur_016_1_cross_account_backfill_replays_the_entire_transfer_family() {
    use currents_domain::segments::{
        SegmentEngine, SegmentPreviewRequestV1, SegmentPreviewV1, SegmentReemissionV1,
    };
    use std::collections::BTreeSet;

    let (mut state, prior, transfer) = original_segment(false);
    let balance = retain_proposal_evidence(
        &mut state,
        810,
        fixture::amount(3400),
        "2026-10-30",
        ObservationKind::Balance,
    );
    let movement = retain_proposal_evidence(
        &mut state,
        830,
        fixture::amount(-100),
        "2026-10-31",
        ObservationKind::Movement,
    );
    let new_segment = id(907);
    let mut opening = dated_draft(balance.clone(), 900, 50, "2026-10-30");
    opening.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: new_segment,
        component_id: prior.component,
        amount: balance.amount,
        cutoff: fixture::date("2026-10-30"),
        evidence: balance,
        included_activity: vec![],
    });
    let reemissions: Vec<_> = transfer
        .iter()
        .enumerate()
        .map(|(ordinal, original)| {
            let fresh = 1070 + 10 * u32::try_from(ordinal).unwrap();
            let AuthoritativeRecordV1::Recognition(r) = &state.records[&RecordId(original.0)]
            else {
                panic!("retained transfer recognition missing");
            };
            SegmentReemissionV1 {
                original: *original,
                replacement: RecognitionId(id(fresh).0),
                posting_ids: r
                    .postings
                    .iter()
                    .enumerate()
                    .map(|(i, _)| id(fresh + 1 + u32::try_from(i).unwrap()))
                    .collect(),
            }
        })
        .collect();
    let covered: Vec<_> = std::iter::once(prior.opening)
        .chain(transfer.iter().copied())
        .collect();
    let fixed_approval: Vec<_> = [10, 30, 50]
        .into_iter()
        .map(|n| AccountId(id(n).0))
        .collect();
    let proposal = SegmentPreviewRequestV1 {
        request: SegmentRequestV1 {
            component: prior.component,
            prior_segment: Some(prior.id),
            opening: RecognitionId(id(903).0),
            included_activity: vec![RecognitionId(id(923).0)],
            replacement_closure: covered.clone(),
            approved_accounts: fixed_approval.clone(),
            residual_disposition: SourceValue::Absent,
        },
        opening,
        backfill: vec![dated_draft(movement, 920, 30, "2026-10-31")],
        reemissions,
    };
    let authority_id = id(941);
    let SegmentPreviewV1::Ready(plan) =
        SegmentEngine::replace_owned(&state, &proposal, authority_id).unwrap()
    else {
        panic!("the complete, approved earlier opening reconciles exactly");
    };
    let frozen = AccountingAdapter::freeze(&plan.accounting, &state).unwrap();
    assert_eq!(frozen.operations.len(), covered.len() + transfer.len() + 2);
    let affected: BTreeSet<_> = frozen
        .operations
        .iter()
        .flat_map(|op| match op {
            FrozenNativeOperationV1::Entry(entry) => entry.postings.iter(),
            _ => panic!("segment must freeze to whole native entries"),
        })
        .map(|posting| posting.original.account_id)
        .collect();
    assert_eq!(
        affected,
        [10, 30, 50, 60, 70, 80]
            .into_iter()
            .map(|n| AccountId(id(n).0))
            .collect()
    );
    assert_ne!(affected, fixed_approval.iter().copied().collect());
    let Some(AuthoritativeRecordV1::SegmentReplacementAuthority(owner)) = plan.records.last()
    else {
        panic!("qualified cross-account closure requires a typed witness");
    };
    assert_eq!(owner.id, authority_id);
    assert_eq!(
        owner
            .affected_accounts
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        affected
    );
    assert_eq!(owner.counterparts.len(), transfer.len());

    let before = state.clone();
    let mut incomplete = plan.as_ref().clone();
    let segment = incomplete
        .records
        .iter_mut()
        .find_map(|r| match r {
            AuthoritativeRecordV1::AccountingSegment(s) => Some(s),
            _ => None,
        })
        .unwrap();
    let omitted_counterpart = transfer[1];
    segment
        .replacement_closure
        .retain(|id| *id != omitted_counterpart);
    assert!(replay_with_frozen_batch(&mut state, incomplete, &frozen).is_err());
    assert_eq!(state.records, before.records);
    assert_eq!(
        state.accounting.as_ref().unwrap().prefix,
        before.accounting.as_ref().unwrap().prefix
    );

    for forged_accounts in [true, false] {
        let mut forged = plan.as_ref().clone();
        let Some(AuthoritativeRecordV1::SegmentReplacementAuthority(owner)) =
            forged.records.last_mut()
        else {
            panic!("typed simulated owner required");
        };
        if forged_accounts {
            owner.affected_accounts.pop();
        } else {
            owner.counterparts[1].replacement = owner.counterparts[0].replacement;
        }
        assert!(replay_with_frozen_batch(&mut state, forged, &frozen).is_err());
        assert_eq!(state.records, before.records);
        assert_eq!(state.through, before.through);
    }

    let mut with_dependency = state.clone();
    retain_frozen_plan(
        &mut with_dependency,
        DecisionPlan {
            records: vec![AuthoritativeRecordV1::Review(ReviewRecordV1 {
                id: id(1234),
                targets: vec![RecordId(transfer[1].0)],
                disposition: ReviewDisposition::NeedsEvidence,
                reason: "Existing predicate dependency must be preserved".into(),
            })],
            invalidations: vec![],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Retained reviewed dependency".into(),
            }),
        },
    );
    assert!(SegmentEngine::replace_owned(&with_dependency, &proposal, authority_id).is_err());

    replay_with_frozen_batch(&mut state, *plan, &frozen)
        .expect("C08 and C05 must activate the entire retained cross-account family together");
    let Some(AuthoritativeRecordV1::AccountingSegment(next)) = state.records.get(&new_segment)
    else {
        panic!("atomic family replay lost its immutable successor");
    };
    assert_eq!(next.prior_segment, Some(prior.id));
    assert_eq!(next.replacement_closure, covered);
    assert_eq!(next.approved_accounts, fixed_approval);
    assert!(matches!(state.records.get(&authority_id),
        Some(AuthoritativeRecordV1::SegmentReplacementAuthority(authority))
            if authority.segment == new_segment && authority.prior_segment == prior.id
                && authority.affected_accounts.iter().copied().collect::<BTreeSet<_>>() == affected
    ));
    for (key, record) in &before.records {
        assert_eq!(state.records.get(key), Some(record));
    }
    let old_index = before.accounting.as_ref().unwrap();
    let current = state.accounting.as_ref().unwrap();
    for original in &covered {
        assert!(current.reversed.contains(original));
        assert!(!old_index.reversed.contains(original));
    }
    for reemission in &proposal.reemissions {
        let [old] = old_index.recognitions[&reemission.original].as_slice() else {
            panic!()
        };
        let [new] = current.recognitions[&reemission.replacement].as_slice() else {
            panic!()
        };
        assert!(!current.reversed.contains(&reemission.replacement));
        assert_eq!(new.booking_date, old.booking_date);
        assert_eq!(new.value_date, old.value_date);
        assert_eq!(new.postings.len(), old.postings.len());
        for ((original, copied), fresh_id) in old
            .postings
            .iter()
            .zip(&new.postings)
            .zip(&reemission.posting_ids)
        {
            let mut expected = original.clone();
            expected.original.id = *fresh_id;
            assert_eq!(*copied, expected);
        }
    }
    RecognitionEngine::audit(&state).unwrap();
    IdentityEngine::audit(&state).unwrap();
    let current_counterpart = proposal.reemissions[1].replacement;
    let mut changed_current = state.clone();
    let Some(AuthoritativeRecordV1::Recognition(r)) = changed_current
        .records
        .get_mut(&RecordId(current_counterpart.0))
    else {
        panic!("expected the preserved live recognition");
    };
    r.postings[0].amount.coefficient -= 1;
    r.postings[1].amount.coefficient += 1;
    assert!(IdentityEngine::audit(&changed_current).is_err());
}
