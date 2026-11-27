//! CUR-017 reconciliation claim records, activity coverage evaluation, and
//! recording, confirming and replaying claims, and when a claim stops
//! applying.
use currents_accounting::AccountingAdapter;
use currents_domain::ReplayEngine;
use currents_domain::catalog::CatalogEngine;
use currents_domain::recognition::RecognitionEngine;
use currents_domain::reconciliation::{
    ClaimApplicabilityV1, InvalidationReasonV1, ReconciliationEngine,
};
use currents_types::*;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
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
        // Native accounts, B among them, so adoptions can post.
        let index = state.accounting.as_mut().unwrap();
        let mut savings = index.accounts[&account(ACCOUNT_A)].clone();
        savings.account_id = account(ACCOUNT_B);
        index.accounts.insert(savings.account_id, savings);
        for mapping in index.accounts.values_mut() {
            mapping.path = format!("Currents:A{}", mapping.account_id.0.simple());
        }
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
    // The claim names the three legs it compared, and no recognition.
    let mut legs: Vec<_> = ["debit", "deposit", "reversal"]
        .iter()
        .map(|name| RecordId(world.leg_of(name).0))
        .collect();
    legs.sort();
    assert_eq!(claim.accounting_dependencies, legs);
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
    assert_eq!(
        corrected.accounting_dependencies,
        claim.accounting_dependencies
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

// Recording, confirming and replaying claims. K is the November activity
// claim on A with every statement line recognized, so it is `Proposed`.

/// One commit of `plan` after `state` with one decision of `kind`, and the
/// frozen instruction bytes its manifest names.
fn proposed(
    state: &DomainState,
    plan: &DecisionPlan,
    kind: DecisionKind,
) -> (DomainCommitV1, Vec<u8>) {
    let seq = CommitSeq(state.through.0 + 1);
    let n = 8000 + 10 * seq.0 as u32;
    let batch = AccountingAdapter::freeze(&plan.accounting, state).unwrap();
    let mut records = plan.records.clone();
    records.push(AuthoritativeRecordV1::FrozenAccountingManifest(
        AccountingAdapter::manifest(id(n), &batch).unwrap(),
    ));
    let command_id = CommandId(id(n + 1).0);
    let command_digest = Digest([9; 32]);
    let adopted: Vec<RecordId> = plan
        .records
        .iter()
        .map(|r| serde_json::from_value(serde_json::to_value(r).unwrap()["value"]["id"].clone()))
        .collect::<Result<_, _>>()
        .unwrap();
    let decision = DecisionEnvelopeV1 {
        id: id(n + 2),
        commit: seq,
        item_position: records.len() as u32 + 1,
        decision_type: kind,
        schema_version: 1,
        actor: id(102),
        authority: AuthorityKind::Owner,
        grant_id: None,
        command_id,
        command_digest,
        references: vec![],
        recorded_time: serde_json::from_value(json!("2026-12-01T00:00:00Z")).unwrap(),
        adopted_result: adopted.clone(),
        policy: PolicyReferenceV1 {
            id: id(104),
            version: 1,
        },
        corrections: vec![],
        supersedes: vec![],
    };
    let mut commit = DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records,
        decisions: vec![decision],
        native_head_after: state
            .accounting
            .as_ref()
            .unwrap()
            .prefix
            .native_head
            .clone(),
        command_result: CommandResultV1 {
            command_id,
            digest: command_digest,
            state: CommandState::Committed,
            commit: Some(seq),
            effects: EffectsV1 {
                records: adopted,
                accounts: vec![],
                amounts: vec![],
                invalidations: plan.invalidations.clone(),
            },
            health: vec![],
        },
        digest: Digest([0; 32]),
    };
    seal(&mut commit);
    let bytes = AccountingAdapter::instruction_bytes(&batch).unwrap();
    (commit, bytes)
}

/// Bind `commit`'s digest again after a change.
fn seal(commit: &mut DomainCommitV1) {
    commit.digest = Digest([0; 32]);
    commit.digest =
        Digest(Sha256::digest(serde_json_canonicalizer::to_vec(&*commit).unwrap()).into());
}

/// Replay `commit` after `state`; the native index follows it.
fn admit(
    state: &DomainState,
    (commit, bytes): &(DomainCommitV1, Vec<u8>),
) -> Result<DomainState, EngineError> {
    let mut next = state.clone();
    ReplayEngine::apply(&mut next, commit)?;
    let mut native = state.clone();
    AccountingAdapter::advance_index(&mut native, commit, bytes)?;
    next.accounting = native.accounting;
    Ok(next)
}

/// The domain replay refuses `commit` after `state` and keeps the prior head.
fn refused(state: &DomainState, (commit, _): &(DomainCommitV1, Vec<u8>)) {
    let mut copy = state.clone();
    assert!(ReplayEngine::apply(&mut copy, commit).is_err());
    assert_eq!(copy.through, state.through);
    assert_eq!(copy.records, state.records);
}

fn only_claim(plan: &DecisionPlan) -> ReconciliationClaimV1 {
    let [AuthoritativeRecordV1::ReconciliationClaim(claim)] = plan.records.as_slice() else {
        panic!("one claim")
    };
    claim.clone()
}

fn only_disposition(plan: &DecisionPlan) -> ReconciliationDispositionV1 {
    let [AuthoritativeRecordV1::ReconciliationDisposition(d)] = plan.records.as_slice() else {
        panic!("one disposition")
    };
    d.clone()
}

fn lifecycle(state: &DomainState, claim: RecordId) -> ReconciliationStatus {
    ReconciliationEngine::lifecycle(state, claim).unwrap()
}

fn bytes(state: &DomainState, record: RecordId) -> Vec<u8> {
    serde_json::to_vec(&state.records[&record]).unwrap()
}

impl World {
    /// Commit `plan` with one `Reconciliation` decision.
    fn commit(&mut self, plan: &DecisionPlan) -> (DomainCommitV1, Vec<u8>) {
        let commit = proposed(&self.state, plan, DecisionKind::Reconciliation);
        self.state = admit(&self.state, &commit).unwrap();
        commit
    }

    /// Record the claim `request` evaluates.
    fn record(&mut self, request: &ReconciliationRequestV1) -> ReconciliationClaimV1 {
        let plan = ReconciliationEngine::record(&self.state, request).unwrap();
        self.commit(&plan);
        only_claim(&plan)
    }

    /// Recognize a backdated -20 line on A on 2026-11-08.
    fn backdate(&mut self) {
        let line = self.observe(3400, ObservationKind::Movement, -20, date("2026-11-08"));
        self.recognize(line, ACCOUNT_A, -20, "2026-11-08", "2026-11-08");
        self.advance();
    }
}

#[test]
fn cur_017_record_and_replay_claim() {
    let world = World::complete();
    let request = world.request();
    let plan = ReconciliationEngine::record(&world.state, &request).unwrap();
    let k = world.evaluate(&request);
    assert_eq!(k.status, ReconciliationStatus::Proposed);
    assert_eq!(
        plan.records,
        vec![AuthoritativeRecordV1::ReconciliationClaim(k.clone())]
    );
    assert!(plan.invalidations.is_empty());
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    let batch = AccountingAdapter::freeze(&plan.accounting, &world.state).unwrap();
    assert!(batch.operations.is_empty());

    // The commit holds one claim and one `Reconciliation` decision.
    let good = proposed(&world.state, &plan, DecisionKind::Reconciliation);
    let claims = good
        .0
        .records
        .iter()
        .filter(|r| matches!(r, AuthoritativeRecordV1::ReconciliationClaim(_)))
        .count();
    assert_eq!(claims, 1);
    assert_eq!(good.0.decisions.len(), 1);
    assert_eq!(
        good.0.decisions[0].decision_type,
        DecisionKind::Reconciliation
    );
    let recorded = admit(&world.state, &good).unwrap();
    assert_eq!(recorded.records[&k.id], plan.records[0]);
    assert_eq!(lifecycle(&recorded, k.id), ReconciliationStatus::Proposed);
    let index = |s: &DomainState| s.accounting.as_ref().unwrap().recognitions.clone();
    assert_eq!(index(&recorded), index(&world.state));

    // Replay derives the claim again and rejects any that differ.
    let forged = |change: &dyn Fn(&mut ReconciliationClaimV1)| {
        let mut claim = k.clone();
        change(&mut claim);
        let mut plan = plan.clone();
        plan.records = vec![AuthoritativeRecordV1::ReconciliationClaim(claim)];
        proposed(&world.state, &plan, DecisionKind::Reconciliation)
    };
    refused(
        &world.state,
        &forged(&|c| c.status = ReconciliationStatus::ConfirmedForScope),
    );
    refused(
        &world.state,
        &forged(&|c| c.status = ReconciliationStatus::PartiallySupported),
    );
    refused(&world.state, &forged(&|c| c.differences.push(amount(1))));
    refused(
        &world.state,
        &forged(&|c| c.exclusions.push(world.line("deposit").observation)),
    );
    refused(
        &world.state,
        &forged(&|c| c.dependency_digest = Digest([1; 32])),
    );

    // So does a claim commit with another decision kind, a second decision,
    // or an invalidation.
    refused(
        &world.state,
        &proposed(&world.state, &plan, DecisionKind::Correction),
    );
    let mut twice = good.clone();
    let mut second = twice.0.decisions[0].clone();
    second.id = id(8999);
    twice.0.decisions.push(second);
    seal(&mut twice.0);
    refused(&world.state, &twice);
    let mut invalidating = plan.clone();
    invalidating.invalidations = vec![k.id];
    refused(
        &world.state,
        &proposed(&world.state, &invalidating, DecisionKind::Reconciliation),
    );
    // A claim whose basis is not the prior commit cannot be derived again.
    let mut later = world.state.clone();
    later.through.0 += 1;
    later.accounting.as_mut().unwrap().prefix.through = later.through;
    refused(
        &later,
        &proposed(&later, &plan, DecisionKind::Reconciliation),
    );
}

#[test]
fn cur_017_confirm_rechecks_digest() {
    let mut world = World::complete();
    let k = world.record(&world.request());
    let at_claim = world.state.clone();

    let error = ReconciliationEngine::confirm(&world.state, k.id, Digest([7; 32])).unwrap_err();
    assert_eq!(error.class, ErrorClass::StaleBasis);
    let plan = ReconciliationEngine::confirm(&world.state, k.id, k.dependency_digest).unwrap();
    assert!(plan.invalidations.is_empty());
    assert!(matches!(plan.accounting, AccountingIntent::NoEffect(_)));
    let confirmation = only_disposition(&plan);
    assert_eq!(confirmation.claim, k.id);
    assert_eq!(confirmation.dependency_digest, k.dependency_digest);
    assert_eq!(
        confirmation.disposition,
        ClaimDispositionV1::ConfirmedForScope
    );
    world.commit(&plan);
    assert_eq!(lifecycle(&at_claim, k.id), ReconciliationStatus::Proposed);
    assert_eq!(
        lifecycle(&world.state, k.id),
        ReconciliationStatus::ConfirmedForScope
    );
    assert_eq!(bytes(&world.state, k.id), bytes(&at_claim, k.id));
    // A confirmed claim is not confirmed or disputed again.
    assert!(ReconciliationEngine::confirm(&world.state, k.id, k.dependency_digest).is_err());
    assert!(
        ReconciliationEngine::dispute(&world.state, k.id, k.dependency_digest, "Wrong").is_err()
    );

    // After a backdated November line on A is recognized, K's digest no
    // longer matches the current one.
    let mut late = World::complete();
    let k = late.record(&late.request());
    late.backdate();
    let error = ReconciliationEngine::confirm(&late.state, k.id, k.dependency_digest).unwrap_err();
    assert_eq!(error.class, ErrorClass::StaleBasis);
    assert_eq!(lifecycle(&late.state, k.id), ReconciliationStatus::Proposed);
}

#[test]
fn cur_017_partial_claim_cannot_be_confirmed() {
    let mut world = World::new();
    let claim = world.record(&world.request());
    assert_eq!(claim.status, ReconciliationStatus::PartiallySupported);
    let at_claim = world.state.clone();
    let digest = claim.dependency_digest;
    assert!(ReconciliationEngine::confirm(&world.state, claim.id, digest).is_err());

    let reason = "The deposit and its reversal are on the statement";
    assert!(
        ReconciliationEngine::dispute(&world.state, claim.id, Digest([7; 32]), reason).is_err()
    );
    assert!(ReconciliationEngine::dispute(&world.state, claim.id, digest, " ").is_err());
    let plan = ReconciliationEngine::dispute(&world.state, claim.id, digest, reason).unwrap();
    let dispute = only_disposition(&plan);
    assert_eq!(dispute.disposition, ClaimDispositionV1::Disputed);
    assert_eq!(dispute.dependency_digest, digest);
    assert_eq!(dispute.reason, reason);
    world.commit(&plan);
    assert_eq!(
        lifecycle(&at_claim, claim.id),
        ReconciliationStatus::PartiallySupported
    );
    assert_eq!(
        lifecycle(&world.state, claim.id),
        ReconciliationStatus::Disputed
    );
    assert_eq!(bytes(&world.state, claim.id), bytes(&at_claim, claim.id));
    assert!(ReconciliationEngine::dispute(&world.state, claim.id, digest, reason).is_err());
}

#[test]
fn cur_017_supersession() {
    let mut world = World::complete();
    let k = world.record(&world.request());
    let original = bytes(&world.state, k.id);

    // A December claim on A leaves K unchanged.
    let mut december = world.request();
    december.cutoffs.start = Some(civil("2026-12-01"));
    december.cutoffs.end = Some(civil("2026-12-31"));
    let other = world.record(&december);
    assert_eq!(
        lifecycle(&world.state, k.id),
        ReconciliationStatus::Proposed
    );
    assert_eq!(lifecycle(&world.state, other.id), other.status);
    let before = world.state.clone();

    // A second November activity claim on A supersedes K from its commit.
    let newer = world.record(&world.request());
    assert_ne!(newer.id, k.id);
    assert_eq!(lifecycle(&before, k.id), ReconciliationStatus::Proposed);
    assert_eq!(
        lifecycle(&world.state, k.id),
        ReconciliationStatus::Superseded
    );
    assert_eq!(
        lifecycle(&world.state, newer.id),
        ReconciliationStatus::Proposed
    );
    assert_eq!(lifecycle(&world.state, other.id), other.status);
    assert_eq!(bytes(&world.state, k.id), original);
    // A superseded claim is not confirmed or disputed.
    assert!(ReconciliationEngine::confirm(&world.state, k.id, k.dependency_digest).is_err());
    assert!(
        ReconciliationEngine::dispute(&world.state, k.id, k.dependency_digest, "Stale").is_err()
    );
}

#[test]
fn cur_017_original_confirmation_reproducible() {
    let mut world = World::complete();
    let base = world.state.clone();
    let mut commits = Vec::new();
    let plan = ReconciliationEngine::record(&world.state, &world.request()).unwrap();
    let k = only_claim(&plan);
    commits.push(world.commit(&plan));
    let original = bytes(&world.state, k.id);
    let plan = ReconciliationEngine::confirm(&world.state, k.id, k.dependency_digest).unwrap();
    let confirmation = only_disposition(&plan);
    commits.push(world.commit(&plan));
    let plan = ReconciliationEngine::record(&world.state, &world.request()).unwrap();
    commits.push(world.commit(&plan));

    // Replay through the confirmation commit, then through the supersession.
    let mut state = base;
    for commit in &commits[..2] {
        state = admit(&state, commit).unwrap();
    }
    assert_eq!(
        lifecycle(&state, k.id),
        ReconciliationStatus::ConfirmedForScope
    );
    let Some(AuthoritativeRecordV1::ReconciliationDisposition(replayed)) =
        state.records.get(&confirmation.id)
    else {
        panic!("the confirmation is replayed")
    };
    assert_eq!(replayed.dependency_digest, k.dependency_digest);
    assert_eq!(bytes(&state, k.id), original);

    state = admit(&state, &commits[2]).unwrap();
    assert_eq!(lifecycle(&state, k.id), ReconciliationStatus::Superseded);
    assert_eq!(bytes(&state, k.id), original);
    assert_eq!(
        state.records[&confirmation.id],
        AuthoritativeRecordV1::ReconciliationDisposition(confirmation)
    );
    assert_eq!(state.records, world.state.records);
}

#[test]
fn cur_017_replay_rejects_forged_confirmation() {
    let mut world = World::complete();
    let k = world.record(&world.request());
    let plan = ReconciliationEngine::confirm(&world.state, k.id, k.dependency_digest).unwrap();
    let with = |change: &dyn Fn(&mut ReconciliationDispositionV1)| {
        let mut confirmation = only_disposition(&plan);
        change(&mut confirmation);
        let mut plan = plan.clone();
        plan.records = vec![AuthoritativeRecordV1::ReconciliationDisposition(
            confirmation,
        )];
        proposed(&world.state, &plan, DecisionKind::Reconciliation)
    };
    refused(
        &world.state,
        &with(&|c| c.dependency_digest = Digest([5; 32])),
    );
    refused(
        &world.state,
        &with(&|c| c.disposition = ClaimDispositionV1::Disputed),
    );
    refused(&world.state, &with(&|c| c.claim = id(9999)));
    admit(
        &world.state,
        &proposed(&world.state, &plan, DecisionKind::Reconciliation),
    )
    .unwrap();

    // After a backdated line, the digest derived again at the prior history
    // differs from K's, so the confirmation K's digest earned earlier is
    // rejected.
    world.backdate();
    refused(
        &world.state,
        &proposed(&world.state, &plan, DecisionKind::Reconciliation),
    );
}

// Claim applicability. K is the November activity claim on A, recorded and
// confirmed. A claim stops applying from the commit that changes an input in
// its scope.

fn applicability(state: &DomainState, claim: RecordId) -> ClaimApplicabilityV1 {
    ReconciliationEngine::applicability(state, claim).unwrap()
}

fn invalidated(reason: InvalidationReasonV1) -> ClaimApplicabilityV1 {
    ClaimApplicabilityV1::Invalidated(vec![reason])
}

impl World {
    /// `World::complete` with K recorded and confirmed.
    fn confirmed() -> (Self, ReconciliationClaimV1) {
        let mut world = Self::complete();
        let k = world.record(&world.request());
        let plan = ReconciliationEngine::confirm(&world.state, k.id, k.dependency_digest).unwrap();
        world.commit(&plan);
        (world, k)
    }

    /// The adoption plan that recognizes a new `value` line on account `on`
    /// dated `day`, from observation `n`, which no claim names.
    fn adoption(&mut self, n: u32, on: u32, value: i128, day: &str) -> DecisionPlan {
        let line = self.observe(n, ObservationKind::Movement, value, date(day));
        let binding = SourceFacetBindingV1 {
            observation: line.observation,
            parse_product: id(n + 1),
            facet: line.facet,
            amount: amount(value),
        };
        let mut request = fixture::posted(binding, n + 10, EXPENSE);
        for item in &mut request.legs {
            item.leg.account_id = account(on);
            item.leg.effective = date(day);
            item.leg.posting = date(day);
        }
        let terms = request.financial.as_mut().unwrap();
        terms.bookkeeping_date = date(day);
        terms.postings[0].account_id = account(on);
        for posting in &mut terms.postings {
            posting.effective = date(day);
            posting.posting = date(day);
        }
        RecognitionEngine::adopt(&self.state, &request).unwrap()
    }

    /// Commit `plan` with one decision of `kind`.
    fn commit_as(&mut self, plan: &DecisionPlan, kind: DecisionKind) {
        let commit = proposed(&self.state, plan, kind);
        self.state = admit(&self.state, &commit).unwrap();
    }

    /// Commit revision `n` of A's catalog after `change`.
    fn revise_catalog(&mut self, n: u32, change: &dyn Fn(&mut SetAccountCatalogV1)) {
        let current = CatalogEngine::current(&self.state, id(ACCOUNT_A + 1)).unwrap();
        let mut request = fixture::catalog();
        request.revision_id = id(n);
        request.prior_revision = Some(current.id);
        request.catalog = current.catalog;
        change(&mut request.catalog);
        let plan = CatalogEngine::set_complete(&self.state, &request).unwrap();
        self.commit_as(&plan, DecisionKind::Catalog);
    }

    /// The leg that allocates the statement line `name`.
    fn leg_of(&self, name: &str) -> LegId {
        let line = self.line(name);
        self.state
            .records
            .values()
            .find_map(|record| match record {
                AuthoritativeRecordV1::FinancialLeg(l)
                    if l.allocations
                        .iter()
                        .any(|a| a.observation == line.observation) =>
                {
                    Some(l.id)
                }
                _ => None,
            })
            .unwrap()
    }
}

#[test]
fn cur_017_3_backdated_posting_invalidates() {
    let (mut world, k) = World::confirmed();
    let at_confirmation = world.state.clone();
    assert_eq!(
        applicability(&world.state, k.id),
        ClaimApplicabilityV1::Current
    );

    // A new 25 USD purchase on A posted 2026-11-12, from a line K never named.
    let plan = world.adoption(3500, ACCOUNT_A, -25, "2026-11-12");
    assert!(!plan.invalidations.contains(&k.id));
    world.commit_as(&plan, DecisionKind::Interpretation);
    assert_eq!(
        applicability(&world.state, k.id),
        invalidated(InvalidationReasonV1::ScopeActivity)
    );
    // The claim record and its confirmation keep their contents.
    assert_eq!(bytes(&world.state, k.id), bytes(&at_confirmation, k.id));
    assert_eq!(
        lifecycle(&world.state, k.id),
        ReconciliationStatus::ConfirmedForScope
    );
    // The history through K's confirmation still reports K current.
    assert_eq!(
        applicability(&at_confirmation, k.id),
        ClaimApplicabilityV1::Current
    );
    assert_eq!(
        lifecycle(&at_confirmation, k.id),
        ReconciliationStatus::ConfirmedForScope
    );
}

#[test]
fn cur_017_3_outside_scope_keeps_claim() {
    // A line on A posted 2026-12-02, after K's interval.
    let (mut world, k) = World::confirmed();
    let plan = world.adoption(3500, ACCOUNT_A, -25, "2026-12-02");
    world.commit_as(&plan, DecisionKind::Interpretation);
    assert_eq!(
        applicability(&world.state, k.id),
        ClaimApplicabilityV1::Current
    );

    // A line on B posted 2026-11-12, inside K's interval.
    let (mut world, k) = World::confirmed();
    let plan = world.adoption(3500, ACCOUNT_B, -25, "2026-11-12");
    world.commit_as(&plan, DecisionKind::Interpretation);
    assert_eq!(
        applicability(&world.state, k.id),
        ClaimApplicabilityV1::Current
    );
}

#[test]
fn cur_017_3_cosmetic_tag_keeps_claim() {
    let (mut world, k) = World::confirmed();
    // An organization tag on a leg K compared.
    let leg = world.leg_of("debit");
    assert!(k.accounting_dependencies.contains(&RecordId(leg.0)));
    let tag = OrganizationRecordV1 {
        id: id(3600),
        version: 1,
        kind: OrganizationKind::Tag,
        targets: vec![RecordId(leg.0)],
        parent: None,
        text: "groceries".into(),
        allocations: vec![],
        supersedes: None,
    };
    let plan = DecisionPlan {
        records: vec![AuthoritativeRecordV1::Organization(tag)],
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "Tag a leg".into(),
        }),
    };
    world.commit_as(&plan, DecisionKind::Organization);
    assert_eq!(
        applicability(&world.state, k.id),
        ClaimApplicabilityV1::Current
    );

    // A new display name for A.
    world.revise_catalog(3610, &|catalog| {
        catalog.display_name = "Everyday checking".into();
    });
    assert_eq!(
        applicability(&world.state, k.id),
        ClaimApplicabilityV1::Current
    );
}

#[test]
fn cur_017_input_changes_invalidate() {
    // A revised version of the deposit line K named.
    let (mut world, k) = World::confirmed();
    let deposit = world.line("deposit").observation;
    world.observe(3700, ObservationKind::Movement, 100, date("2026-11-12"));
    let Some(AuthoritativeRecordV1::ObservationVersion(revision)) =
        world.state.records.get_mut(&id(3700))
    else {
        unreachable!()
    };
    revision.prior_version = Some(deposit);
    world.advance();
    assert_eq!(
        applicability(&world.state, k.id),
        invalidated(InvalidationReasonV1::EvidenceRevision)
    );

    // A new source alias on A's catalog entry.
    let (mut world, k) = World::confirmed();
    world.revise_catalog(3710, &|catalog| {
        catalog.aliases.push(SourceAliasV1 {
            source: source(),
            original_id: "checking-0042".into(),
        });
    });
    assert_eq!(
        applicability(&world.state, k.id),
        invalidated(InvalidationReasonV1::AccountMapping)
    );

    // The named receipt now reports November partial.
    let (mut world, k) = World::confirmed();
    let mut coverage = journey()["coverage"].clone();
    coverage["status"] = json!("Partial");
    world.receipt(RECEIPT, &coverage);
    world.advance();
    assert_eq!(
        applicability(&world.state, k.id),
        invalidated(InvalidationReasonV1::Coverage)
    );
}

#[test]
fn cur_017_replay_rejects_claim_invalidation_elsewhere() {
    let (mut world, k) = World::confirmed();
    let mut plan = world.adoption(3500, ACCOUNT_A, -25, "2026-11-12");
    plan.invalidations.push(k.id);
    refused(
        &world.state,
        &proposed(&world.state, &plan, DecisionKind::Interpretation),
    );
    // The same adoption without the invalidation is admitted.
    plan.invalidations.clear();
    world.commit_as(&plan, DecisionKind::Interpretation);
    assert!(!world.state.claim_invalidated(k.id));
}

#[test]
fn cur_017_invalidated_claim_cannot_be_confirmed() {
    let mut world = World::complete();
    let k = world.record(&world.request());
    assert_eq!(
        lifecycle(&world.state, k.id),
        ReconciliationStatus::Proposed
    );
    let plan = world.adoption(3500, ACCOUNT_A, -25, "2026-11-12");
    world.commit_as(&plan, DecisionKind::Interpretation);
    assert_eq!(
        applicability(&world.state, k.id),
        invalidated(InvalidationReasonV1::ScopeActivity)
    );
    let error = ReconciliationEngine::confirm(&world.state, k.id, k.dependency_digest).unwrap_err();
    assert_eq!(error.class, ErrorClass::StaleBasis);
    assert_eq!(
        lifecycle(&world.state, k.id),
        ReconciliationStatus::Proposed
    );
}
