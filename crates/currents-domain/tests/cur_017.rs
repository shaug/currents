//! CUR-017 reconciliation claim records and activity coverage evaluation.
use currents_domain::catalog::CatalogEngine;
use currents_domain::reconciliation::ReconciliationEngine;
use currents_types::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::{amount, date, id};

/// The claim golden vector from the contract fixtures.
fn golden() -> Value {
    let records: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/contracts/v1/records.json")).unwrap();
    records
        .into_iter()
        .find(|r| r["kind"] == "ReconciliationClaim")
        .expect("claim fixture")
}
fn canonical(value: &Value) -> Vec<u8> {
    serde_json_canonicalizer::to_vec(value).unwrap()
}
fn decode(value: &Value) -> serde_json::Result<AuthoritativeRecordV1> {
    serde_json::from_slice(&canonical(value))
}

#[test]
fn cur_017_claim_record_round_trip() {
    let id = json!("00000000-0000-0000-0000-000000000096");
    for kind in ["Balance", "Activity", "Relationship", "Holdings"] {
        for status in [
            "Proposed",
            "PartiallySupported",
            "ConfirmedForScope",
            "Disputed",
            "Superseded",
        ] {
            let mut record = golden();
            let value = &mut record["value"];
            value["kind"] = json!(kind);
            value["status"] = json!(status);
            value["accounting_dependencies"] = json!([id]);
            value["reasons"] = json!([
                "UnknownOpening",
                "IncompleteInterval",
                "IncompatibleBalanceKinds",
                "MissingDateMapping",
                "UnexplainedResidual"
            ]);
            let claim = decode(&record).unwrap();
            let encoded = serde_json::to_value(&claim).unwrap();
            assert_eq!(canonical(&encoded), canonical(&record), "{kind} {status}");
        }
    }

    // An unknown kind or status, or the old untyped comparison, fails.
    for (field, value) in [
        ("kind", "Cash"),
        ("status", "Matched"),
        ("status", "Invalidated"),
        ("reasons", "UnknownOpening"),
    ] {
        let mut record = golden();
        record["value"][field] = json!(value);
        assert!(decode(&record).is_err(), "{field} {value}");
    }
    let mut legacy = golden();
    let value = legacy["value"].as_object_mut().unwrap();
    value.remove("kind");
    value.insert("comparison_type".into(), json!("synthetic"));
    assert!(decode(&legacy).is_err());
    let mut both = golden();
    both["value"]["comparison_type"] = json!("synthetic");
    assert!(decode(&both).is_err());
}

// Activity coverage over the S9 offsetting-omissions journey.

const ACCOUNT_A: u32 = 10;
const ACCOUNT_B: u32 = 60;
const EXPENSE: u32 = 30;
const RECEIPT: u32 = 1100;
const OPENING: u32 = 1200;
const CLOSING: u32 = 1210;

fn journey() -> Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/journeys/s9/offsetting-omissions.json"
    ))
    .unwrap()
}

fn civil(day: &str) -> Date {
    let TemporalValue::Date { civil, .. } = date(day) else {
        unreachable!()
    };
    civil
}

fn account(n: u32) -> AccountId {
    AccountId(id(n).0)
}

fn basis(value: &str) -> DateBasis {
    serde_json::from_value(json!(value)).unwrap()
}

/// Account A's statement and receipt, with only the fixture's recognized
/// lines recognized.
struct World {
    state: DomainState,
    lines: BTreeMap<String, (EvidenceLineV1, i128, String)>,
    evidence: Vec<RecordId>,
    next: u32,
}

impl World {
    fn new() -> Self {
        let journey = journey();
        let mut state = fixture::monetary_state();
        let mut savings = fixture::catalog();
        savings.revision_id = id(ACCOUNT_B + 2);
        savings.catalog.component_id = id(ACCOUNT_B + 1);
        savings.catalog.account_id = account(ACCOUNT_B);
        savings.ownership.account_id = account(ACCOUNT_B);
        savings.catalog.display_name = "Synthetic savings".into();
        let plan = CatalogEngine::set_complete(&state, &savings).unwrap();
        fixture::apply(&mut state, &plan);
        let mut world = Self {
            state,
            lines: BTreeMap::new(),
            evidence: vec![],
            next: 5000,
        };
        world.receipt(RECEIPT, &journey["coverage"]);
        for (n, key) in [(OPENING, "opening"), (CLOSING, "closing")] {
            let balance = &journey[key];
            world.observe(
                n,
                ObservationKind::Balance,
                balance["amount"].as_i64().unwrap().into(),
                date(balance["as_of"].as_str().unwrap()),
            );
            world.evidence.push(id(n));
        }
        for (i, line) in journey["lines"].as_array().unwrap().iter().enumerate() {
            let n = 1300 + 10 * i as u32;
            let value: i128 = line["amount"].as_i64().unwrap().into();
            let day = line["date"].as_str().unwrap().to_string();
            let identity = world.observe(n, ObservationKind::Movement, value, date(&day));
            world.evidence.push(id(n));
            world.lines.insert(
                line["name"].as_str().unwrap().into(),
                (identity, value, day),
            );
        }
        for name in journey["recognized"].as_array().unwrap() {
            world.recognize_line(name.as_str().unwrap());
        }
        world
    }

    /// The fixture world with every statement line recognized.
    fn complete() -> Self {
        let mut world = Self::new();
        world.recognize_line("deposit");
        world.recognize_line("reversal");
        world
    }

    fn line(&self, name: &str) -> EvidenceLineV1 {
        self.lines[name].0
    }

    /// Import receipt `n` reporting `coverage`, named as evidence.
    fn receipt(&mut self, n: u32, coverage: &Value) {
        let reported = vec![CoverageV1 {
            scope: DateScope {
                start: Some(civil(coverage["start"].as_str().unwrap())),
                end: Some(civil(coverage["end"].as_str().unwrap())),
                basis: basis(coverage["basis"].as_str().unwrap()),
            },
            status: serde_json::from_value(coverage["status"].clone()).unwrap(),
        }];
        let receipt = ImportReceiptV1 {
            id: id(n),
            source: source(),
            adapter_version: "synthetic-file/1".into(),
            parser_version: "1".into(),
            requested_coverage: reported.clone(),
            reported_coverage: reported,
            received_time: serde_json::from_value(json!("2026-12-01T00:00:00Z")).unwrap(),
            result: ImportResult::Complete,
            cursor: SourceValue::Absent,
            truncated: false,
            complete: true,
            contents: vec![],
        };
        self.state
            .records
            .insert(id(n), AuthoritativeRecordV1::ImportReceipt(receipt));
        if !self.evidence.contains(&id(n)) {
            self.evidence.push(id(n));
        }
    }

    /// Observation `n` with one facet, parsed by product `n + 1`.
    fn observe(
        &mut self,
        n: u32,
        kind: ObservationKind,
        value: i128,
        temporal: TemporalValue,
    ) -> EvidenceLineV1 {
        let observation = ObservationVersionV1 {
            id: id(n),
            source: source(),
            original_ids: vec![format!("row:{n}")],
            sanitized_representation: "synthetic".into(),
            reported_fields: vec![],
            temporal: temporal.clone(),
            semantic_kind: kind,
            prior_version: None,
            tombstone_of: None,
            content_digest: Digest([0; 32]),
        };
        self.state.records.insert(
            id(n),
            AuthoritativeRecordV1::ObservationVersion(observation),
        );
        let line = EvidenceLineV1 {
            observation: id(n),
            facet: id(n + 2),
        };
        self.state.records.insert(
            id(n + 1),
            AuthoritativeRecordV1::AddressedParseProduct(AddressedParseProductV1 {
                id: id(n + 1),
                schema_version: 1,
                product: ParseProductV1 {
                    source_version_id: id(n),
                    parser_id: "synthetic".into(),
                    parser_version: "1".into(),
                    facets: vec![MovementFacetV1 {
                        lineage_key: line.facet,
                        kind,
                        amount: SourceValue::Known(amount(value)),
                        relationships: vec![],
                        temporal,
                    }],
                },
            }),
        );
        line
    }

    fn recognize_line(&mut self, name: &str) -> (LegId, RecognitionId) {
        let (line, value, day) = self.lines[name].clone();
        self.recognize(line, ACCOUNT_A, value, &day, &day)
    }

    /// A posted activity recognition of `line` on account `on` against
    /// expense, with one leg.
    fn recognize(
        &mut self,
        line: EvidenceLineV1,
        on: u32,
        value: i128,
        effective: &str,
        posting: &str,
    ) -> (LegId, RecognitionId) {
        let n = self.next;
        self.next += 10;
        let allocation = allocation(line, value);
        let leg = FinancialLegV1 {
            id: LegId(id(n).0),
            event_id: id(n + 1),
            account_id: account(on),
            movement: amount(value),
            allocations: vec![allocation.clone()],
            effective: date(effective),
            posting: date(posting),
            relationships: vec![],
            unresolved_residual: SourceValue::Absent,
            interpretation: SourceValue::Known(fixture::interpretation()),
        };
        let posting_on = |n: u32, on: u32, value: i128| PostingV1 {
            id: id(n),
            account_id: account(on),
            amount: amount(value),
            effective: date(effective),
            posting: date(posting),
        };
        let recognition = RecognitionV1 {
            id: RecognitionId(id(n + 2).0),
            lineage: id(n + 3),
            postings: vec![
                posting_on(n + 4, on, value),
                posting_on(n + 5, EXPENSE, -value),
            ],
            kind: RecognitionKind::Activity,
            evidence_allocations: vec![allocation],
            leg_ids: vec![leg.id],
            scope: DateScope {
                start: Some(civil(effective)),
                end: Some(civil(effective)),
                basis: DateBasis::Effective,
            },
            bookkeeping_date: date(posting),
            date_policy: PolicyReferenceV1 {
                id: id(103),
                version: 1,
            },
        };
        let ids = (leg.id, recognition.id);
        self.state
            .records
            .insert(id(n), AuthoritativeRecordV1::FinancialLeg(leg));
        self.state
            .records
            .insert(id(n + 2), AuthoritativeRecordV1::Recognition(recognition));
        ids
    }

    /// Activity coverage on A over the fixture's coverage interval.
    fn request(&self) -> ReconciliationRequestV1 {
        let coverage = &journey()["coverage"];
        ReconciliationRequestV1 {
            kind: ReconciliationKindV1::Activity,
            scope: AccountScope {
                accounts: vec![account(ACCOUNT_A)],
                components: vec![],
            },
            cutoffs: DateScope {
                start: Some(civil(coverage["start"].as_str().unwrap())),
                end: Some(civil(coverage["end"].as_str().unwrap())),
                basis: basis(coverage["basis"].as_str().unwrap()),
            },
            evidence: self.evidence.clone(),
            exclusions: vec![],
            basis: read_basis(&self.state),
        }
    }

    fn evaluate(&self, request: &ReconciliationRequestV1) -> ReconciliationClaimV1 {
        ReconciliationEngine::evaluate(&self.state, request).unwrap()
    }

    /// Commit the next state. The accounting index follows it.
    fn advance(&mut self) {
        self.state.through.0 += 1;
        let index = self.state.accounting.as_mut().unwrap();
        index.prefix.through = self.state.through;
    }
}

fn source() -> SourceIdentityV1 {
    SourceIdentityV1 {
        source: "synthetic".into(),
        connection_id: id(102),
        device_id: None,
    }
}

fn read_basis(state: &DomainState) -> ReadBasis {
    serde_json::from_value(json!({
        "dataset_id": id(100),
        "through_commit": state.through,
        "effective_scope": {"start": null, "end": null, "basis": "Effective"},
        "evaluation_instant": "2026-12-01T00:00:00Z",
        "query_semantics_version": 1,
        "selection_policy_version": 1,
        "unit_catalog_version": 1
    }))
    .unwrap()
}

fn allocation(line: EvidenceLineV1, value: i128) -> EvidenceAllocationV1 {
    EvidenceAllocationV1 {
        observation: line.observation,
        facet: line.facet,
        amount: amount(value),
    }
}

#[test]
fn cur_017_1_offsetting_omissions() {
    let journey = journey();
    let world = World::new();
    // The net check alone passes: opening plus recognized movement is the
    // closing.
    let recognized: i64 = journey["recognized"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| world.lines[name.as_str().unwrap()].1 as i64)
        .sum();
    assert_eq!(
        journey["opening"]["amount"].as_i64().unwrap() + recognized,
        journey["closing"]["amount"].as_i64().unwrap()
    );

    let request = world.request();
    let claim = world.evaluate(&request);
    assert_eq!(
        serde_json::to_value(claim.status).unwrap(),
        journey["expected"]["status"]
    );
    let missing: Vec<_> = journey["expected"]["missing_allocations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| ClaimReasonV1::MissingAllocation(world.line(name.as_str().unwrap())))
        .collect();
    assert_eq!(claim.reasons, missing);
    assert!(claim.differences.is_empty());
    assert_eq!(
        claim.matched_allocations,
        vec![allocation(world.line("debit"), -40)]
    );

    // The claim records its inputs and basis.
    assert_eq!(claim.kind, ReconciliationKindV1::Activity);
    assert_eq!(claim.basis, request.basis);
    assert_eq!(claim.scope, request.scope.accounts);
    assert_eq!(claim.unit, UnitId(id(1).0));
    assert_eq!(claim.cutoffs, request.cutoffs);
    let mut evidence = request.evidence.clone();
    evidence.sort();
    assert_eq!(claim.evidence_dependencies, evidence);
    assert_eq!(claim.comparison_version, 1);
    assert!(claim.exclusions.is_empty());

    // Evaluation is deterministic, and the claim's line reasons survive
    // the wire.
    assert_eq!(world.evaluate(&request), claim);
    let record = AuthoritativeRecordV1::ReconciliationClaim(claim);
    let bytes = serde_json::to_vec(&record).unwrap();
    let decoded: AuthoritativeRecordV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, record);
}

#[test]
fn cur_017_1_complete_coverage_is_proposed() {
    let world = World::complete();
    let claim = world.evaluate(&world.request());
    assert_eq!(claim.status, ReconciliationStatus::Proposed);
    assert!(claim.reasons.is_empty());
    assert!(claim.differences.is_empty());
    assert_eq!(
        claim.matched_allocations,
        vec![
            allocation(world.line("debit"), -40),
            allocation(world.line("deposit"), 100),
            allocation(world.line("reversal"), -100),
        ]
    );
    // The claim names the three recognitions and three legs it compared.
    assert_eq!(claim.accounting_dependencies.len(), 6);
}

#[test]
fn cur_017_activity_explicit_exclusion() {
    let world = World::new();
    let deposit = world.line("deposit").observation;
    let reversal = world.line("reversal").observation;

    let mut request = world.request();
    request.exclusions = vec![reversal, deposit];
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::Proposed);
    assert!(claim.reasons.is_empty());
    let mut both = vec![deposit, reversal];
    both.sort();
    assert_eq!(claim.exclusions, both);

    request.exclusions = vec![deposit];
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::PartiallySupported);
    assert_eq!(
        claim.reasons,
        vec![ClaimReasonV1::MissingAllocation(world.line("reversal"))]
    );
    assert_eq!(claim.exclusions, vec![deposit]);

    // An exclusion must name a source line or a recognized leg.
    request.exclusions = vec![id(9999)];
    assert!(ReconciliationEngine::evaluate(&world.state, &request).is_err());

    // A recognized leg on A in November that allocates no source line, as
    // an association's transfer leg does, keeps the claim partial until it
    // is excluded.
    let mut world = World::complete();
    let transfer = world.observe(3300, ObservationKind::Movement, -100, date("2026-11-15"));
    let (leg, recognition) = world.recognize(transfer, ACCOUNT_A, -100, "2026-11-15", "2026-11-15");
    for record in world.state.records.values_mut() {
        match record {
            AuthoritativeRecordV1::FinancialLeg(l) if l.id == leg => l.allocations.clear(),
            AuthoritativeRecordV1::Recognition(r) if r.id == recognition => {
                r.evidence_allocations.clear()
            }
            _ => {}
        }
    }
    let mut request = world.request();
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::PartiallySupported);
    assert_eq!(claim.reasons, vec![ClaimReasonV1::UnsupportedLeg(leg)]);
    request.exclusions = vec![RecordId(leg.0)];
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::Proposed);
    assert_eq!(claim.exclusions, vec![RecordId(leg.0)]);
}

#[test]
fn cur_017_activity_needs_complete_interval() {
    let world = World::complete();

    // Without the receipt's coverage report.
    let mut request = world.request();
    request.evidence.retain(|e| *e != id(RECEIPT));
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::PartiallySupported);
    assert_eq!(claim.reasons, vec![ClaimReasonV1::IncompleteInterval]);

    // An interval that extends past the reported coverage. The statement
    // has no closing balance at its new end either.
    let mut request = world.request();
    request.cutoffs.end = Some(civil("2026-12-01"));
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::PartiallySupported);
    assert_eq!(
        claim.reasons,
        vec![
            ClaimReasonV1::IncompleteInterval,
            ClaimReasonV1::UnknownClosing
        ]
    );

    // A coverage report by another date basis, or a partial one.
    for (basis, status) in [("Effective", "Complete"), ("Posting", "Partial")] {
        let mut world = World::complete();
        let mut coverage = journey()["coverage"].clone();
        coverage["basis"] = json!(basis);
        coverage["status"] = json!(status);
        world.receipt(RECEIPT, &coverage);
        let claim = world.evaluate(&world.request());
        assert_eq!(claim.reasons, vec![ClaimReasonV1::IncompleteInterval]);
    }
}

#[test]
fn cur_017_activity_scope_and_basis() {
    let mut world = World::complete();
    let before = world.evaluate(&world.request());
    assert_eq!(before.status, ReconciliationStatus::Proposed);

    // A recognized leg on account B in November.
    let savings = world.observe(3000, ObservationKind::Movement, 500, date("2026-11-10"));
    world.recognize(savings, ACCOUNT_B, 500, "2026-11-10", "2026-11-10");
    // A recognized leg on A that posts on 2026-12-01.
    let late = world.observe(3100, ObservationKind::Movement, -7, date("2026-11-30"));
    let (late_leg, _) = world.recognize(late, ACCOUNT_A, -7, "2026-11-30", "2026-12-01");

    let claim = world.evaluate(&world.request());
    assert_eq!(claim.status, ReconciliationStatus::Proposed);
    assert_eq!(claim.matched_allocations, before.matched_allocations);
    assert_eq!(
        claim.accounting_dependencies,
        before.accounting_dependencies
    );
    assert_eq!(claim.dependency_digest, before.dependency_digest);

    // By effective date, the late leg falls in November, and the source
    // does not report its line.
    let mut coverage = journey()["coverage"].clone();
    coverage["basis"] = json!("Effective");
    world.receipt(1150, &coverage);
    let mut request = world.request();
    request.cutoffs.basis = DateBasis::Effective;
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::PartiallySupported);
    assert_eq!(
        claim.reasons,
        vec![ClaimReasonV1::UnexpectedAllocation(late)]
    );
    assert!(
        claim
            .accounting_dependencies
            .contains(&RecordId(late_leg.0))
    );

    // A named line without a civil date cannot be placed in the interval.
    let unknown = world.observe(
        3200,
        ObservationKind::Movement,
        3,
        serde_json::from_value(json!({"kind":"Unknown","original_text":"sometime"})).unwrap(),
    );
    let mut request = world.request();
    request.evidence.push(unknown.observation);
    let claim = world.evaluate(&request);
    assert_eq!(claim.status, ReconciliationStatus::PartiallySupported);
    assert_eq!(claim.reasons, vec![ClaimReasonV1::MissingDateMapping]);
}

#[test]
fn cur_017_activity_digest_ignores_posting_ids() {
    let mut world = World::complete();
    let claim = world.evaluate(&world.request());

    // An identity-only correction: the debit's recognition is reversed and
    // replaced with fresh recognition and posting IDs on the same leg.
    let debit = world.line("debit");
    let original = world
        .state
        .records
        .values()
        .find_map(|record| match record {
            AuthoritativeRecordV1::Recognition(r)
                if r.evidence_allocations[0].observation == debit.observation =>
            {
                Some(r.clone())
            }
            _ => None,
        })
        .unwrap();
    let mut replacement = original.clone();
    replacement.id = RecognitionId(id(7000).0);
    for (i, posting) in replacement.postings.iter_mut().enumerate() {
        posting.id = id(7001 + i as u32);
    }
    world
        .state
        .records
        .insert(id(7000), AuthoritativeRecordV1::Recognition(replacement));
    world
        .state
        .accounting
        .as_mut()
        .unwrap()
        .reversed
        .insert(original.id);
    world.advance();
    let corrected = world.evaluate(&world.request());
    assert_eq!(corrected.status, ReconciliationStatus::Proposed);
    assert_eq!(corrected.dependency_digest, claim.dependency_digest);
    assert!(corrected.accounting_dependencies.contains(&id(7000)));
    assert!(
        !corrected
            .accounting_dependencies
            .contains(&RecordId(original.id.0))
    );

    // A recognized line's amount changes.
    let mut changed = World::complete();
    let leg = changed
        .state
        .records
        .values_mut()
        .find_map(|record| match record {
            AuthoritativeRecordV1::FinancialLeg(l)
                if l.allocations[0].observation == debit.observation =>
            {
                Some(l)
            }
            _ => None,
        })
        .unwrap();
    leg.movement = amount(-45);
    leg.allocations[0].amount = amount(-45);
    let after = changed.evaluate(&changed.request());
    assert_ne!(after.dependency_digest, claim.dependency_digest);

    // A named observation gains a newer version.
    let mut revised = World::complete();
    let deposit = revised.line("deposit").observation;
    revised.observe(7100, ObservationKind::Movement, 100, date("2026-11-12"));
    let Some(AuthoritativeRecordV1::ObservationVersion(next)) =
        revised.state.records.get_mut(&id(7100))
    else {
        unreachable!()
    };
    next.prior_version = Some(deposit);
    let after = revised.evaluate(&revised.request());
    assert_ne!(after.dependency_digest, claim.dependency_digest);
}

#[test]
fn cur_017_activity_refuses_other_bases() {
    let world = World::complete();
    let mut request = world.request();
    request.basis.through_commit = CommitSeq(world.state.through.0 + 1);
    let error = ReconciliationEngine::evaluate(&world.state, &request).unwrap_err();
    assert_eq!(error.class, ErrorClass::StaleBasis);

    // Other claim kinds are not evaluated yet.
    for kind in [
        ReconciliationKindV1::Balance,
        ReconciliationKindV1::Relationship,
        ReconciliationKindV1::Holdings,
    ] {
        let mut request = world.request();
        request.kind = kind;
        let error = ReconciliationEngine::evaluate(&world.state, &request).unwrap_err();
        assert_eq!(error.class, ErrorClass::UnsupportedVersion);
    }
}
