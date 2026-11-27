//! Records that depend on a segment replacement or a correction. A claim in
//! the changed scope is invalidated in the same commit, organization on a
//! retired leg needs the owner's decision, and the other dependent families
//! still refuse the change.
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

/// An organization record that targets a leg the change retires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationQuestionV1 {
    pub record: RecordId,
    pub leg: LegId,
}

/// A posting date: a civil day, or another temporal value kept whole.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Day {
    Civil(Date),
    Other(String),
}
fn day(value: &TemporalValue) -> Result<Day, EngineError> {
    match value {
        TemporalValue::Date { civil, .. } => Ok(Day::Civil(*civil)),
        other => Ok(Day::Other(
            serde_json::to_string(other).map_err(|_| EngineError::invalid())?,
        )),
    }
}

/// The net posting change of one plan, and the identities it retires.
pub(crate) struct Change {
    /// Net amount by account, unit, effective date and posting date.
    net: BTreeMap<(AccountId, UnitId, Day, Day), Amount>,
    /// Every account a reversed or added posting touches.
    accounts: BTreeSet<AccountId>,
    retired: BTreeSet<RecordId>,
    retired_legs: BTreeSet<LegId>,
}
impl Change {
    /// Each correction reverses its active recognition and adds its
    /// replacements. The reversed recognitions and their postings retire,
    /// with `legs` and `records`.
    pub(crate) fn new(
        state: &DomainState,
        corrections: &[CorrectionV1],
        legs: impl IntoIterator<Item = LegId>,
        records: impl IntoIterator<Item = RecordId>,
    ) -> Result<Self, EngineError> {
        let mut change = Self {
            net: BTreeMap::new(),
            accounts: BTreeSet::new(),
            retired: records.into_iter().collect(),
            retired_legs: legs.into_iter().collect(),
        };
        change
            .retired
            .extend(change.retired_legs.iter().map(|leg| RecordId(leg.0)));
        for correction in corrections {
            let key = RecordId(correction.active_recognition.0);
            let Some(AuthoritativeRecordV1::Recognition(original)) = state.records.get(&key) else {
                return Err(EngineError::invalid());
            };
            change.retired.insert(key);
            for posting in &original.postings {
                change.retired.insert(posting.id);
                change.add(posting, posting.amount.checked_neg()?)?;
            }
            for posting in correction.replacement.iter().flat_map(|r| &r.postings) {
                change.add(posting, posting.amount)?;
            }
        }
        Ok(change)
    }
    fn add(&mut self, posting: &PostingV1, amount: Amount) -> Result<(), EngineError> {
        self.accounts.insert(posting.account_id);
        let key = (
            posting.account_id,
            amount.unit_id,
            day(&posting.effective)?,
            day(&posting.posting)?,
        );
        let total = match self.net.get(&key) {
            Some(total) => total.checked_add(&amount)?,
            None => amount,
        };
        self.net.insert(key, total);
        Ok(())
    }
    /// Every account whose net change is nonzero on some date, in ID order.
    pub(crate) fn changed_accounts(&self) -> Vec<AccountId> {
        let changed: BTreeSet<_> = self
            .net
            .iter()
            .filter(|(_, amount)| amount.coefficient != 0)
            .map(|((account, ..), _)| *account)
            .collect();
        changed.into_iter().collect()
    }
    /// The claim's account and date predicate sees a nonzero net change.
    fn in_scope(&self, claim: &ReconciliationClaimV1) -> Result<bool, EngineError> {
        let within = |day: &Day| match day {
            Day::Civil(day) => {
                claim.cutoffs.start.is_none_or(|start| start <= *day)
                    && claim.cutoffs.end.is_none_or(|end| *day <= end)
            }
            // An instant is never proven outside the range.
            Day::Other(_) => true,
        };
        let mut totals = BTreeMap::<_, Amount>::new();
        for ((account, unit, effective, posting), amount) in &self.net {
            let day = match claim.cutoffs.basis {
                DateBasis::Effective => effective,
                DateBasis::Posting => posting,
            };
            if !claim.scope.contains(account) || !within(day) {
                continue;
            }
            let key = (*account, *unit, day.clone());
            let total = match totals.get(&key) {
                Some(total) => total.checked_add(amount)?,
                None => *amount,
            };
            totals.insert(key, total);
        }
        Ok(totals.values().any(|total| total.coefficient != 0))
    }
    /// The claim's explicit dependencies name a retired identity.
    fn named(&self, claim: &ReconciliationClaimV1) -> bool {
        claim
            .evidence_dependencies
            .iter()
            .chain(&claim.decision_dependencies)
            .chain(&claim.exclusions)
            .chain(
                claim
                    .matched_allocations
                    .iter()
                    .flat_map(|a| [&a.observation, &a.facet]),
            )
            .any(|id| self.retired.contains(id))
    }
}

pub(crate) enum Dependents {
    /// The claims the change invalidates, in record order.
    Claims(Vec<RecordId>),
    NeedsOrganizationDecision(Vec<OrganizationQuestionV1>),
}

/// Assess every dependent record against one change. Review records,
/// forecast runs, occurrence matches, allocations and associations that name
/// an affected account or a retired identity refuse the change, until their
/// owners define correction rules. So does organization on a retired identity
/// other than a leg.
pub(crate) fn assess(state: &DomainState, change: &Change) -> Result<Dependents, EngineError> {
    let names: BTreeSet<String> = change
        .accounts
        .iter()
        .map(|a| a.0)
        .chain(change.retired.iter().map(|r| r.0))
        .map(|id| id.to_string())
        .collect();
    let mut claims = Vec::new();
    let mut questions = Vec::new();
    for (key, record) in &state.records {
        match record {
            AuthoritativeRecordV1::Review(_)
            | AuthoritativeRecordV1::ForecastRun(_)
            | AuthoritativeRecordV1::OccurrenceMatch(_)
            | AuthoritativeRecordV1::Allocation(_)
            | AuthoritativeRecordV1::Association(_) => {
                if names_any(&serialized(record)?, &names) {
                    return Err(EngineError::version());
                }
            }
            AuthoritativeRecordV1::Organization(organization) => {
                let legs: Vec<_> = change
                    .retired_legs
                    .iter()
                    .filter(|leg| organization.targets.contains(&RecordId(leg.0)))
                    .map(|&leg| OrganizationQuestionV1 {
                        record: organization.id,
                        leg,
                    })
                    .collect();
                let others: BTreeSet<String> = change
                    .retired
                    .iter()
                    .filter(|id| !change.retired_legs.contains(&LegId(id.0)))
                    .map(|id| id.0.to_string())
                    .collect();
                if names_any(&serialized(record)?, &others) {
                    return Err(EngineError::version());
                }
                questions.extend(legs);
            }
            AuthoritativeRecordV1::ReconciliationClaim(claim) => {
                if *key != claim.id {
                    return Err(EngineError::invalid());
                }
                if state.claim_status(claim.id) != Some(ReconciliationStatus::Invalidated)
                    && (change.named(claim) || change.in_scope(claim)?)
                {
                    claims.push(claim.id);
                }
            }
            _ => {}
        }
    }
    if questions.is_empty() {
        Ok(Dependents::Claims(claims))
    } else {
        Ok(Dependents::NeedsOrganizationDecision(questions))
    }
}

fn serialized(record: &AuthoritativeRecordV1) -> Result<serde_json::Value, EngineError> {
    serde_json::to_value(record).map_err(|_| EngineError::invalid())
}
fn names_any(value: &serde_json::Value, ids: &BTreeSet<String>) -> bool {
    match value {
        serde_json::Value::String(text) => ids.contains(text),
        serde_json::Value::Array(items) => items.iter().any(|v| names_any(v, ids)),
        serde_json::Value::Object(fields) => fields.values().any(|v| names_any(v, ids)),
        _ => false,
    }
}
