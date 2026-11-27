//! Synthetic transfer families recognized against a clearing account.
use super::fixture;
use currents_types::*;
/// Retained transfer rows: account 10 sends 1,000 USD to account 60 on
/// 2026-10-31, received on 2026-11-02, then sends 200 USD on 2026-11-04,
/// received on 2026-11-05.
pub(crate) fn cross_account_rows() -> Vec<RelationshipSourceRowV1> {
    let row = |record: &str, day: &str, group: &str, value: &str| {
        serde_json::json!({"source_record":record,"source_revision":"original",
            "prior_revision":null,"status":"Posted","effective":day,"facets":[
            {"lineage":"principal","role":"Movement","group":group,"amount":value,"unit":"USD"}]})
    };
    serde_json::from_value(serde_json::json!([
        row("send-a", "2026-10-31", "transfer-1", "-1000"),
        row("receipt-c", "2026-11-02", "transfer-1", "1000"),
        row("later-send-a", "2026-11-04", "transfer-2", "-200"),
        row("later-receipt-c", "2026-11-05", "transfer-2", "200"),
    ]))
    .unwrap()
}
/// A transfer family from `base`: one leg per `(account, row)`, each
/// recognized against clearing account 80 on its row's date.
pub(crate) fn clearing_family(
    capture: RecordId,
    rows: &[RelationshipSourceRowV1],
    base: u32,
    legs: &[(u32, usize)],
) -> CompleteAssociationRequestV1 {
    use fixture::*;
    let interpretation = InterpretationV1 {
        kind: DecisionKind::Interpretation,
        authority: AuthorityKind::Owner,
        evidence: vec![capture],
        reason: "Explicit synthetic transfer".into(),
    };
    let mut request = CompleteAssociationRequestV1 {
        revision: AssociationRevisionV1 {
            id: id(base + 50),
            schema_version: 1,
            root: id(base + 50),
            prior_revision: None,
            kind: RelationshipKind::Transfer,
            legs: vec![],
            allocations: vec![],
            captured_allocations: vec![],
            recognitions: vec![],
            residuals: vec![],
            superseded_adoptions: vec![],
            survivor: None,
            retired_recognitions: vec![],
            interpretation: interpretation.clone(),
            governing_policy: PolicyReferenceV1 {
                id: id(104),
                version: 1,
            },
        },
        events: vec![],
        legs: vec![],
        recognitions: vec![],
    };
    let mut total = amount(0);
    for (i, &(on, row)) in (0u32..).zip(legs) {
        let row = &rows[row];
        let movement = amount(row.facets[0].amount.parse().unwrap());
        let day = date(&row.effective);
        let leg = FinancialLegV1 {
            id: LegId(id(base + 1 + i).0),
            event_id: id(base),
            account_id: AccountId(id(on).0),
            movement,
            allocations: vec![],
            effective: day.clone(),
            posting: day.clone(),
            relationships: vec![],
            unresolved_residual: SourceValue::Absent,
            interpretation: SourceValue::Known(interpretation.clone()),
        };
        let posting = |n: u32, on: u32, amount: Amount| PostingV1 {
            id: id(n),
            account_id: AccountId(id(on).0),
            amount,
            effective: day.clone(),
            posting: day.clone(),
        };
        let revision = &mut request.revision;
        revision.legs.push(leg.id);
        revision
            .recognitions
            .push(RecognitionId(id(base + 10 + i).0));
        revision
            .captured_allocations
            .push(CapturedFacetAllocationV1 {
                source: CapturedFacetBindingV1 {
                    capture,
                    source_record: row.source_record.clone(),
                    source_revision: row.source_revision.clone(),
                    facet: row.facets[0].lineage.clone(),
                    parse_revision: None,
                },
                leg: leg.id,
                allocated: movement,
                mode: FacetAllocationModeV1::Additive,
            });
        request.recognitions.push(RecognitionV1 {
            id: RecognitionId(id(base + 10 + i).0),
            lineage: id(base + 20 + i),
            postings: vec![
                posting(base + 30 + 2 * i, on, movement),
                posting(base + 31 + 2 * i, 80, movement.checked_neg().unwrap()),
            ],
            kind: RecognitionKind::Activity,
            evidence_allocations: vec![],
            leg_ids: vec![leg.id],
            scope: DateScope {
                start: None,
                end: None,
                basis: DateBasis::Effective,
            },
            bookkeeping_date: day.clone(),
            date_policy: PolicyReferenceV1 {
                id: id(103),
                version: 1,
            },
        });
        total = total.checked_add(&movement).unwrap();
        request.legs.push(leg);
    }
    request.revision.residuals = vec![total.normalized().unwrap()];
    request.events = vec![FinancialEventV1 {
        id: id(base),
        legs: request.revision.legs.clone(),
        relationships: vec![],
        interpretation: SourceValue::Known(interpretation),
    }];
    request
}
/// The captured fact for row `record` in `capture`.
pub(crate) fn row_fact(capture: RecordId, record: &str) -> CapturedFacetBindingV1 {
    CapturedFacetBindingV1 {
        capture,
        source_record: record.into(),
        source_revision: "original".into(),
        facet: "principal".into(),
        parse_revision: None,
    }
}
/// Recognition `original` again with fresh identities from `base`: record
/// `base`, recognition `base + 10` and postings from `base + 11`.
pub(crate) fn reemission(
    state: &DomainState,
    original: u32,
    base: u32,
) -> currents_domain::segments::ReemissionV1 {
    use fixture::id;
    let Some(AuthoritativeRecordV1::Recognition(r)) = state.records.get(&id(original)) else {
        panic!("recognition {original} missing")
    };
    let mut replacement = r.clone();
    replacement.id = RecognitionId(id(base + 10).0);
    for (posting, n) in replacement.postings.iter_mut().zip(base + 11..) {
        posting.id = id(n);
    }
    currents_domain::segments::ReemissionV1 {
        id: id(base),
        original: r.id,
        replacement,
    }
}
