//! CUR-019 versioned owner organization on economic legs, using synthetic
//! identities and the exact public interfaces.
use currents_domain::{
    ReplayEngine,
    organization::{LegValueRequestV1, OrganizationChangeV1, OrganizationEngine},
    recognition::RecognitionEngine,
};
use currents_types::*;
#[path = "support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
use fixture::*;

/// The expense account E1 that both purchases debit.
const E1: u32 = 30;
/// Purchase L1 (96.00 USD on 2026-11-03) and L2 (40.00 USD on 2026-11-04),
/// both on account K, the fixture's external account. Each leg is its
/// purchase's ID plus one, its recognition plus three and its first posting
/// plus five.
const L1: u32 = 8050;
const L2: u32 = 8100;
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

/// A posted purchase of `value` on `day` whose evidence starts at `evidence_id`.
fn purchase(state: &mut DomainState, n: u32, evidence_id: u32, value: i128, day: &str) {
    let source = fixture::evidence(state, evidence_id, value, None);
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
    purchase(&mut state, L1, 8000, -96, "2026-11-03");
    purchase(&mut state, L2, 8010, -40, "2026-11-04");
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
