//! CUR015 association behavior, using synthetic identities and exact public interfaces.
use currents_domain::catalog::CatalogEngine;
use currents_types::*;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;
#[path = "support/associations.rs"]
mod association_fixtures;
use association_fixtures::*;

#[test]
fn explicit_clearing_and_prepayment_are_declared_assets_not_external_openings() {
    for role in ["Clearing", "TaxPrepayment"] {
        let state = state();
        let mut wire = serde_json::to_value(catalog()).unwrap();
        wire["origin"] = serde_json::json!({"kind":"Internal","value":{
            "role":role,"created_at":date("2026-11-01")
        }});
        let mut request: CatalogRequestV1 = serde_json::from_value(wire).unwrap();
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        let AccountingIntent::MetadataDeclaration(declaration) = plan.accounting else {
            panic!("creation is metadata, not a financial recognition")
        };
        assert_eq!(declaration.accounts.len(), 1);
        assert_eq!(declaration.accounts[0].account_class, AccountClass::Asset);
        assert_eq!(
            declaration.accounts[0].opening_boundary,
            SourceValue::Known(date("2026-11-01"))
        );
        assert!(plan.records.iter().all(|record| !matches!(
            record,
            AuthoritativeRecordV1::Recognition(_) | AuthoritativeRecordV1::AccountingSegment(_)
        )));
        request.catalog.account_class = AccountClass::Expense;
        assert!(CatalogEngine::set_complete(&state, &request).is_err());
        request.catalog.account_class = AccountClass::Asset;
        request.origin = AccountOriginV1::External;
        let external = CatalogEngine::set_complete(&state, &request).unwrap();
        let AccountingIntent::MetadataDeclaration(declaration) = external.accounting else {
            panic!()
        };
        assert_eq!(
            declaration.accounts[0].opening_boundary,
            SourceValue::Unknown
        );
    }
}

#[test]
fn cur_015_4_explicit_owner_expiry_is_nonmonetary_and_absence_is_not_expiry() {
    use currents_domain::recognition::RecognitionEngine;
    use fixture::*;
    let mut state = activity_state();
    let source = evidence(&mut state, 200, 80, None);
    let request = pending(source, 300);
    let plan = RecognitionEngine::adopt(&state, &request).unwrap();
    apply(&mut state, &plan);
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
    state.through = CommitSeq(100_000);
    assert_eq!(RecognitionEngine::pending(&state).unwrap().len(), 1);
    let expiry: AuthoritativeRecordV1 = serde_json::from_value(serde_json::json!({
        "kind":"PendingExpiry", "value":{
            "id":id(400), "schema_version":1, "pending_adoption":id(300),
            "interpretation":interpretation(), "governing_policy":request.governing_policy
        }
    }))
    .unwrap();
    let AuthoritativeRecordV1::PendingExpiry(expiry_value) = &expiry else {
        panic!("expiry required")
    };
    let expiration =
        currents_domain::expiry::PendingExpiryEngine::expire(&state, expiry_value).unwrap();
    assert!(matches!(
        expiration.accounting,
        AccountingIntent::NoEffect(_)
    ));
    assert_eq!(expiration.records, vec![expiry.clone()]);
    for case in 0..4 {
        let mut bad = expiry_value.clone();
        match case {
            0 => bad.schema_version = 2,
            1 => bad.interpretation.reason.clear(),
            2 => bad.governing_policy.version = 999,
            _ => bad.interpretation.evidence.push(id(999)),
        }
        assert!(currents_domain::expiry::PendingExpiryEngine::expire(&state, &bad).is_err());
    }
    state.records.insert(id(400), expiry.clone());
    assert!(currents_domain::expiry::PendingExpiryEngine::expire(&state, expiry_value).is_err());
    assert!(RecognitionEngine::pending(&state).unwrap().is_empty());
    assert!(
        state.records.contains_key(&id(300)),
        "expiry cannot delete historical adoption"
    );
    assert!(
        !state
            .records
            .values()
            .any(|r| matches!(r, AuthoritativeRecordV1::Recognition(_)))
    );
    let mut bad = serde_json::to_value(expiry).unwrap();
    bad["value"]["pending_adoption"] = serde_json::to_value(id(999)).unwrap();
    state
        .records
        .insert(id(400), serde_json::from_value(bad).unwrap());
    assert!(
        RecognitionEngine::pending(&state).is_err(),
        "projection cannot accept an unrelated expiry"
    );
}

#[test]
fn quantified_source_capacity_is_not_multiplied_by_many_to_many_edges() {
    use currents_domain::associations::IdentityEngine;
    use fixture::*;
    let mut state = activity_state();
    let source = evidence(&mut state, 200, 1000, None);
    let mut first = pending(source.clone(), 300).legs[0].leg.clone();
    first.movement = amount(600);
    let mut second = first.clone();
    second.id = LegId(id(999).0);
    second.movement = amount(400);
    let edges = vec![
        QuantifiedFacetAllocationV1 {
            source: source.clone(),
            leg: first.id,
            allocated: amount(600),
            mode: FacetAllocationModeV1::Additive,
        },
        QuantifiedFacetAllocationV1 {
            source: source.clone(),
            leg: second.id,
            allocated: amount(400),
            mode: FacetAllocationModeV1::Additive,
        },
    ];
    let legs = vec![first.clone(), second.clone()];
    let residuals = IdentityEngine::quantities(&state, &legs, &edges).unwrap();
    assert_eq!(residuals.len(), 1);
    assert_eq!(residuals[0].remaining, amount(0));
    let partial = IdentityEngine::quantities(&state, &[first.clone()], &edges[..1]).unwrap();
    assert_eq!(partial[0].remaining, amount(400));
    let mut overlap = edges.clone();
    overlap[1].allocated = amount(500);
    second.movement = amount(500);
    assert!(IdentityEngine::quantities(&state, &[first, second], &overlap).is_err());
    let mut mismatch = edges.clone();
    mismatch[0].allocated = amount(-600);
    assert!(IdentityEngine::quantities(&state, &legs, &mismatch).is_err());
    let mut unknown = edges;
    unknown[0].source.parse_product = id(777);
    assert!(IdentityEngine::quantities(&state, &legs, &unknown).is_err());
}

#[test]
fn corroborating_evidence_does_not_double_the_economic_leg() {
    use currents_domain::associations::IdentityEngine;
    use fixture::*;
    let mut state = activity_state();
    let a = evidence(&mut state, 200, 1000, None);
    let b = evidence(&mut state, 210, 1000, None);
    let leg = pending(a.clone(), 300).legs[0].leg.clone();
    let edges = vec![
        QuantifiedFacetAllocationV1 {
            source: a,
            leg: leg.id,
            allocated: amount(1000),
            mode: FacetAllocationModeV1::Corroborating,
        },
        QuantifiedFacetAllocationV1 {
            source: b,
            leg: leg.id,
            allocated: amount(1000),
            mode: FacetAllocationModeV1::Corroborating,
        },
    ];
    let residuals = IdentityEngine::quantities(&state, std::slice::from_ref(&leg), &edges).unwrap();
    assert_eq!(residuals.len(), 2);
    assert!(residuals.iter().all(|r| r.remaining == amount(0)));
    let mut duplicated = edges.clone();
    duplicated.push(edges[0].clone());
    assert!(IdentityEngine::quantities(&state, &[leg], &duplicated).is_err());
}

#[test]
fn association_revision_prefixes_are_immutable_and_forks_are_rejected() {
    use currents_domain::associations::IdentityEngine;
    use fixture::*;
    let first = AssociationRevisionV1 {
        id: id(600),
        schema_version: 1,
        root: id(600),
        prior_revision: None,
        kind: RelationshipKind::Transfer,
        legs: vec![LegId(id(301).0)],
        allocations: vec![],
        captured_allocations: vec![],
        recognitions: vec![],
        residuals: vec![],
        superseded_adoptions: vec![],
        survivor: None,
        retired_recognitions: vec![],
        interpretation: interpretation(),
        governing_policy: PolicyReferenceV1 {
            id: id(110),
            version: 1,
        },
    };
    let original_bytes = serde_json::to_vec(&first).unwrap();
    let mut second = first.clone();
    second.id = id(601);
    second.prior_revision = Some(first.id);
    second.legs = vec![LegId(id(401).0)];
    assert_eq!(
        IdentityEngine::revision_chain(std::slice::from_ref(&first), first.root).unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        IdentityEngine::revision_chain(&[second.clone(), first.clone()], first.root).unwrap(),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(serde_json::to_vec(&first).unwrap(), original_bytes);
    let mut fork = second.clone();
    fork.id = id(602);
    assert!(
        IdentityEngine::revision_chain(&[first.clone(), second.clone(), fork], first.root).is_err()
    );
    for case in 0..5 {
        let mut bad = second.clone();
        match case {
            0 => bad.prior_revision = Some(bad.id),
            1 => bad.root = id(999),
            2 => bad.kind = RelationshipKind::Payroll,
            3 => bad.schema_version = 2,
            _ => bad.prior_revision = Some(id(999)),
        }
        assert!(
            IdentityEngine::revision_chain(&[first.clone(), bad], first.root).is_err(),
            "case {case}"
        );
    }
    let mut disconnected_a = second.clone();
    disconnected_a.prior_revision = Some(id(602));
    let mut disconnected_b = second;
    disconnected_b.id = id(602);
    disconnected_b.prior_revision = Some(disconnected_a.id);
    assert!(
        IdentityEngine::revision_chain(
            &[first.clone(), disconnected_a, disconnected_b],
            first.root
        )
        .is_err()
    );
    assert!(IdentityEngine::revision_chain(&[first.clone(), first.clone()], first.root).is_err());
    assert!(IdentityEngine::revision_chain(&[], first.root).is_err());
}

#[test]
fn captured_facets_have_exact_typed_identity_and_explicit_revision_order() {
    use currents_domain::associations::IdentityEngine;
    use fixture::*;
    let rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(include_bytes!(
        "../../../fixtures/journeys/s3/relationship-source.json"
    ))
    .unwrap();
    let mut state = state();
    let capture = retained_relationships(&mut state, 1000, rows.clone());
    let binding = CapturedFacetBindingV1 {
        capture,
        source_record: "paystub-1".into(),
        source_revision: "original".into(),
        facet: "gross".into(),
    };
    let first = IdentityEngine::captured_facet(&state, &binding, UnitId(id(1).0)).unwrap();
    assert_eq!(first.amount, amount(5000));
    assert_eq!(first.role, RelationshipFacetRoleV1::GrossPay);
    let mut missing = binding.clone();
    missing.facet = "guess".into();
    assert!(IdentityEngine::captured_facet(&state, &missing, UnitId(id(1).0)).is_err());
    let mut revised = rows.clone();
    revised[0].source_revision = "revised".into();
    revised[0].prior_revision = Some("original".into());
    revised[0].facets[0].amount = "5200".into();
    let next_capture = retained_relationships(&mut state, 1010, revised);
    let next = CapturedFacetBindingV1 {
        capture: next_capture,
        source_revision: "revised".into(),
        ..binding.clone()
    };
    let current = IdentityEngine::captured_facet(&state, &next, UnitId(id(1).0)).unwrap();
    assert_eq!(current.amount, amount(5200));
    assert_eq!(current.fact_key, first.fact_key);
    assert!(IdentityEngine::captured_facet(&state, &binding, UnitId(id(1).0)).is_err());
    let mut ambiguous = rows;
    ambiguous[0].source_revision = "unordered".into();
    retained_relationships(&mut state, 1020, ambiguous);
    assert!(IdentityEngine::captured_facet(&state, &next, UnitId(id(1).0)).is_err());
    let historic = IdentityEngine::retained_facet(&state, &binding, UnitId(id(1).0)).unwrap();
    assert_eq!(
        historic, first,
        "later evidence cannot rewrite the previously captured basis"
    );
}

#[test]
fn cur_015_1_complete_transfer_preserves_dates_and_separate_fee() {
    use currents_domain::associations::IdentityEngine;
    let (state, request) = transfer_request();
    let plan = IdentityEngine::associate(&state, &request).unwrap();
    let AccountingIntent::Recognitions(entries) = plan.accounting else {
        panic!("initial atomic entries required")
    };
    assert_eq!(entries.len(), 4);
    assert_eq!(
        entries[0].postings[0].effective,
        fixture::date("2026-11-01")
    );
    assert_eq!(
        entries[1].postings[0].effective,
        fixture::date("2026-11-03")
    );
    assert_eq!(
        entries[2].postings[0].effective,
        fixture::date("2026-11-05")
    );
    for case in 0..4 {
        let mut bad = request.clone();
        match case {
            0 => bad.recognitions[0].postings[1].account_id = AccountId(fixture::id(30).0),
            1 => bad
                .revision
                .captured_allocations
                .push(bad.revision.captured_allocations[0].clone()),
            2 => bad.legs[1].effective = fixture::date("2026-11-01"),
            _ => bad.revision.residuals = vec![fixture::amount(1)],
        }
        assert!(
            IdentityEngine::associate(&state, &bad).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn transfer_source_arrival_permutations_have_identical_financial_plan() {
    use currents_domain::associations::IdentityEngine;
    let (state, request) = transfer_request();
    let expected = IdentityEngine::associate(&state, &request).unwrap();
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut alternative = state.clone();
        let Some(AuthoritativeRecordV1::RelationshipCapture(capture)) =
            alternative.records.get_mut(&fixture::id(1501))
        else {
            panic!()
        };
        let original = capture.rows.clone();
        capture.rows = order
            .into_iter()
            .map(|n| original[n].clone())
            .chain(std::iter::once(original[3].clone()))
            .collect();
        assert_eq!(
            IdentityEngine::associate(&alternative, &request).unwrap(),
            expected
        );
    }
}

#[test]
fn cur_015_2_payroll_requires_distinct_facets_and_explicit_withholding_choice() {
    use currents_domain::associations::IdentityEngine;
    let (state, request) = payroll_request();
    let plan = IdentityEngine::associate(&state, &request).unwrap();
    let AccountingIntent::Recognitions(entries) = plan.accounting else {
        panic!()
    };
    assert_eq!(entries.len(), 5);
    assert_eq!(
        entries
            .iter()
            .map(|r| r.postings[0].amount.coefficient)
            .collect::<Vec<_>>(),
        vec![-5000, 1000, 500, 200, 3300]
    );
    let mut prepayment = request.clone();
    prepayment.legs[1].account_id = AccountId(fixture::id(120).0);
    prepayment.recognitions[1].postings[0].account_id = prepayment.legs[1].account_id;
    assert!(IdentityEngine::associate(&state, &prepayment).is_ok());
    let mut unresolved = request.clone();
    unresolved.legs[1].account_id = AccountId(fixture::id(40).0);
    unresolved.recognitions[1].postings[0].account_id = unresolved.legs[1].account_id;
    assert!(IdentityEngine::associate(&state, &unresolved).is_err());
    let mut net = request.clone();
    net.legs = vec![request.legs[4].clone()];
    net.recognitions = vec![request.recognitions[4].clone()];
    net.revision.legs = vec![net.legs[0].id];
    net.revision.recognitions = vec![net.recognitions[0].id];
    net.revision.captured_allocations = vec![request.revision.captured_allocations[4].clone()];
    net.revision.residuals = vec![fixture::amount(3300)];
    net.events[0].legs = net.revision.legs.clone();
    assert!(IdentityEngine::associate(&state, &net).is_err());
}

#[test]
fn complete_association_replay_rejects_balanced_wrong_role_atomically() {
    let (state, request) = transfer_request();
    let plan = currents_domain::associations::IdentityEngine::associate(&state, &request).unwrap();
    let seq = CommitSeq(state.through.0 + 1);
    let commit = DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records: plan.records,
        decisions: vec![],
        native_head_after: state
            .accounting
            .as_ref()
            .unwrap()
            .prefix
            .native_head
            .clone(),
        digest: Digest([8; 32]),
        command_result: CommandResultV1 {
            command_id: CommandId(fixture::id(9999).0),
            digest: Digest([9; 32]),
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
    let mut positive = state.clone();
    currents_domain::ReplayEngine::apply(&mut positive, &commit).unwrap();
    let mut bad = commit.clone();
    let Some(AuthoritativeRecordV1::Recognition(recognition)) = bad
        .records
        .iter_mut()
        .find(|r| matches!(r, AuthoritativeRecordV1::Recognition(_)))
    else {
        panic!()
    };
    recognition.postings[1].account_id = AccountId(fixture::id(30).0);
    let mut rejected = state.clone();
    assert!(currents_domain::ReplayEngine::apply(&mut rejected, &bad).is_err());
    assert_eq!(rejected.through, state.through);
    assert_eq!(rejected.records, state.records);
}

fn retain_association(projection: &mut DomainState, request: &CompleteAssociationRequestV1) {
    let plan =
        currents_domain::associations::IdentityEngine::associate(projection, request).unwrap();
    for record in plan.records {
        let key = match &record {
            AuthoritativeRecordV1::FinancialEvent(v) => v.id,
            AuthoritativeRecordV1::FinancialLeg(v) => RecordId(v.id.0),
            AuthoritativeRecordV1::Recognition(v) => RecordId(v.id.0),
            AuthoritativeRecordV1::AssociationRevision(v) => v.id,
            _ => panic!("unexpected association record"),
        };
        projection.records.insert(key, record);
    }
    let index = projection.accounting.as_mut().unwrap();
    for id in &request.revision.retired_recognitions {
        index.reversed.insert(*id);
    }
    cache_recognitions(projection, &request.recognitions);
}
fn cache_recognitions(projection: &mut DomainState, recognitions: &[RecognitionV1]) {
    let index = projection.accounting.as_mut().unwrap();
    for recognition in recognitions {
        // Domain fixture models a previously verified frozen prefix. Native
        // operation correspondence is separately exercised by storage tests.
        index.recognitions.insert(
            recognition.id,
            vec![FrozenEntryV1 {
                recognition_id: recognition.id,
                key: format!("fixture/{}", recognition.id.0),
                booking_date: "2026-11-01".into(),
                value_date: "2026-11-01".into(),
                postings: recognition
                    .postings
                    .iter()
                    .map(|p| FrozenPostingV1 {
                        original: p.clone(),
                        native_coefficient: p.amount.coefficient.to_string(),
                        path: index.accounts[&p.account_id].path.clone(),
                        unit: index.units[&p.amount.unit_id].clone(),
                    })
                    .collect(),
                date_policy: index.date_policies
                    [&(recognition.date_policy.id, recognition.date_policy.version)]
                    .clone(),
                generated_document: Digest([0; 32]),
                reverses: None,
            }],
        );
    }
}

#[test]
fn cur_015_3_source_revision_replaces_all_old_claims_without_mutating_history() {
    use currents_domain::associations::IdentityEngine;
    let (mut state, original) = transfer_request();
    retain_association(&mut state, &original);
    let old_records = state.records.clone();
    let mut rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(include_bytes!(
        "../../../fixtures/journeys/s2/relationship-source.json"
    ))
    .unwrap();
    for (i, row) in rows.iter_mut().enumerate() {
        row.source_revision = "revised".into();
        row.prior_revision = Some("original".into());
        row.facets[0].amount = ["-1100", "700", "400", "-10"][i].into();
    }
    let capture = retained_relationships(&mut state, 1700, rows);
    let next = revised_transfer(&original, capture);
    let plan = IdentityEngine::associate(&state, &next).unwrap();
    let AccountingIntent::Corrections(corrections) = plan.accounting else {
        panic!("remap must reverse old native authority atomically")
    };
    assert_eq!(
        corrections
            .iter()
            .map(|c| c.active_recognition)
            .collect::<Vec<_>>(),
        original.revision.recognitions
    );
    assert_eq!(
        corrections
            .iter()
            .flat_map(|c| c.replacement.clone())
            .collect::<Vec<_>>(),
        next.recognitions
    );
    for (key, record) in old_records {
        assert_eq!(state.records.get(&key), Some(&record));
    }
    let mut accepted = state.clone();
    retain_association(&mut accepted, &next);
    IdentityEngine::audit(&accepted).unwrap();
    accepted
        .accounting
        .as_mut()
        .unwrap()
        .reversed
        .remove(&original.recognitions[0].id);
    assert!(
        IdentityEngine::audit(&accepted).is_err(),
        "historical declaration without native retirement cannot be healthy"
    );
    let mut missing = next.clone();
    missing.revision.retired_recognitions.pop();
    assert!(IdentityEngine::associate(&state, &missing).is_err());
    state
        .accounting
        .as_mut()
        .unwrap()
        .reversed
        .insert(original.recognitions[0].id);
    assert!(IdentityEngine::associate(&state, &next).is_err());
}

#[test]
fn cur_015_4_source_tombstone_retains_history_and_never_reverses_money() {
    use currents_domain::associations::IdentityEngine;
    let (mut projection, request) = transfer_request();
    retain_association(&mut projection, &request);
    let before = projection.accounting.as_ref().unwrap().clone();
    let old = projection.records.clone();
    let mut rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(include_bytes!(
        "../../../fixtures/journeys/s2/relationship-source.json"
    ))
    .unwrap();
    for row in &mut rows {
        row.source_revision = "deleted".into();
        row.prior_revision = Some("original".into());
        row.status = RelationshipSourceStatusV1::Tombstone;
        row.facets.clear();
    }
    retained_relationships(&mut projection, 1800, rows);
    IdentityEngine::audit(&projection).unwrap();
    assert_eq!(
        projection.accounting.as_ref().unwrap().recognitions,
        before.recognitions
    );
    assert_eq!(
        projection.accounting.as_ref().unwrap().reversed,
        before.reversed
    );
    for (id, record) in old {
        assert_eq!(projection.records.get(&id), Some(&record));
    }
    for edge in &request.revision.captured_allocations {
        assert!(
            IdentityEngine::captured_facet(&projection, &edge.source, edge.allocated.unit_id)
                .is_err(),
            "deleted source cannot authorize fresh recognition"
        );
        assert!(
            IdentityEngine::retained_facet(&projection, &edge.source, edge.allocated.unit_id)
                .is_ok(),
            "old evidence is not deleted"
        );
    }
}

#[test]
fn explicit_n_way_duplicates_keep_chosen_survivor_and_reverse_every_redundant_original() {
    use currents_domain::{associations::IdentityEngine, recognition::RecognitionEngine};
    let mut projection = monetary_state();
    let mut requests = Vec::new();
    let mut bindings = Vec::new();
    for n in [200, 220, 240] {
        let source = evidence(&mut projection, n, -96, None);
        let request = posted(source.clone(), n + 100, 30);
        let plan = RecognitionEngine::adopt(&projection, &request).unwrap();
        let AccountingIntent::Recognition(recognition) = &plan.accounting else {
            panic!()
        };
        apply(&mut projection, &plan);
        cache_recognitions(&mut projection, std::slice::from_ref(recognition));
        requests.push(request);
        bindings.push(source);
    }
    let survivor = requests[1].event.legs[0];
    let revision = AssociationRevisionV1 {
        id: id(9000),
        schema_version: 1,
        root: id(9000),
        prior_revision: None,
        kind: RelationshipKind::Duplicate,
        legs: requests.iter().map(|r| r.event.legs[0]).collect(),
        allocations: bindings
            .into_iter()
            .map(|source| QuantifiedFacetAllocationV1 {
                source,
                leg: survivor,
                allocated: amount(-96),
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
        interpretation: interpretation(),
        governing_policy: requests[0].governing_policy.clone(),
    };
    let request = CompleteAssociationRequestV1 {
        revision,
        events: vec![],
        legs: vec![],
        recognitions: vec![],
    };
    let plan = IdentityEngine::associate(&projection, &request).unwrap();
    assert_eq!(
        plan.records,
        vec![AuthoritativeRecordV1::AssociationRevision(
            request.revision.clone()
        )]
    );
    let seq = CommitSeq(projection.through.0 + 1);
    let commit = DomainCommitV1 {
        seq,
        previous_digest: projection.domain_digest,
        records: plan.records.clone(),
        decisions: vec![],
        native_head_after: projection
            .accounting
            .as_ref()
            .unwrap()
            .prefix
            .native_head
            .clone(),
        digest: Digest([42; 32]),
        command_result: CommandResultV1 {
            command_id: CommandId(id(9999).0),
            digest: Digest([1; 32]),
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
    let mut replayed = projection.clone();
    currents_domain::ReplayEngine::apply(&mut replayed, &commit).unwrap();
    for id in &request.revision.retired_recognitions {
        replayed.accounting.as_mut().unwrap().reversed.insert(*id);
    }
    replayed.accounting.as_mut().unwrap().prefix.through = replayed.through;
    replayed.accounting.as_mut().unwrap().prefix.domain_digest = replayed.domain_digest;
    IdentityEngine::audit(&replayed).unwrap();
    RecognitionEngine::audit(&replayed).unwrap(); // old adoptions still fully audited, never skipped
    let mut corrupt = commit;
    let AuthoritativeRecordV1::AssociationRevision(r) = &mut corrupt.records[0] else {
        panic!()
    };
    r.retired_recognitions.pop();
    let mut untouched = projection.clone();
    assert!(currents_domain::ReplayEngine::apply(&mut untouched, &corrupt).is_err());
    assert_eq!(untouched.records, projection.records);
    let AccountingIntent::Corrections(corrections) = plan.accounting else {
        panic!()
    };
    assert_eq!(corrections.len(), 2);
    assert!(corrections.iter().all(|c| c.replacement.is_empty()));
    assert!(
        corrections
            .iter()
            .all(|c| c.active_recognition != request.revision.recognitions[0])
    );
    let mut implicit = request.clone();
    implicit.revision.survivor = None;
    assert!(IdentityEngine::associate(&projection, &implicit).is_err());
    let mut partial = request.clone();
    partial.revision.retired_recognitions.pop();
    assert!(IdentityEngine::associate(&projection, &partial).is_err());
    let mut self_reversal = request;
    self_reversal.revision.retired_recognitions[0] = self_reversal.revision.recognitions[0];
    assert!(IdentityEngine::associate(&projection, &self_reversal).is_err());
}

#[test]
fn cur_015_4_reconnect_is_an_explicit_stable_account_alias_not_new_money() {
    let mut projection = activity_state();
    let mut revision = catalog();
    revision.revision_id = id(8000);
    revision.prior_revision = Some(id(12));
    let alias = SourceAliasV1 {
        source: SourceIdentityV1 {
            source: "synthetic".into(),
            connection_id: id(8001),
            device_id: None,
        },
        original_id: "old-provider-account".into(),
    };
    revision.catalog.aliases = vec![alias.clone()];
    let first = CatalogEngine::set_complete(&projection, &revision).unwrap();
    assert!(matches!(first.accounting, AccountingIntent::NoEffect(_)));
    apply(&mut projection, &first);
    let before = projection.records.clone();
    let reconnected = SourceAliasV1 {
        source: SourceIdentityV1 {
            connection_id: id(8002),
            ..alias.source.clone()
        },
        original_id: "new-provider-account".into(),
    };
    assert!(
        !CatalogEngine::current(&projection, id(11))
            .unwrap()
            .catalog
            .aliases
            .contains(&reconnected)
    );
    revision.prior_revision = Some(revision.revision_id);
    revision.revision_id = id(8003);
    revision.catalog.aliases.push(reconnected.clone());
    let plan = CatalogEngine::set_complete(&projection, &revision).unwrap();
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    apply(&mut projection, &plan);
    let current = CatalogEngine::current(&projection, id(11)).unwrap();
    assert_eq!(current.account_id, AccountId(id(10).0));
    assert_eq!(current.catalog.aliases, vec![alias, reconnected]);
    for (key, record) in before {
        assert_eq!(projection.records.get(&key), Some(&record));
    }
    assert!(!plan.records.iter().any(|r| matches!(
        r,
        AuthoritativeRecordV1::Recognition(_) | AuthoritativeRecordV1::AccountComponent(_)
    )));
    let mut inferred = revision;
    inferred.prior_revision = None;
    inferred.revision_id = id(8004);
    assert!(CatalogEngine::set_complete(&projection, &inferred).is_err());
}
