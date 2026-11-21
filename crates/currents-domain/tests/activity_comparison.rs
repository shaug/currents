//! Public Rust-domain C09 comparison contract, with synthetic input values.
use currents_domain::reconciliation::{
    ActivityDifferenceV1, ActivityLineV1, EvidenceLineIdV1, compare_activity_allocations,
};
use currents_types::{AccountId, Amount, DateBasis, RecordId, TemporalValue, UnitId};
use serde_json::json;

fn id(n: &str) -> RecordId {
    RecordId(format!("00000000-0000-0000-0000-{n:0>12}").parse().unwrap())
}

fn date(day: &str) -> TemporalValue {
    serde_json::from_value(json!({"kind":"Date","civil":day,"original_text":day})).unwrap()
}

#[test]
fn exact_line_comparison_does_not_net_or_substitute() {
    let day = date("2026-11-05");
    let TemporalValue::Date { civil, .. } = day else {
        unreachable!()
    };
    let account = AccountId(id("10").0);
    let line = |n, coefficient, scale| ActivityLineV1 {
        identity: EvidenceLineIdV1 {
            observation: id("20"),
            facet: id(n),
        },
        account,
        amount: Amount::new(coefficient, scale, UnitId(id("30").0)).unwrap(),
        effective: day.clone(),
        posting: day.clone(),
    };
    let positive = line("1", 100, 0);
    let negative = line("2", -100, 0);
    let same_value_other_line = line("3", 100, 0);
    let run = |sources: &[ActivityLineV1], allocations: &[ActivityLineV1]| {
        compare_activity_allocations(
            &[account],
            DateBasis::Posting,
            civil,
            civil,
            sources,
            allocations,
        )
        .unwrap()
    };
    assert_eq!(
        run(&[positive.clone(), negative.clone()], &[]),
        vec![
            ActivityDifferenceV1::MissingAllocation(positive.identity),
            ActivityDifferenceV1::MissingAllocation(negative.identity)
        ]
    );
    assert_eq!(
        run(
            std::slice::from_ref(&positive),
            std::slice::from_ref(&same_value_other_line)
        ),
        vec![
            ActivityDifferenceV1::MissingAllocation(positive.identity),
            ActivityDifferenceV1::UnexpectedAllocation(same_value_other_line.identity)
        ]
    );
    assert_eq!(
        run(
            std::slice::from_ref(&positive),
            &[line("1", 40, 0), line("1", 60, 0)]
        ),
        vec![ActivityDifferenceV1::UnsupportedMultiplicity(
            positive.identity
        )]
    );
    assert_eq!(
        run(std::slice::from_ref(&positive), &[line("1", -100, 0)]),
        vec![ActivityDifferenceV1::DifferentTerms(positive.identity)]
    );
    let mut outside_same_line = line("1", 100, 0);
    outside_same_line.posting = date("2026-11-06");
    assert_eq!(
        run(
            std::slice::from_ref(&positive),
            &[positive.clone(), outside_same_line]
        ),
        vec![ActivityDifferenceV1::UnsupportedMultiplicity(
            positive.identity
        )]
    );
    assert!(run(&[positive], &[line("1", 1_000, 1)]).is_empty());
}
