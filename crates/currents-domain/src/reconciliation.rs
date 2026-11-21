//! Pure, bounded comparison of caller-selected Activity lines, not claim admission.
use crate::activity_predicate::ActivityPredicateV1;
use currents_types::{AccountId, Amount, Date, DateBasis, EngineError, RecordId, TemporalValue};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceLineIdV1 {
    pub observation: RecordId,
    pub facet: RecordId,
}

#[derive(Clone, Debug)]
pub struct ActivityLineV1 {
    pub identity: EvidenceLineIdV1,
    pub account: AccountId,
    pub amount: Amount,
    pub effective: TemporalValue,
    pub posting: TemporalValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityDifferenceV1 {
    MissingAllocation(EvidenceLineIdV1),
    UnexpectedAllocation(EvidenceLineIdV1),
    UnsupportedMultiplicity(EvidenceLineIdV1),
    DifferentTerms(EvidenceLineIdV1),
}

type Terms = (AccountId, Date, Amount);

#[derive(Default)]
struct LineGroup {
    in_scope: Option<Terms>,
    seen: bool,
    repeated: bool,
}

fn group_lines(
    predicate: &ActivityPredicateV1<'_>,
    rows: &[ActivityLineV1],
) -> Result<BTreeMap<EvidenceLineIdV1, LineGroup>, EngineError> {
    let mut groups = BTreeMap::<_, LineGroup>::new();
    for row in rows {
        let group = groups.entry(row.identity).or_default();
        group.repeated |= group.seen;
        group.seen = true;
        if let Some(date) =
            predicate.posting_date_in_scope(row.account, &row.effective, &row.posting)?
        {
            group.in_scope = Some((row.account, date, row.amount.normalized()?));
        }
    }
    Ok(groups)
}

/// Compare exact source identities and signed terms for a bounded one-to-one
/// candidate. Repeat identities touching the comparison (including split
/// allocations straddling its bounds) are unsupported, never netted. Callers
/// must include all sibling rows for relevant identities; do not filter these
/// rows by date or account before comparison.
///
/// An empty difference list is not a reconciliation claim: independent source
/// completeness, active recognized-leg selection, reference qualification,
/// exact retained basis, and independent boundary agreement are all outside
/// this pure function. Neither input is authenticated by the comparison.
pub fn compare_activity_allocations(
    accounts: &[AccountId],
    basis: DateBasis,
    start: Date,
    end: Date,
    sources: &[ActivityLineV1],
    allocations: &[ActivityLineV1],
) -> Result<Vec<ActivityDifferenceV1>, EngineError> {
    use ActivityDifferenceV1::*;
    let predicate = ActivityPredicateV1::new(accounts, basis, start, end)?;
    let sources = group_lines(&predicate, sources)?;
    let allocations = group_lines(&predicate, allocations)?;
    let mut differences = Vec::new();
    for id in sources
        .keys()
        .chain(allocations.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let source = sources.get(&id);
        let allocation = allocations.get(&id);
        let expected = source.and_then(|g| g.in_scope);
        let actual = allocation.and_then(|g| g.in_scope);
        if expected.is_none() && actual.is_none() {
            continue;
        }
        if source.is_some_and(|g| g.repeated) || allocation.is_some_and(|g| g.repeated) {
            differences.push(UnsupportedMultiplicity(id));
            continue;
        }
        match (expected, actual) {
            (Some(_), None) => differences.push(MissingAllocation(id)),
            (None, Some(_)) => differences.push(UnexpectedAllocation(id)),
            (Some(expected), Some(actual)) if expected != actual => {
                differences.push(DifferentTerms(id))
            }
            _ => {}
        }
    }
    Ok(differences)
}
