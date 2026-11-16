//! Synthetic request templates only; integration tests use a separately verified real prefix.
use super::fixture;
use currents_types::*;
pub(crate) fn retained_relationships(
    projection: &mut DomainState,
    n: u32,
    rows: Vec<RelationshipSourceRowV1>,
) -> RecordId {
    use fixture::*;
    let raw = ContentReferenceV1 {
        digest: Digest([1; 32]),
        byte_count: CommitSeq(1),
        media_type: RELATIONSHIP_SOURCE_MEDIA.into(),
    };
    let source = SourceIdentityV1 {
        source: "synthetic".into(),
        connection_id: id(900),
        device_id: None,
    };
    projection.records.insert(
        id(n),
        AuthoritativeRecordV1::ImportReceipt(ImportReceiptV1 {
            id: id(n),
            source: source.clone(),
            adapter_version: "synthetic-relationships/1".into(),
            parser_version: "1".into(),
            requested_coverage: vec![],
            reported_coverage: vec![],
            received_time: serde_json::from_str("\"2026-11-02T00:00:00Z\"").unwrap(),
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![raw.clone()],
        }),
    );
    let capture = RelationshipCaptureV1 {
        id: id(n + 1),
        schema_version: 1,
        receipt_id: id(n),
        source,
        original_capture_time: serde_json::from_str("\"2026-11-01T00:00:00Z\"").unwrap(),
        raw,
        rows,
    };
    projection.records.insert(
        capture.id,
        AuthoritativeRecordV1::RelationshipCapture(capture),
    );
    id(n + 1)
}

pub(crate) fn transfer_request() -> (DomainState, CompleteAssociationRequestV1) {
    use currents_domain::catalog::CatalogEngine;
    use fixture::*;
    let mut projection = monetary_state();
    for (n, role) in [(60, None), (70, None), (80, Some(ControlRoleV1::Clearing))] {
        let mut request = catalog();
        request.revision_id = id(n + 2);
        request.catalog.component_id = id(n + 1);
        request.catalog.account_id = AccountId(id(n).0);
        request.ownership.account_id = request.catalog.account_id;
        if let Some(role) = role {
            request.origin = AccountOriginV1::Internal(InternalAccountV1 {
                role,
                created_at: date("2026-01-01"),
            });
        }
        let plan = CatalogEngine::set_complete(&projection, &request).unwrap();
        apply(&mut projection, &plan);
        let index = projection.accounting.as_mut().unwrap();
        index.accounts.insert(
            request.catalog.account_id,
            NativeAccountMappingV1 {
                account_id: request.catalog.account_id,
                unit_id: UnitId(id(1).0),
                account_class: AccountClass::Asset,
                path: format!("Currents:A{}", request.catalog.account_id.0.simple()),
            },
        );
    }
    let rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/journeys/s2/relationship-source.json"
    ))
    .unwrap();
    let capture = retained_relationships(&mut projection, 1500, rows.clone());
    let interpretation = InterpretationV1 {
        kind: DecisionKind::Interpretation,
        authority: AuthorityKind::Owner,
        evidence: vec![capture],
        reason: "Explicit synthetic transfer and separate fee".into(),
    };
    let mut legs = vec![];
    let mut recognitions = vec![];
    let mut edges = vec![];
    for (position, row) in rows.iter().enumerate() {
        let n = u32::try_from(position).unwrap();
        let movement = amount([-1000, 600, 400, -10][position]);
        let account = AccountId(id([10, 60, 70, 10][position]).0);
        let counter = AccountId(id(if n == 3 { 30 } else { 80 }).0);
        let leg = FinancialLegV1 {
            id: LegId(id(2001 + n).0),
            event_id: id(2000),
            account_id: account,
            movement,
            allocations: vec![],
            effective: date(&row.effective),
            posting: date(&row.effective),
            relationships: vec![],
            unresolved_residual: SourceValue::Absent,
            interpretation: SourceValue::Known(interpretation.clone()),
        };
        edges.push(CapturedFacetAllocationV1 {
            source: CapturedFacetBindingV1 {
                capture,
                source_record: row.source_record.clone(),
                source_revision: row.source_revision.clone(),
                facet: row.facets[0].lineage.clone(),
            },
            leg: leg.id,
            allocated: movement,
            mode: FacetAllocationModeV1::Additive,
        });
        recognitions.push(RecognitionV1 {
            id: RecognitionId(id(4000 + n).0),
            lineage: id(4100 + n),
            postings: vec![
                PostingV1 {
                    id: id(5000 + 2 * n),
                    account_id: account,
                    amount: movement,
                    effective: leg.effective.clone(),
                    posting: leg.posting.clone(),
                },
                PostingV1 {
                    id: id(5001 + 2 * n),
                    account_id: counter,
                    amount: movement.checked_neg().unwrap(),
                    effective: leg.effective.clone(),
                    posting: leg.posting.clone(),
                },
            ],
            kind: RecognitionKind::Activity,
            evidence_allocations: vec![],
            leg_ids: vec![leg.id],
            scope: DateScope {
                start: None,
                end: None,
                basis: DateBasis::Effective,
            },
            bookkeeping_date: leg.posting.clone(),
            date_policy: PolicyReferenceV1 {
                id: id(103),
                version: 1,
            },
        });
        legs.push(leg);
    }
    let event = FinancialEventV1 {
        id: id(2000),
        legs: legs.iter().map(|l| l.id).collect(),
        relationships: vec![],
        interpretation: SourceValue::Known(interpretation.clone()),
    };
    let revision = AssociationRevisionV1 {
        id: id(3000),
        schema_version: 1,
        root: id(3000),
        prior_revision: None,
        kind: RelationshipKind::Transfer,
        legs: legs.iter().map(|l| l.id).collect(),
        allocations: vec![],
        captured_allocations: edges,
        survivor: None,
        retired_recognitions: vec![],
        recognitions: recognitions.iter().map(|r| r.id).collect(),
        residuals: vec![amount(0)],
        superseded_adoptions: vec![],
        interpretation,
        governing_policy: PolicyReferenceV1 {
            id: id(104),
            version: 1,
        },
    };
    (
        projection,
        CompleteAssociationRequestV1 {
            revision,
            events: vec![event],
            legs,
            recognitions,
        },
    )
}

pub(crate) fn payroll_request() -> (DomainState, CompleteAssociationRequestV1) {
    use currents_domain::catalog::CatalogEngine;
    use fixture::*;
    let (mut projection, mut request) = transfer_request();
    for (n, role, class, sign) in [
        (
            90,
            ControlRoleV1::IncomeFlow,
            AccountClass::Income,
            NormalSign::Credit,
        ),
        (
            120,
            ControlRoleV1::TaxPrepayment,
            AccountClass::Asset,
            NormalSign::Debit,
        ),
    ] {
        let mut catalog_request = catalog();
        catalog_request.revision_id = id(n + 2);
        catalog_request.catalog.component_id = id(n + 1);
        catalog_request.catalog.account_id = AccountId(id(n).0);
        catalog_request.ownership.account_id = catalog_request.catalog.account_id;
        catalog_request.catalog.account_class = class;
        catalog_request.catalog.normal_sign = sign;
        catalog_request.origin = AccountOriginV1::Internal(InternalAccountV1 {
            role,
            created_at: date("2026-01-01"),
        });
        let plan = CatalogEngine::set_complete(&projection, &catalog_request).unwrap();
        apply(&mut projection, &plan);
        projection.accounting.as_mut().unwrap().accounts.insert(
            catalog_request.catalog.account_id,
            NativeAccountMappingV1 {
                account_id: catalog_request.catalog.account_id,
                unit_id: UnitId(id(1).0),
                account_class: class,
                path: format!(
                    "Currents:A{}",
                    catalog_request.catalog.account_id.0.simple()
                ),
            },
        );
    }
    let rows: Vec<RelationshipSourceRowV1> = serde_json::from_slice(include_bytes!(
        "../../../../fixtures/journeys/s3/relationship-source.json"
    ))
    .unwrap();
    let capture = retained_relationships(&mut projection, 1600, rows.clone());
    request.revision.kind = RelationshipKind::Payroll;
    request.revision.interpretation.evidence = vec![capture];
    request.revision.interpretation.reason =
        "Explicit synthetic gross payroll and chosen tax expense".into();
    let leg_template = request.legs[0].clone();
    let recognition_template = request.recognitions[0].clone();
    request.legs.clear();
    request.recognitions.clear();
    request.revision.captured_allocations.clear();
    for (position, facet) in rows[0].facets.iter().enumerate() {
        let n = u32::try_from(position).unwrap();
        let movement = amount([-5000, 1000, 500, 200, 3300][position]);
        let mut leg = leg_template.clone();
        leg.id = LegId(id(2001 + n).0);
        leg.account_id = AccountId(id([90, 30, 60, 70, 10][position]).0);
        leg.movement = movement;
        leg.interpretation = SourceValue::Known(request.revision.interpretation.clone());
        let mut recognition = recognition_template.clone();
        recognition.id = RecognitionId(id(4000 + n).0);
        recognition.lineage = id(4100 + n);
        recognition.leg_ids = vec![leg.id];
        recognition.postings[0].id = id(5000 + 2 * n);
        recognition.postings[0].account_id = leg.account_id;
        recognition.postings[0].amount = movement;
        recognition.postings[1].id = id(5001 + 2 * n);
        recognition.postings[1].account_id = AccountId(id(80).0);
        recognition.postings[1].amount = movement.checked_neg().unwrap();
        request
            .revision
            .captured_allocations
            .push(CapturedFacetAllocationV1 {
                source: CapturedFacetBindingV1 {
                    capture,
                    source_record: rows[0].source_record.clone(),
                    source_revision: rows[0].source_revision.clone(),
                    facet: facet.lineage.clone(),
                },
                leg: leg.id,
                allocated: movement,
                mode: FacetAllocationModeV1::Additive,
            });
        request.legs.push(leg);
        request.recognitions.push(recognition);
    }
    request.revision.legs = request.legs.iter().map(|l| l.id).collect();
    request.revision.recognitions = request.recognitions.iter().map(|r| r.id).collect();
    request.events[0].legs = request.revision.legs.clone();
    request.events[0].interpretation = SourceValue::Known(request.revision.interpretation.clone());
    (projection, request)
}

pub(crate) fn revised_transfer(
    original: &CompleteAssociationRequestV1,
    capture: RecordId,
) -> CompleteAssociationRequestV1 {
    let mut next = original.clone();
    next.revision.id = fixture::id(7000);
    next.revision.prior_revision = Some(original.revision.id);
    next.revision.retired_recognitions = original.revision.recognitions.clone();
    next.revision.interpretation.evidence = vec![capture];
    next.events[0].id = fixture::id(7100);
    next.events[0].interpretation = SourceValue::Known(next.revision.interpretation.clone());
    for (i, leg) in next.legs.iter_mut().enumerate() {
        let n = u32::try_from(i).unwrap();
        leg.id = LegId(fixture::id(7200 + n).0);
        leg.event_id = next.events[0].id;
        leg.movement = fixture::amount([-1100, 700, 400, -10][i]);
        leg.interpretation = SourceValue::Known(next.revision.interpretation.clone());
        let r = &mut next.recognitions[i];
        r.id = RecognitionId(fixture::id(7300 + n).0);
        r.lineage = fixture::id(7400 + n);
        r.leg_ids = vec![leg.id];
        r.postings[0].id = fixture::id(7500 + 2 * n);
        r.postings[1].id = fixture::id(7501 + 2 * n);
        r.postings[0].amount = leg.movement;
        r.postings[1].amount = leg.movement.checked_neg().unwrap();
        let edge = &mut next.revision.captured_allocations[i];
        edge.leg = leg.id;
        edge.allocated = leg.movement;
        edge.source.capture = capture;
        edge.source.source_revision = "revised".into();
    }
    next.revision.legs = next.legs.iter().map(|l| l.id).collect();
    next.events[0].legs = next.revision.legs.clone();
    next.revision.recognitions = next.recognitions.iter().map(|r| r.id).collect();
    next
}
