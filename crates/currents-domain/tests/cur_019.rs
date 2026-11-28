//! CUR-019 versioned owner organization on economic legs, using synthetic
//! identities and the exact public interfaces.
use currents_domain::{
    ReplayEngine,
    organization::{
        LegValueRequestV1, OrganizationChangeV1, OrganizationEngine, OrganizationTierV1,
        ResolvedFieldV1,
    },
    recognition::RecognitionEngine,
};
use currents_types::*;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;

/// The expense account E1 that both purchases debit.
const E1: u32 = 30;
/// Purchase L1 (96.00 USD on 2026-11-03) and L2 (40.00 USD on 2026-11-10),
/// both on account K, the fixture's external account. Each leg is its
/// purchase's ID plus one, its recognition plus three and its first posting
/// plus five. Observations O1 and O2 back them, and each observation's parse
/// product is its ID plus one. The S6 journey fixture gives their dates,
/// amounts and reported fields.
const L1: u32 = 8050;
const L2: u32 = 8100;
const O1: u32 = 8000;
const O2: u32 = 8010;
/// O1's source revision, which reports another category.
const O1_REVISED: u32 = 8020;
/// Aliases A1 and A2 for party P. Party Q and its aliases: A3 has A1's exact
/// text and A4 differs from A1 only in letter case.
const A1: u32 = 640;
const A2: u32 = 641;
const Q: u32 = 650;
const A3: u32 = 651;
const A4: u32 = 652;
/// Party P, categories Food, Coffee (under Food) and Groceries (under Food),
/// tags Travel and Flights (under Travel), and note N.
const P: u32 = 600;
const FOOD: u32 = 610;
const COFFEE: u32 = 611;
const GROCERIES: u32 = 612;
const TRAVEL: u32 = 620;
const FLIGHTS: u32 = 621;
const N: u32 = 630;

fn leg(purchase: u32) -> LegId {
    LegId(id(purchase + 1).0)
}

/// The S6 journey's synthetic input.
fn journey() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/journeys/s6/processor-change.json"
    ))
    .unwrap()
}

fn text(pointer: &str) -> String {
    journey().pointer(pointer).unwrap().as_str().unwrap().into()
}

fn reported(name: &str, value: &str) -> ReportedFieldV1 {
    ReportedFieldV1 {
        name: name.into(),
        value: SourceValue::Known(value.into()),
    }
}

/// A posted purchase whose observation `evidence_id` reports the fixture's
/// `fields`: its amount, date, description and any category.
fn purchase(state: &mut DomainState, n: u32, evidence_id: u32, fields: &serde_json::Value) {
    let value = i128::from(fields["amount"].as_i64().unwrap());
    let day = fields["date"].as_str().unwrap();
    let source = fixture::evidence(state, evidence_id, value, None);
    let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
        state.records.get_mut(&id(evidence_id))
    else {
        panic!("observation {evidence_id} missing")
    };
    for name in ["description", "category"] {
        if let Some(text) = fields[name].as_str() {
            observation.reported_fields.push(reported(name, text));
        }
    }
    let mut request = posted(source, n, E1);
    for adopted in &mut request.legs {
        adopted.leg.effective = date(day);
        adopted.leg.posting = date(day);
    }
    let terms = request.financial.as_mut().unwrap();
    terms.bookkeeping_date = date(day);
    for posting in &mut terms.postings {
        posting.effective = date(day);
        posting.posting = date(day);
    }
    let plan = RecognitionEngine::adopt(state, &request).unwrap();
    apply(state, &plan);
}

/// Account K holding purchases L1 and L2, before any organization.
fn world() -> DomainState {
    let mut state = monetary_state();
    let journey = journey();
    purchase(&mut state, L1, O1, &journey["purchases"][0]);
    purchase(&mut state, L2, O2, &journey["purchases"][1]);
    state
}

fn define(n: u32, kind: OrganizationKind, parent: Option<u32>, text: &str) -> OrganizationRecordV1 {
    OrganizationRecordV1 {
        id: id(n),
        version: 1,
        kind,
        targets: vec![],
        parent: parent.map(id),
        text: text.into(),
        allocations: vec![],
        supersedes: None,
    }
}

/// Version `prior.version + 1` of `prior`, with ID `n`.
fn revise(prior: &OrganizationRecordV1, n: u32) -> OrganizationRecordV1 {
    OrganizationRecordV1 {
        id: id(n),
        version: prior.version + 1,
        supersedes: Some(prior.id),
        ..prior.clone()
    }
}

/// P, Food, Coffee, Travel and Flights.
fn definitions() -> Vec<OrganizationRecordV1> {
    vec![
        define(P, OrganizationKind::Party, None, "Blue Bottle"),
        define(FOOD, OrganizationKind::Category, None, "Food"),
        define(COFFEE, OrganizationKind::Category, Some(FOOD), "Coffee"),
        define(TRAVEL, OrganizationKind::Tag, None, "Travel"),
        define(FLIGHTS, OrganizationKind::Tag, Some(TRAVEL), "Flights"),
    ]
}

fn value(
    field: OrganizationFieldV1,
    legs: &[u32],
    value: Option<u32>,
    action: OverrideActionV1,
) -> LegValueRequestV1 {
    LegValueRequestV1 {
        field,
        legs: legs.iter().map(|n| leg(*n)).collect(),
        value: value.map(id),
        action,
    }
}

fn category(legs: &[u32], to: Option<u32>, action: OverrideActionV1) -> OrganizationChangeV1 {
    OrganizationChangeV1 {
        records: vec![],
        values: vec![value(OrganizationFieldV1::Category, legs, to, action)],
    }
}

/// The committed commit after `state` that holds `records`.
fn commit(state: &DomainState, records: Vec<AuthoritativeRecordV1>) -> DomainCommitV1 {
    let seq = CommitSeq(state.through.0 + 1);
    let n = u32::try_from(seq.0).unwrap();
    DomainCommitV1 {
        seq,
        previous_digest: state.domain_digest,
        records,
        decisions: vec![],
        native_head_after: NativeHead {
            semantics_version: 1,
            checkpoint: Digest([0; 32]),
        },
        digest: Digest([u8::try_from(seq.0 % 256).unwrap(); 32]),
        command_result: CommandResultV1 {
            command_id: CommandId(id(9000 + n).0),
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
    }
}

/// An organized state and the history of organization commits after `base`.
struct Organized {
    base: DomainState,
    state: DomainState,
    history: Vec<DomainCommitV1>,
}

impl Organized {
    fn new() -> Self {
        let base = world();
        Self {
            state: base.clone(),
            base,
            history: vec![],
        }
    }
    /// P, Food, Coffee, Travel, Flights and Groceries committed.
    fn defined() -> Self {
        let mut organized = Self::new();
        let mut records = definitions();
        records.push(define(
            GROCERIES,
            OrganizationKind::Category,
            Some(FOOD),
            "Groceries",
        ));
        organized.change(&OrganizationChangeV1 {
            records,
            values: vec![],
        });
        organized
    }
    /// Preview `change`, commit its plan and replay it.
    fn change(&mut self, change: &OrganizationChangeV1) -> (DecisionPlan, Vec<LegId>) {
        let preview = OrganizationEngine::set(&self.state, change).unwrap();
        assert!(preview.plan.invalidations.is_empty());
        assert!(matches!(
            preview.plan.accounting,
            AccountingIntent::NoEffect(_)
        ));
        let next = commit(&self.state, preview.plan.records.clone());
        ReplayEngine::apply(&mut self.state, &next).unwrap();
        self.history.push(next);
        (preview.plan, preview.skipped)
    }
    /// A fresh replay of the first `commits` organization commits.
    fn through(&self, commits: usize) -> DomainState {
        let mut state = self.base.clone();
        for next in &self.history[..commits] {
            ReplayEngine::apply(&mut state, next).unwrap();
        }
        state
    }
    /// The engine refuses `change` with no plan.
    fn refuses(&self, change: &OrganizationChangeV1) {
        let preview = OrganizationEngine::set(&self.state, change);
        assert!(preview.is_err(), "accepted {change:?}");
    }
    /// Replay refuses a commit holding `records`, and the state stays at its
    /// prior head.
    fn replay_refuses(&self, records: Vec<AuthoritativeRecordV1>) {
        let mut copy = self.state.clone();
        assert!(
            ReplayEngine::apply(&mut copy, &commit(&self.state, records.clone())).is_err(),
            "replayed {records:?}"
        );
        assert_eq!(copy.through, self.state.through);
        assert_eq!(copy.domain_digest, self.state.domain_digest);
        assert_eq!(copy.records, self.state.records);
    }
    /// Both the engine and replay refuse a change that is exactly `records`.
    fn refuses_records(&self, records: Vec<OrganizationRecordV1>) {
        self.refuses(&OrganizationChangeV1 {
            records: records.clone(),
            values: vec![],
        });
        self.replay_refuses(
            records
                .into_iter()
                .map(AuthoritativeRecordV1::Organization)
                .collect(),
        );
    }
    fn definition(&self, n: u32) -> OrganizationRecordV1 {
        let Some(AuthoritativeRecordV1::Organization(o)) = self.state.records.get(&id(n)) else {
            panic!("definition {n} missing")
        };
        o.clone()
    }
    /// The defined state with alias A1 for P committed after it.
    fn aliased() -> Self {
        let mut organized = Self::defined();
        organized.change(&records(vec![alias(A1, P, &text("/aliases/0/text"))]));
        organized
    }
    /// The legs whose party `change` would change, without committing it.
    fn party_changes(&self, change: &OrganizationChangeV1) -> Vec<LegId> {
        OrganizationEngine::set(&self.state, change)
            .unwrap()
            .party_changes
    }
    /// Commit `records`, which no organization engine plans, and replay them.
    fn commit_records(&mut self, records: Vec<AuthoritativeRecordV1>) {
        let next = commit(&self.state, records);
        ReplayEngine::apply(&mut self.state, &next).unwrap();
        self.history.push(next);
    }
}

fn records(records: Vec<OrganizationRecordV1>) -> OrganizationChangeV1 {
    OrganizationChangeV1 {
        records,
        values: vec![],
    }
}

fn alias(n: u32, party: u32, text: &str) -> OrganizationRecordV1 {
    define(n, OrganizationKind::Alias, Some(party), text)
}

/// A source revision of `observation`, with ID `n`, that also reports
/// `category`, and its parse product.
fn revision(
    state: &DomainState,
    observation: u32,
    n: u32,
    category: &str,
) -> Vec<AuthoritativeRecordV1> {
    let Some(AuthoritativeRecordV1::ObservationVersion(prior)) =
        state.records.get(&id(observation))
    else {
        panic!("observation {observation} missing")
    };
    let mut revised = prior.clone();
    revised.id = id(n);
    revised.prior_version = Some(prior.id);
    revised
        .reported_fields
        .retain(|field| field.name != "category");
    revised.reported_fields.push(reported("category", category));
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get(&id(observation + 1))
    else {
        panic!("parse product {} missing", observation + 1)
    };
    let mut product = product.clone();
    product.id = id(n + 1);
    product.product.source_version_id = revised.id;
    vec![
        AuthoritativeRecordV1::ObservationVersion(revised),
        AuthoritativeRecordV1::AddressedParseProduct(product),
    ]
}

/// The resolved `field` on `purchase`'s leg.
fn resolved(state: &DomainState, purchase: u32, field: OrganizationFieldV1) -> ResolvedFieldV1 {
    OrganizationEngine::resolve(state, leg(purchase), field).unwrap()
}

/// The provider tier's answer for `field` on `purchase`'s leg.
fn provider(state: &DomainState, purchase: u32, field: OrganizationFieldV1) -> ResolvedFieldV1 {
    OrganizationEngine::provider(state, leg(purchase), field).unwrap()
}

fn sources(ids: &[RecordId]) -> Vec<RecordId> {
    let mut sources = ids.to_vec();
    sources.sort();
    sources
}

/// A definition from the provider tier, read from `observation` through
/// `alias`.
fn aliased_party(party: u32, observation: u32, alias: u32) -> ResolvedFieldV1 {
    ResolvedFieldV1::Known {
        tier: OrganizationTierV1::Provider,
        value: OrganizationValueV1::Reference(id(party)),
        sources: sources(&[id(observation), id(alias)]),
    }
}

/// Text `observation` reports, from the provider tier.
fn reported_value(text: &str, observation: u32) -> ResolvedFieldV1 {
    ResolvedFieldV1::Known {
        tier: OrganizationTierV1::Provider,
        value: OrganizationValueV1::Text(text.into()),
        sources: vec![id(observation)],
    }
}

/// The owner's current value for `field` on `purchase`'s leg, from the owner
/// tier, with its assignment as the source.
fn owned(state: &DomainState, purchase: u32, field: OrganizationFieldV1) -> ResolvedFieldV1 {
    let held = state.owner_value(leg(purchase), field).unwrap();
    ResolvedFieldV1::Known {
        tier: OrganizationTierV1::Owner,
        value: OrganizationValueV1::Reference(held.value),
        sources: vec![held.record],
    }
}

/// The bytes of O1, O2 and their parse products.
fn source_bytes(state: &DomainState) -> Vec<Vec<u8>> {
    [O1, O1 + 1, O2, O2 + 1]
        .iter()
        .map(|n| serde_json::to_vec(&state.records[&id(*n)]).unwrap())
        .collect()
}

/// The owner's current definition for `field` on `purchase`'s leg, if any.
fn current(state: &DomainState, purchase: u32, field: OrganizationFieldV1) -> Option<RecordId> {
    state.owner_value(leg(purchase), field).map(|v| v.value)
}

/// The one assignment `plan` holds.
fn assignment(plan: &DecisionPlan) -> OrganizationAssignmentV1 {
    let [AuthoritativeRecordV1::OrganizationAssignment(a)] = plan.records.as_slice() else {
        panic!("one assignment: {:?}", plan.records)
    };
    a.clone()
}

#[test]
fn cur_019_owner_meaning_replays_identically() {
    let mut organized = Organized::new();
    organized.change(&OrganizationChangeV1 {
        records: definitions(),
        values: vec![],
    });
    let mut note = define(N, OrganizationKind::Note, None, "Team offsite");
    note.targets = vec![RecordId(leg(L1).0)];
    let fields = [
        (OrganizationFieldV1::Party, P),
        (OrganizationFieldV1::Category, COFFEE),
        (OrganizationFieldV1::Tag, TRAVEL),
    ];
    let set = |(field, to)| value(field, &[L1], Some(to), OverrideActionV1::Replace);
    let (plan, skipped) = organized.change(&OrganizationChangeV1 {
        records: vec![note],
        values: fields.map(set).to_vec(),
    });
    assert!(skipped.is_empty());
    assert_eq!(plan.records.len(), 4);

    let fresh = organized.through(organized.history.len());
    for state in [&organized.state, &fresh] {
        for (field, expected) in fields {
            let held = state.owner_value(leg(L1), field).unwrap();
            assert_eq!(held.value, id(expected));
            assert_eq!(held.version, 1);
            assert_eq!(current(state, L2, field), None);
        }
    }
    assert_eq!(fresh.records, organized.state.records);
    // Every committed record keeps its bytes in the replayed state.
    for next in &organized.history {
        for record in &next.records {
            let value = serde_json::to_value(record).unwrap();
            let key: RecordId = serde_json::from_value(value["value"]["id"].clone()).unwrap();
            assert_eq!(
                serde_json::to_vec(&fresh.records[&key]).unwrap(),
                serde_json::to_vec(record).unwrap()
            );
        }
    }
}

#[test]
fn cur_019_revisions_keep_prior_meaning() {
    let mut organized = Organized::defined();
    let (coffee, _) = organized.change(&category(&[L1], Some(COFFEE), OverrideActionV1::Replace));
    let v1 = assignment(&coffee);
    let earlier = organized.history.len();
    let (groceries, _) =
        organized.change(&category(&[L1], Some(GROCERIES), OverrideActionV1::Replace));
    let v2 = assignment(&groceries);
    assert_eq!(v2.version, 2);
    assert_eq!(v2.supersedes, Some(v1.id));
    assert_eq!(v2.legs, vec![leg(L1)]);

    let field = OrganizationFieldV1::Category;
    assert_eq!(
        organized.through(earlier).owner_value(leg(L1), field),
        Some(OwnerValueV1 {
            record: v1.id,
            version: 1,
            value: id(COFFEE)
        })
    );
    assert_eq!(
        organized
            .through(organized.history.len())
            .owner_value(leg(L1), field),
        Some(OwnerValueV1 {
            record: v2.id,
            version: 2,
            value: id(GROCERIES)
        })
    );
    // Version 1 keeps its bytes.
    let v1_record = AuthoritativeRecordV1::OrganizationAssignment(v1.clone());
    assert_eq!(
        serde_json::to_vec(&organized.state.records[&v1.id]).unwrap(),
        serde_json::to_vec(&v1_record).unwrap()
    );
}

#[test]
fn cur_019_revision_guards() {
    let mut organized = Organized::defined();
    let (coffee, _) = organized.change(&category(&[L1], Some(COFFEE), OverrideActionV1::Replace));
    let coffee_v1 = organized.definition(COFFEE);
    organized.change(&OrganizationChangeV1 {
        records: vec![revise(&coffee_v1, 700)],
        values: vec![],
    });

    // A second successor of version 1.
    organized.refuses_records(vec![revise(&coffee_v1, 701)]);
    // Version 3 after version 1.
    let travel = organized.definition(TRAVEL);
    organized.refuses_records(vec![OrganizationRecordV1 {
        version: 3,
        ..revise(&travel, 702)
    }]);
    // A change of kind.
    organized.refuses_records(vec![OrganizationRecordV1 {
        kind: OrganizationKind::Category,
        ..revise(&travel, 703)
    }]);
    // A category whose parent chain returns to itself: Food under Coffee,
    // which is under Food.
    let food = organized.definition(FOOD);
    organized.refuses_records(vec![OrganizationRecordV1 {
        parent: Some(id(COFFEE)),
        ..revise(&food, 704)
    }]);
    organized.refuses_records(vec![define(
        705,
        OrganizationKind::Category,
        Some(705),
        "Itself",
    )]);
    // A missing parent.
    organized.refuses_records(vec![define(
        706,
        OrganizationKind::Category,
        Some(9999),
        "Orphan",
    )]);

    // A target that is a recognition ID, a posting ID or a missing leg.
    for (n, target) in [(707, L1 + 3), (708, L1 + 5), (709, 9998)] {
        let mut note = define(n, OrganizationKind::Note, None, "Not a leg");
        note.targets = vec![id(target)];
        organized.refuses_records(vec![note]);
        organized.refuses(&OrganizationChangeV1 {
            records: vec![],
            values: vec![LegValueRequestV1 {
                field: OrganizationFieldV1::Category,
                legs: vec![LegId(id(target).0)],
                value: Some(id(GROCERIES)),
                action: OverrideActionV1::Replace,
            }],
        });
        let mut forged = assignment(&coffee);
        forged.id = id(n + 100);
        forged.legs = vec![LegId(id(target).0)];
        organized.replay_refuses(vec![AuthoritativeRecordV1::OrganizationAssignment(forged)]);
    }

    // Replay also refuses a second successor of a leg value, a second first
    // version on the same leg and field, and a Preserve over a value.
    let v1 = assignment(&coffee);
    let (groceries, _) =
        organized.change(&category(&[L1], Some(GROCERIES), OverrideActionV1::Replace));
    let v2 = assignment(&groceries);
    for forged in [
        OrganizationAssignmentV1 {
            id: id(710),
            version: 2,
            supersedes: Some(v1.id),
            ..v1.clone()
        },
        OrganizationAssignmentV1 {
            id: id(711),
            ..v1.clone()
        },
        OrganizationAssignmentV1 {
            id: id(712),
            version: 3,
            supersedes: Some(v2.id),
            override_action: OverrideActionV1::Preserve,
            ..v1.clone()
        },
    ] {
        organized.replay_refuses(vec![AuthoritativeRecordV1::OrganizationAssignment(forged)]);
    }
}

#[test]
fn cur_019_override_actions() {
    let field = OrganizationFieldV1::Category;
    let mut organized = Organized::defined();
    let (coffee, _) = organized.change(&category(&[L1], Some(COFFEE), OverrideActionV1::Replace));
    let coffee = assignment(&coffee);

    // Preserve sets only the leg without a value and lists the other.
    let (preserved, skipped) = organized.change(&category(
        &[L1, L2],
        Some(GROCERIES),
        OverrideActionV1::Preserve,
    ));
    assert_eq!(skipped, vec![leg(L1)]);
    let preserved = assignment(&preserved);
    assert_eq!(
        (
            preserved.legs.clone(),
            preserved.version,
            preserved.supersedes
        ),
        (vec![leg(L2)], 1, None)
    );
    assert_eq!(current(&organized.state, L1, field), Some(id(COFFEE)));
    assert_eq!(current(&organized.state, L2, field), Some(id(GROCERIES)));

    // Replace supersedes the current value.
    let (replaced, skipped) =
        organized.change(&category(&[L1], Some(GROCERIES), OverrideActionV1::Replace));
    assert!(skipped.is_empty());
    let replaced = assignment(&replaced);
    assert_eq!(
        (replaced.version, replaced.supersedes),
        (2, Some(coffee.id))
    );
    assert_eq!(current(&organized.state, L1, field), Some(id(GROCERIES)));
    let before_clear = organized.history.len();

    // Clear releases the value with a new version.
    let (cleared, _) = organized.change(&category(&[L1], None, OverrideActionV1::Clear));
    let cleared = assignment(&cleared);
    assert_eq!(
        (cleared.version, cleared.supersedes, cleared.value),
        (3, Some(replaced.id), None)
    );
    assert_eq!(current(&organized.state, L1, field), None);
    assert_eq!(current(&organized.state, L2, field), Some(id(GROCERIES)));
    let earlier = organized.through(before_clear);
    assert_eq!(current(&earlier, L1, field), Some(id(GROCERIES)));
    assert_eq!(
        current(&organized.through(organized.history.len()), L1, field),
        None
    );

    // Clearing a leg with no value has nothing to release. A value set after
    // a clear is the next version.
    organized.refuses(&category(&[L1], None, OverrideActionV1::Clear));
    let (again, skipped) =
        organized.change(&category(&[L1], Some(COFFEE), OverrideActionV1::Preserve));
    assert!(skipped.is_empty());
    assert_eq!(assignment(&again).supersedes, Some(cleared.id));
    assert_eq!(current(&organized.state, L1, field), Some(id(COFFEE)));
}

#[test]
fn cur_019_1_processor_change_maps_stable_merchant() {
    let party = OrganizationFieldV1::Party;
    let mut organized = Organized::aliased();
    let before = source_bytes(&organized.state);
    assert_eq!(
        resolved(&organized.state, L1, party),
        aliased_party(P, O1, A1)
    );
    assert_eq!(
        resolved(&organized.state, L2, party),
        ResolvedFieldV1::Unknown
    );

    // The processor's new description reaches P through alias A2.
    let a2 = records(vec![alias(A2, P, &text("/aliases/1/text"))]);
    assert_eq!(organized.party_changes(&a2), vec![leg(L2)]);
    organized.change(&a2);
    assert_eq!(
        resolved(&organized.state, L1, party),
        aliased_party(P, O1, A1)
    );
    assert_eq!(
        resolved(&organized.state, L2, party),
        aliased_party(P, O2, A2)
    );
    assert_eq!(source_bytes(&organized.state), before);
}

#[test]
fn cur_019_1_category_exception_survives_import() {
    let field = OrganizationFieldV1::Category;
    let mut organized = Organized::aliased();
    assert_eq!(
        provider(&organized.state, L1, field),
        reported_value(&text("/purchases/0/category"), O1)
    );
    organized.change(&category(&[L1], Some(COFFEE), OverrideActionV1::Replace));
    let coffee = owned(&organized.state, L1, field);
    assert!(matches!(
        &coffee,
        ResolvedFieldV1::Known { value: OrganizationValueV1::Reference(v), .. } if *v == id(COFFEE)
    ));
    assert_eq!(resolved(&organized.state, L1, field), coffee);
    let before = source_bytes(&organized.state);

    // A source revision of O1 changes only the provider proposal.
    let dining = text("/revision/category");
    organized.commit_records(revision(&organized.state, O1, O1_REVISED, &dining));
    assert_eq!(resolved(&organized.state, L1, field), coffee);
    assert_eq!(
        provider(&organized.state, L1, field),
        reported_value(&dining, O1_REVISED)
    );
    assert_eq!(source_bytes(&organized.state), before);

    // Releasing the owner value shows the provider proposal.
    organized.change(&category(&[L1], None, OverrideActionV1::Clear));
    assert_eq!(
        resolved(&organized.state, L1, field),
        reported_value(&dining, O1_REVISED)
    );
    assert_eq!(
        resolved(&organized.state, L1, OrganizationFieldV1::Party),
        aliased_party(P, O1_REVISED, A1)
    );
}

#[test]
fn cur_019_precedence_owner_provider_unknown() {
    let mut organized = Organized::aliased();
    let fields = [
        OrganizationFieldV1::Party,
        OrganizationFieldV1::Category,
        OrganizationFieldV1::Tag,
    ];
    // L2's description matches no alias and O2 reports no category. No
    // source reports a tag.
    for field in fields {
        assert_eq!(
            resolved(&organized.state, L2, field),
            ResolvedFieldV1::Unknown
        );
    }
    assert_eq!(
        resolved(&organized.state, L1, OrganizationFieldV1::Tag),
        ResolvedFieldV1::Unknown
    );
    assert_eq!(
        resolved(&organized.state, L1, OrganizationFieldV1::Party),
        aliased_party(P, O1, A1)
    );
    assert_eq!(
        resolved(&organized.state, L1, OrganizationFieldV1::Category),
        reported_value(&text("/purchases/0/category"), O1)
    );

    // Owner values win over the provider proposal, which they leave alone.
    let set = |field, purchase, to| value(field, &[purchase], Some(to), OverrideActionV1::Replace);
    organized.change(&OrganizationChangeV1 {
        records: vec![define(Q, OrganizationKind::Party, None, "Ritual")],
        values: vec![
            set(OrganizationFieldV1::Party, L1, Q),
            set(OrganizationFieldV1::Category, L1, GROCERIES),
            set(OrganizationFieldV1::Tag, L2, TRAVEL),
        ],
    });
    for (purchase, field) in [
        (L1, OrganizationFieldV1::Party),
        (L1, OrganizationFieldV1::Category),
        (L2, OrganizationFieldV1::Tag),
    ] {
        assert_eq!(
            resolved(&organized.state, purchase, field),
            owned(&organized.state, purchase, field)
        );
    }
    let party = organized
        .state
        .owner_value(leg(L1), OrganizationFieldV1::Party)
        .unwrap();
    assert_eq!(
        resolved(&organized.state, L1, OrganizationFieldV1::Party),
        ResolvedFieldV1::Known {
            tier: OrganizationTierV1::Owner,
            value: OrganizationValueV1::Reference(id(Q)),
            sources: vec![party.record],
        }
    );
    assert_eq!(
        provider(&organized.state, L1, OrganizationFieldV1::Party),
        aliased_party(P, O1, A1)
    );
    assert_eq!(
        resolved(&organized.state, L2, OrganizationFieldV1::Party),
        ResolvedFieldV1::Unknown
    );

    // The resolver refuses an ID that is not a leg.
    for target in [L1 + 3, 9998] {
        assert!(
            OrganizationEngine::resolve(
                &organized.state,
                LegId(id(target).0),
                OrganizationFieldV1::Party
            )
            .is_err()
        );
    }
}

#[test]
fn cur_019_alias_conflict_is_explicit() {
    let party = OrganizationFieldV1::Party;
    let mut organized = Organized::aliased();
    let a1_text = text("/aliases/0/text");

    // An alias that differs only in letter case matches nothing.
    let lower = records(vec![
        define(Q, OrganizationKind::Party, None, "Ritual"),
        alias(A4, Q, &a1_text.to_lowercase()),
    ]);
    assert!(organized.party_changes(&lower).is_empty());
    organized.change(&lower);
    assert_eq!(
        resolved(&organized.state, L1, party),
        aliased_party(P, O1, A1)
    );

    // A second party's alias with the exact text conflicts with A1.
    let exact = records(vec![alias(A3, Q, &a1_text)]);
    assert_eq!(organized.party_changes(&exact), vec![leg(L1)]);
    organized.change(&exact);
    let conflict = ResolvedFieldV1::Conflict {
        tier: OrganizationTierV1::Provider,
        sources: sources(&[id(O1), id(A1), id(A3)]),
    };
    assert_eq!(resolved(&organized.state, L1, party), conflict);
    assert_eq!(
        resolved(&organized.state, L2, party),
        ResolvedFieldV1::Unknown
    );

    // Revising A3 to other text ends the conflict.
    let revised = records(vec![OrganizationRecordV1 {
        text: "RITUAL COFFEE".into(),
        ..revise(&organized.definition(A3), 653)
    }]);
    assert_eq!(organized.party_changes(&revised), vec![leg(L1)]);
    organized.change(&revised);
    assert_eq!(
        resolved(&organized.state, L1, party),
        aliased_party(P, O1, A1)
    );
}

#[test]
fn cur_019_s6_journey_rebuilds_identical_meaning() {
    let mut organized = Organized::aliased();
    organized.change(&records(vec![alias(A2, P, &text("/aliases/1/text"))]));
    assert_eq!(text("/owner_category/category"), "Coffee");
    organized.change(&category(&[L1], Some(COFFEE), OverrideActionV1::Replace));
    organized.commit_records(revision(
        &organized.state,
        O1,
        O1_REVISED,
        &text("/revision/category"),
    ));
    organized.change(&category(&[L1], None, OverrideActionV1::Clear));

    let fields = [
        OrganizationFieldV1::Party,
        OrganizationFieldV1::Category,
        OrganizationFieldV1::Tag,
    ];
    // Every leg's resolved value for every field.
    let meaning = |state: &DomainState| -> Vec<(LegId, OrganizationFieldV1, ResolvedFieldV1)> {
        let legs = state.records.values().filter_map(|record| match record {
            AuthoritativeRecordV1::FinancialLeg(l) => Some(l.id),
            _ => None,
        });
        legs.flat_map(|leg| {
            fields.map(|field| {
                let resolved = OrganizationEngine::resolve(state, leg, field).unwrap();
                (leg, field, resolved)
            })
        })
        .collect()
    };
    for commits in 0..=organized.history.len() {
        let first = meaning(&organized.through(commits));
        assert_eq!(first.len(), 6);
        assert_eq!(first, meaning(&organized.through(commits)));
    }
    let last = meaning(&organized.through(organized.history.len()));
    assert_eq!(last, meaning(&organized.state));
    let dining = text("/revision/category");
    let party = OrganizationFieldV1::Party;
    let category = OrganizationFieldV1::Category;
    for (purchase, field, expected) in [
        (L1, party, aliased_party(P, O1_REVISED, A1)),
        (L2, party, aliased_party(P, O2, A2)),
        (L1, category, reported_value(&dining, O1_REVISED)),
        (L2, category, ResolvedFieldV1::Unknown),
    ] {
        let found = (leg(purchase), field, expected);
        assert!(last.contains(&found), "{found:?} not in {last:?}");
    }
}
