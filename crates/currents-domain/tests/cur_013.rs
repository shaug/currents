//! CUR013 public domain behavior. All identities and evidence are synthetic.
use currents_domain::catalog::CatalogEngine;
use currents_types::*;
use serde_json::Value;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;

#[test]
fn cur_013_catalog_commands() {
    let mut state = state();
    let before = state.clone();
    let mut request = catalog();
    let plan = CatalogEngine::set_complete(&state, &request).unwrap();
    let AccountingIntent::MetadataDeclaration(declaration) = &plan.accounting else {
        panic!("catalog must use declaration-only intent")
    };
    assert_eq!(declaration.accounts.len(), 1);
    let original = declaration.accounts[0].clone();
    assert_eq!(original.opening_boundary, SourceValue::Unknown);
    assert_eq!(original.ownership, request.ownership);
    apply(&mut state, &plan);
    request.prior_revision = Some(request.revision_id);
    request.revision_id = id(13);
    request.catalog.display_name = "Renamed checking".into();
    request.catalog.lifecycle = Lifecycle::Closed;
    let update = CatalogEngine::set_complete(&state, &request).unwrap();
    assert!(matches!(update.accounting, AccountingIntent::NoEffect(_)));
    assert!(
        update
            .records
            .iter()
            .all(|r| !matches!(r, AuthoritativeRecordV1::AccountComponent(_)))
    );
    apply(&mut state, &update);
    assert_eq!(
        state.records.get(&id(11)),
        Some(&AuthoritativeRecordV1::AccountComponent(original))
    );
    assert_eq!(
        CatalogEngine::current(&state, id(11))
            .unwrap()
            .catalog
            .display_name,
        "Renamed checking"
    );
    assert!(CatalogEngine::current(&before, id(11)).is_err());
    assert!(CatalogEngine::set_complete(&state, &request).is_err());
    request.prior_revision = Some(id(13));
    request.revision_id = id(14);
    request.catalog.unit_id = UnitId(id(999).0);
    assert!(CatalogEngine::set_complete(&state, &request).is_err());
}
#[test]
fn catalog_missing_ownership_and_invalid_control_reject() {
    let mut state = state();
    state.records.remove(&id(100));
    let mut request = catalog();
    assert_eq!(
        CatalogEngine::set_complete(&state, &request)
            .unwrap_err()
            .class,
        ErrorClass::IncompleteBasis
    );
    request.origin = AccountOriginV1::Internal(InternalAccountV1 {
        role: ControlRoleV1::UnresolvedEquity,
        created_at: date("2026-11-01"),
    });
    assert!(CatalogEngine::set_complete(&state, &request).is_err());
    request.catalog.account_class = AccountClass::Equity;
    request.catalog.normal_sign = NormalSign::Credit;
    let plan = CatalogEngine::set_complete(&state, &request).unwrap();
    let AccountingIntent::MetadataDeclaration(d) = plan.accounting else {
        panic!()
    };
    assert_eq!(
        d.accounts[0].opening_boundary,
        SourceValue::Known(date("2026-11-01"))
    );
    assert!(
        plan.records
            .iter()
            .all(|r| !matches!(r, AuthoritativeRecordV1::Recognition(_)))
    );
    request.schema_version = 2;
    assert_eq!(
        CatalogEngine::set_complete(&state, &request)
            .unwrap_err()
            .class,
        ErrorClass::UnsupportedVersion
    );
}
#[test]
fn legacy_record_bytes_are_unchanged() {
    let old: Value =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    let records: Vec<AuthoritativeRecordV1> = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(records).unwrap(), old);
}

#[test]
fn pending_is_explicit_durable_domain_only_and_never_age_expires() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = activity_state();
    let binding = evidence(&mut state, 200, 80, None);
    let request = pending(binding.clone(), 300);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    assert!(
        plan.records
            .iter()
            .all(|r| !matches!(r, AuthoritativeRecordV1::Recognition(_)))
    );
    apply(&mut state, &plan);
    let exposures = RecognitionEngine::pending(&state).unwrap();
    assert_eq!(exposures.len(), 1);
    assert_eq!(exposures[0].id, id(300));
    assert_eq!(exposures[0].facets[0].source, binding);
    // Projection has no clock/expiry argument; later unrelated commits cannot expire evidence.
    state.through = CommitSeq(10_000);
    assert_eq!(RecognitionEngine::pending(&state).unwrap(), exposures);
    assert!(RecognitionEngine::adopt(&state, &request).is_err());
    assert!(RecognitionEngine::adopt(&state, &pending(binding, 400)).is_err());
}
#[test]
fn pending_requires_exact_adopted_product_policy_and_facet() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = activity_state();
    let binding = evidence(&mut state, 200, 80, None);
    let valid = pending(binding, 300);
    assert!(RecognitionEngine::adopt(&state, &valid).is_ok());
    let mut bad = valid.clone();
    bad.legs[0].sources[0].parse_product = id(200);
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid.clone();
    bad.governing_policy.version = 2;
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid.clone();
    bad.legs[0].sources[0].facet = id(901);
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid.clone();
    bad.legs[0].leg.movement = amount(96);
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid;
    bad.schema_version = 2;
    assert_eq!(
        RecognitionEngine::adopt(&state, &bad).unwrap_err().class,
        ErrorClass::UnsupportedVersion
    );
}

#[test]
fn pending_reparse_and_source_revision_cannot_create_another_exposure() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = activity_state();
    let binding = evidence(&mut state, 200, 80, None);
    let first = pending(binding.clone(), 300);
    let plan = RecognitionEngine::adopt(&state, &first).unwrap();
    apply(&mut state, &plan);
    // A new parser product for the exact same observation/facet does not free ownership.
    let AuthoritativeRecordV1::AddressedParseProduct(mut reparse) = state.records[&id(201)].clone()
    else {
        panic!()
    };
    reparse.id = id(205);
    reparse.product.parser_version = "2".into();
    state.records.insert(
        id(205),
        AuthoritativeRecordV1::AddressedParseProduct(reparse),
    );
    let mut next = binding;
    next.parse_product = id(205);
    assert!(RecognitionEngine::adopt(&state, &pending(next, 400)).is_err());
    // Explicit source revisions retain the original fact lineage even with fresh IDs.
    let revision = evidence(&mut state, 210, 80, Some(id(200)));
    assert!(RecognitionEngine::adopt(&state, &pending(revision, 500)).is_err());
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
}

#[test]
fn cur_013_1() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let binding = evidence(&mut state, 200, -80, None);
    let request = pending(binding, 300);
    let first = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &first);
    let old = state.records.clone();
    let final_source = evidence(&mut state, 210, -96, Some(id(200)));
    let mut request = posted(final_source.clone(), 400, 30);
    request.prior_pending = Some(id(300));
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    let AccountingIntent::Recognition(recognition) = &plan.accounting else {
        panic!("posted activity requires money")
    };
    assert_eq!(recognition.postings.len(), 2);
    assert_eq!(recognition.postings[0].amount, amount(-96));
    apply(&mut state, &plan);
    assert!(RecognitionEngine::pending(&state).unwrap().is_empty());
    for (key, value) in old {
        assert_eq!(state.records.get(&key), Some(&value));
    }
    let later = evidence(&mut state, 220, -96, Some(id(210)));
    let mut duplicate = posted(later, 500, 30);
    duplicate.prior_pending = Some(id(300));
    assert!(RecognitionEngine::adopt(&state, &duplicate).is_err());
    let another = posted(final_source, 600, 30);
    assert!(RecognitionEngine::adopt(&state, &another).is_err());
    assert!(RecognitionEngine::pending(&state).unwrap().is_empty());
}
#[test]
fn cur_013_2() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let source = evidence(&mut state, 200, -96, None);
    let corroboration = evidence(&mut state, 210, -96, None);
    let mut request = posted(source.clone(), 300, 30);
    request.legs[0].sources.push(corroboration.clone());
    request.legs[0].leg.allocations.push(EvidenceAllocationV1 {
        observation: corroboration.observation,
        facet: corroboration.facet,
        amount: corroboration.amount,
    });
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    let AccountingIntent::Recognition(r) = &plan.accounting else {
        panic!()
    };
    assert_eq!(r.postings[0].amount, amount(-96));
    assert_eq!(r.leg_ids.len(), 1);
    assert_eq!(r.evidence_allocations.len(), 2);
    apply(&mut state, &plan);
    let mut same_leg = posted(source.clone(), 400, 30);
    same_leg.legs[0].leg.id = request.legs[0].leg.id;
    same_leg.event.legs = vec![request.legs[0].leg.id];
    assert!(RecognitionEngine::adopt(&state, &same_leg).is_err());
    assert!(RecognitionEngine::adopt(&state, &posted(source, 500, 30)).is_err());
    assert!(RecognitionEngine::adopt(&state, &posted(corroboration, 600, 30)).is_err());
}
fn unknown_opening_case() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let source = evidence(&mut state, 200, 3300, None);
    let mut request = posted(source, 300, 40);
    request.financial.as_mut().unwrap().unresolved_account = Some(AccountId(id(40).0));
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    let AccountingIntent::Recognition(r) = &plan.accounting else {
        panic!()
    };
    assert_eq!(r.kind, RecognitionKind::Activity);
    assert_eq!(r.postings[0].amount, amount(3300));
    assert_eq!(r.postings[1].account_id, AccountId(id(40).0));
    assert!(
        plan.records
            .iter()
            .all(|r| !matches!(r, AuthoritativeRecordV1::AccountingSegment(_)))
    );
    let Some(AuthoritativeRecordV1::AccountComponent(c)) = state.records.get(&id(11)) else {
        panic!()
    };
    assert_eq!(c.opening_boundary, SourceValue::Unknown);
    let mut bad = request;
    bad.financial.as_mut().unwrap().unresolved_account = None;
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
}
#[test]
fn cur_013_4() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let source = evidence(&mut state, 200, -96, None);
    let valid = posted(source, 300, 30);
    assert!(RecognitionEngine::adopt(&state, &valid).is_ok());
    let mut invalid = Vec::new();
    let mut bad = valid.clone();
    bad.financial.as_mut().unwrap().postings.pop();
    invalid.push(bad);
    let mut bad = valid.clone();
    bad.financial.as_mut().unwrap().postings.clear();
    invalid.push(bad);
    let mut bad = valid.clone();
    bad.financial.as_mut().unwrap().postings[1].amount = amount(95);
    invalid.push(bad);
    let mut bad = valid.clone();
    bad.financial.as_mut().unwrap().postings[1].amount.unit_id = UnitId(id(999).0);
    invalid.push(bad);
    let mut bad = valid.clone();
    bad.financial.as_mut().unwrap().postings[0].amount =
        Amount::new(-96001, 3, UnitId(id(1).0)).unwrap();
    invalid.push(bad);
    let mut bad = valid.clone();
    bad.financial.as_mut().unwrap().bookkeeping_date = TemporalValue::Unknown {
        original_text: "missing".into(),
    };
    invalid.push(bad);
    let mut bad = valid.clone();
    bad.financial.as_mut().unwrap().date_policy = PolicyReferenceV1 {
        id: id(104),
        version: 1,
    };
    invalid.push(bad);
    let before = state.records.clone();
    for bad in invalid {
        assert!(RecognitionEngine::adopt(&state, &bad).is_err());
        assert_eq!(state.records, before);
    }
    state.accounting = None;
    assert!(RecognitionEngine::adopt(&state, &valid).is_err());
}

#[test]
fn catalog_closure_does_not_erase_previously_adopted_pending() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = activity_state();
    let source = evidence(&mut state, 200, 80, None);
    let request = pending(source, 300);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &plan);
    let mut closed = catalog();
    closed.revision_id = id(13);
    closed.prior_revision = Some(id(12));
    closed.catalog.lifecycle = Lifecycle::Closed;
    let plan = CatalogEngine::set_complete(&state, &closed).unwrap();
    apply(&mut state, &plan);
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
}

#[test]
fn cur_013_3() {
    unknown_opening_case();
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let request = supported_opening(&mut state);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    let AccountingIntent::Recognition(recognition) = &plan.accounting else {
        panic!()
    };
    assert_eq!(recognition.kind, RecognitionKind::Opening);
    let segment = plan
        .records
        .iter()
        .find_map(|r| match r {
            AuthoritativeRecordV1::AccountingSegment(s) => Some(s),
            _ => None,
        })
        .unwrap();
    assert_eq!(segment.id, id(707));
    assert_eq!(segment.component, id(11));
    assert_eq!(segment.opening, recognition.id);
    let inclusion = request
        .financial
        .as_ref()
        .unwrap()
        .opening
        .as_ref()
        .unwrap()
        .included_activity[0]
        .clone();
    apply(&mut state, &plan);
    assert!(RecognitionEngine::adopt(&state, &posted(inclusion.clone(), 800, 30)).is_err());
    let revision = evidence(&mut state, 270, -96, Some(inclusion.observation));
    assert!(RecognitionEngine::adopt(&state, &posted(revision, 900, 30)).is_err());
    // A second anchor cannot silently replace the approved initial segment.
    let mut again = request;
    again.adoption_id = id(1000);
    assert!(RecognitionEngine::adopt(&state, &again).is_err());
}
#[test]
fn unsupported_or_contradictory_opening_basis_rejects() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let valid = supported_opening(&mut state);
    assert!(RecognitionEngine::adopt(&state, &valid).is_ok());
    let mut bad = valid.clone();
    bad.financial
        .as_mut()
        .unwrap()
        .opening
        .as_mut()
        .unwrap()
        .component_id = id(31);
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid.clone();
    bad.financial
        .as_mut()
        .unwrap()
        .opening
        .as_mut()
        .unwrap()
        .amount = amount(3301);
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid.clone();
    bad.financial
        .as_mut()
        .unwrap()
        .opening
        .as_mut()
        .unwrap()
        .cutoff = TemporalValue::Unknown {
        original_text: "unknown".into(),
    };
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid.clone();
    bad.financial
        .as_mut()
        .unwrap()
        .opening
        .as_mut()
        .unwrap()
        .included_activity[0]
        .amount = amount(-97);
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    let mut bad = valid;
    bad.financial.as_mut().unwrap().opening = None;
    assert!(RecognitionEngine::adopt(&state, &bad).is_err());
}

#[test]
fn complete_catalog_ownership_is_explicit_and_conflicts_reject() {
    let mut state = state();
    let mut request = catalog();
    assert!(CatalogEngine::set_complete(&state, &request).is_ok());
    let mut wire = serde_json::to_value(&request).unwrap();
    wire.as_object_mut().unwrap().remove("ownership");
    assert!(serde_json::from_value::<CatalogRequestV1>(wire).is_err());
    request.ownership.denominator = CommitSeq(0);
    assert!(CatalogEngine::set_complete(&state, &request).is_err());
    request.ownership.denominator = CommitSeq(1);
    request.ownership.numerator = CommitSeq(2);
    assert!(CatalogEngine::set_complete(&state, &request).is_err());
    request = catalog();
    request.ownership.account_id = AccountId(id(99).0);
    assert!(CatalogEngine::set_complete(&state, &request).is_err());
    request = catalog();
    let Some(AuthoritativeRecordV1::Dataset(dataset)) = state.records.get_mut(&id(100)) else {
        panic!()
    };
    let mut conflict = request.ownership.clone();
    conflict.denominator = CommitSeq(2);
    dataset.ownership_perimeter.push(conflict);
    assert!(CatalogEngine::set_complete(&state, &request).is_err());
}

#[test]
fn opening_segment_identity_cannot_alias_another_new_record() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let valid = supported_opening(&mut state);
    assert!(RecognitionEngine::adopt(&state, &valid).is_ok());
    for collision in [
        valid.adoption_id,
        valid.event.id,
        RecordId(valid.legs[0].leg.id.0),
        RecordId(valid.financial.as_ref().unwrap().recognition_id.0),
    ] {
        let mut bad = valid.clone();
        bad.financial
            .as_mut()
            .unwrap()
            .opening
            .as_mut()
            .unwrap()
            .segment_id = collision;
        assert!(RecognitionEngine::adopt(&state, &bad).is_err());
    }
}
#[test]
fn legacy_recognized_source_revision_remains_owned_without_new_adoption_record() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let source = evidence(&mut state, 200, -96, None);
    let request = posted(source, 300, 30);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &plan);
    // Older accepted C05 recognition has its authoritative allocations/legs,
    // but predates this additive ActivityAdoption representation.
    state.records.remove(&id(300));
    let revision = evidence(&mut state, 210, -96, Some(id(200)));
    assert!(RecognitionEngine::adopt(&state, &posted(revision, 400, 30)).is_err());
}

#[test]
fn stored_adoption_audit_rejects_reused_recognition_lineage() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let first = evidence(&mut state, 200, -96, None);
    let request = posted(first, 300, 30);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &plan);
    let next = evidence(&mut state, 210, -96, None);
    let request = posted(next, 400, 30);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &plan);
    assert!(RecognitionEngine::audit(&state).is_ok());
    let Some(AuthoritativeRecordV1::Recognition(r)) = state.records.get_mut(&id(403)) else {
        panic!()
    };
    r.lineage = id(304);
    assert!(RecognitionEngine::audit(&state).is_err());
}
#[test]
fn stored_initial_opening_audit_rejects_two_independent_anchors_on_one_component() {
    use currents_domain::recognition::RecognitionEngine;
    let mut base = monetary_state();
    let first = supported_opening(&mut base);
    let second_source = evidence(&mut base, 280, 3300, None);
    let Some(AuthoritativeRecordV1::ObservationVersion(o)) = base.records.get_mut(&id(280)) else {
        panic!()
    };
    o.semantic_kind = ObservationKind::Balance;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) = base.records.get_mut(&id(281))
    else {
        panic!()
    };
    p.product.facets[0].kind = ObservationKind::Balance;
    let mut second = posted(second_source.clone(), 800, 50);
    second.financial.as_mut().unwrap().opening = Some(OpeningBasisV1 {
        segment_id: id(807),
        component_id: id(11),
        amount: amount(3300),
        cutoff: date("2026-11-01"),
        evidence: second_source,
        included_activity: vec![],
    });
    let a = RecognitionEngine::adopt(&base, &first).unwrap();
    let b = RecognitionEngine::adopt(&base, &second).unwrap();
    let mut alone = base.clone();
    apply(&mut alone, &a);
    assert!(RecognitionEngine::audit(&alone).is_ok());
    // Each anchor is individually well formed; their combined history is not.
    apply(&mut alone, &b);
    assert!(RecognitionEngine::audit(&alone).is_err());
}

#[test]
fn retained_reparse_cannot_rename_an_owned_facet() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let source = evidence(&mut state, 200, -96, None);
    let plan = RecognitionEngine::adopt(&state, &posted(source, 300, 30)).unwrap();
    apply(&mut state, &plan);
    let Some(AuthoritativeRecordV1::AddressedParseProduct(mut p)) =
        state.records.get(&id(201)).cloned()
    else {
        panic!()
    };
    p.id = id(205);
    p.product.parser_version = "2".into();
    p.product.facets[0].lineage_key = id(901);
    state
        .records
        .insert(id(205), AuthoritativeRecordV1::AddressedParseProduct(p));
    let renamed = SourceFacetBindingV1 {
        observation: id(200),
        parse_product: id(205),
        facet: id(901),
        amount: amount(-96),
    };
    assert!(RecognitionEngine::adopt(&state, &posted(renamed, 400, 30)).is_err());
    assert!(RecognitionEngine::audit(&state).is_err());
}
#[test]
fn source_revision_and_multi_facet_products_cannot_create_new_ownership() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let source = evidence(&mut state, 200, -96, None);
    let plan = RecognitionEngine::adopt(&state, &posted(source, 300, 30)).unwrap();
    apply(&mut state, &plan);
    let mut revised = evidence(&mut state, 210, -96, Some(id(200)));
    let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) = state.records.get_mut(&id(211))
    else {
        panic!()
    };
    p.product.facets[0].lineage_key = id(901);
    revised.facet = id(901);
    assert!(RecognitionEngine::adopt(&state, &posted(revised, 400, 30)).is_err());
    let mut fresh = monetary_state();
    let source = evidence(&mut fresh, 200, -96, None);
    let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) = fresh.records.get_mut(&id(201))
    else {
        panic!()
    };
    let mut extra = p.product.facets[0].clone();
    extra.lineage_key = id(901);
    p.product.facets.push(extra);
    assert!(RecognitionEngine::adopt(&fresh, &posted(source, 300, 30)).is_err());
}

#[test]
fn policy_history_projection_is_exact_and_atomic_without_latest_fallback() {
    use currents_domain::{policy_history::PolicyHistory, recognition::RecognitionEngine};
    let mut state = monetary_state();
    let source = evidence(&mut state, 200, -96, None);
    let valid = posted(source, 300, 30);
    assert!(RecognitionEngine::adopt(&state, &valid).is_ok());
    let first = state.dataset_policies[&(id(104), 1)].clone();
    let before = state.dataset_policies.clone();
    let mut second = first.clone();
    second.version = 2;
    second.freshness_seconds = CommitSeq(2);
    let mut conflict = first.clone();
    conflict.freshness_seconds = CommitSeq(99);
    assert!(
        PolicyHistory::project(
            &state,
            &[
                AuthoritativeRecordV1::DatasetPolicy(second.clone()),
                AuthoritativeRecordV1::DatasetPolicy(conflict)
            ]
        )
        .is_err()
    );
    assert_eq!(state.dataset_policies, before);
    let prospective = PolicyHistory::project(
        &state,
        &[AuthoritativeRecordV1::DatasetPolicy(second.clone())],
    )
    .unwrap();
    assert!(prospective.contains_key(&(id(104), 2)));
    assert!(!state.dataset_policies.contains_key(&(id(104), 2)));
    // A latest-record entry alone, or another policy kind, cannot satisfy an
    // unavailable exact-prefix governing-policy projection.
    state
        .records
        .insert(id(104), AuthoritativeRecordV1::DatasetPolicy(second));
    let mut future = valid.clone();
    future.governing_policy.version = 2;
    assert!(RecognitionEngine::adopt(&state, &future).is_err());
    let mut wrong = valid.clone();
    wrong.governing_policy.id = id(103);
    assert!(RecognitionEngine::adopt(&state, &wrong).is_err());
    assert!(RecognitionEngine::adopt(&state, &valid).is_ok());
    let mut zero = first.clone();
    zero.version = 0;
    assert!(PolicyHistory::project(&state, &[AuthoritativeRecordV1::DatasetPolicy(zero)]).is_err());
    state.dataset_policies.insert((id(104), 2), first);
    assert!(PolicyHistory::project(&state, &[]).is_err());
}

#[test]
fn replay_recognition_rejects_future_policy_and_orphan_authority_atomically() {
    use currents_domain::{ReplayEngine, recognition::RecognitionEngine};
    let mut prior = monetary_state();
    let source = evidence(&mut prior, 200, -96, None);
    let plan = RecognitionEngine::adopt(&prior, &posted(source, 300, 30)).unwrap();
    let seq = CommitSeq(prior.through.0 + 1);
    let commit = DomainCommitV1 {
        seq,
        previous_digest: prior.domain_digest,
        records: plan.records,
        decisions: vec![],
        native_head_after: prior
            .accounting
            .as_ref()
            .unwrap()
            .prefix
            .native_head
            .clone(),
        digest: Digest([77; 32]),
        command_result: CommandResultV1 {
            command_id: CommandId(id(999).0),
            digest: Digest([88; 32]),
            state: CommandState::Committed,
            commit: Some(seq),
            effects: EffectsV1 {
                records: vec![],
                accounts: vec![],
                amounts: vec![],
                invalidations: vec![],
            },
            health: vec![],
        },
    };
    let mut positive = prior.clone();
    ReplayEngine::apply(&mut positive, &commit).unwrap();
    assert_eq!(positive.through, seq);
    for orphan in [true, false] {
        let mut bad = commit.clone();
        if orphan {
            bad.records
                .retain(|r| !matches!(r, AuthoritativeRecordV1::ActivityAdoption(_)));
        } else {
            for r in &mut bad.records {
                if let AuthoritativeRecordV1::ActivityAdoption(a) = r {
                    a.governing_policy.version = 2;
                }
            }
            // Even co-locating a future policy with the persisted adoption must
            // not replace its actually available command-admission prefix.
            let mut future = prior.dataset_policies[&(id(104), 1)].clone();
            future.version = 2;
            bad.records
                .push(AuthoritativeRecordV1::DatasetPolicy(future));
        }
        let mut unchanged = prior.clone();
        assert!(ReplayEngine::apply(&mut unchanged, &bad).is_err());
        assert_eq!(unchanged.records, prior.records);
        assert_eq!(unchanged.dataset_policies, prior.dataset_policies);
        assert_eq!(unchanged.through, prior.through);
        assert_eq!(unchanged.domain_digest, prior.domain_digest);
    }
}

#[test]
fn anchor_posting_date_opening_must_equal_cutoff() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let valid = supported_opening(&mut state);
    let plan = RecognitionEngine::adopt(&state, &valid).unwrap();
    for effective in [date("2026-10-31"), date("2026-11-02")] {
        let mut bad = valid.clone();
        for posting in &mut bad.financial.as_mut().unwrap().postings {
            posting.effective = effective.clone();
        }
        assert!(
            RecognitionEngine::adopt(&state, &bad).is_err(),
            "opening native date must equal its cutoff"
        );
        let mut corrupted = state.clone();
        let mut stored = plan.clone();
        for record in &mut stored.records {
            if let AuthoritativeRecordV1::Recognition(r) = record {
                for posting in &mut r.postings {
                    posting.effective = effective.clone();
                }
            }
        }
        apply(&mut corrupted, &stored);
        assert!(RecognitionEngine::audit(&corrupted).is_err());
    }
}

#[test]
fn anchor_posting_date_activity_must_follow_cutoff() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    let opening = supported_opening(&mut state);
    let plan = RecognitionEngine::adopt(&state, &opening).unwrap();
    apply(&mut state, &plan);
    let source = evidence(&mut state, 280, -96, None);
    let mut valid = posted(source, 800, 30);
    valid.legs[0].leg.effective = date("2026-11-02");
    for p in &mut valid.financial.as_mut().unwrap().postings {
        p.effective = date("2026-11-02");
    }
    assert!(RecognitionEngine::adopt(&state, &valid).is_ok());
    for effective in [date("2026-10-31"), date("2026-11-01")] {
        let mut bad = valid.clone();
        for p in &mut bad.financial.as_mut().unwrap().postings {
            p.effective = effective.clone();
        }
        assert!(
            RecognitionEngine::adopt(&state, &bad).is_err(),
            "native activity must be strictly after the anchor"
        );
    }
}

fn reject_replayed_anchor_date(opening: bool) {
    use currents_domain::{ReplayEngine, recognition::RecognitionEngine};
    let mut prior = monetary_state();
    let anchor = supported_opening(&mut prior);
    let anchor_plan = RecognitionEngine::adopt(&prior, &anchor).unwrap();
    let plan = if opening {
        anchor_plan
    } else {
        apply(&mut prior, &anchor_plan);
        let source = evidence(&mut prior, 280, -96, None);
        let mut request = posted(source, 800, 30);
        request.legs[0].leg.effective = date("2026-11-02");
        for p in &mut request.financial.as_mut().unwrap().postings {
            p.effective = date("2026-11-02");
        }
        RecognitionEngine::adopt(&prior, &request).unwrap()
    };
    let seq = CommitSeq(prior.through.0 + 1);
    let mut commit = DomainCommitV1 {
        seq,
        previous_digest: prior.domain_digest,
        records: plan.records,
        decisions: vec![],
        native_head_after: prior
            .accounting
            .as_ref()
            .unwrap()
            .prefix
            .native_head
            .clone(),
        digest: Digest([77; 32]),
        command_result: CommandResultV1 {
            command_id: CommandId(id(999).0),
            digest: Digest([88; 32]),
            state: CommandState::Committed,
            commit: Some(seq),
            effects: EffectsV1 {
                records: vec![],
                accounts: vec![],
                amounts: vec![],
                invalidations: vec![],
            },
            health: vec![],
        },
    };
    let mut positive = prior.clone();
    ReplayEngine::apply(&mut positive, &commit).unwrap();
    for r in &mut commit.records {
        if let AuthoritativeRecordV1::Recognition(r) = r {
            for p in &mut r.postings {
                p.effective = date("2026-10-31");
            }
            let TemporalValue::Date { civil, .. } = date("2026-10-31") else {
                panic!()
            };
            r.scope.start = Some(civil);
            r.scope.end = Some(civil);
        }
    }
    let mut unchanged = prior.clone();
    assert!(ReplayEngine::apply(&mut unchanged, &commit).is_err());
    assert_eq!(unchanged.records, prior.records);
    assert_eq!(unchanged.dataset_policies, prior.dataset_policies);
    assert_eq!(unchanged.through, prior.through);
    assert_eq!(unchanged.domain_digest, prior.domain_digest);
    assert_eq!(
        unchanged.accounting.as_ref().unwrap().prefix,
        prior.accounting.as_ref().unwrap().prefix
    );
    assert_eq!(
        unchanged.accounting.as_ref().unwrap().recognitions,
        prior.accounting.as_ref().unwrap().recognitions
    );
}

#[test]
fn anchor_posting_date_replay_opening_rejects_atomically() {
    reject_replayed_anchor_date(true);
}
#[test]
fn anchor_posting_date_replay_activity_rejects_atomically() {
    reject_replayed_anchor_date(false);
}

#[test]
fn anchor_posting_date_instant_conversion_is_not_guessed() {
    use currents_domain::recognition::RecognitionEngine;
    let mut state = monetary_state();
    state
        .accounting
        .as_mut()
        .unwrap()
        .date_policies
        .get_mut(&(id(103), 1))
        .unwrap()
        .conversion = DateConversionV1::Instants(InstantConversionV1::FixedOffset(FixedOffsetV1 {
        offset: "+00:00".into(),
    }));
    let mut request = supported_opening(&mut state);
    let TemporalValue::Date { civil, .. } = date("2026-11-01") else {
        panic!()
    };
    let instant = TemporalValue::Instant {
        utc: civil.with_hms(12, 0, 0).unwrap().assume_utc(),
        original_offset: Some("+00:00".into()),
        original_zone: None,
        precision: TemporalPrecision::Second,
        original_text: "synthetic noon".into(),
    };
    for p in &mut request.financial.as_mut().unwrap().postings {
        p.effective = instant.clone();
    }
    assert!(RecognitionEngine::adopt(&state, &request).is_err());
    let valid = supported_opening(&mut state);
    let plan = RecognitionEngine::adopt(&state, &valid).unwrap();
    apply(&mut state, &plan);
    let source = evidence(&mut state, 280, -96, None);
    let mut request = posted(source, 800, 30);
    request.legs[0].leg.effective = date("2026-11-02");
    for p in &mut request.financial.as_mut().unwrap().postings {
        p.effective = instant.clone();
    }
    assert!(RecognitionEngine::adopt(&state, &request).is_err());
}
