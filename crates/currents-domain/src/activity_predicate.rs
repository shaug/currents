//! Exact scope selection shared by the domain Activity comparison.
use currents_types::{AccountId, Date, DateBasis, EngineError, TemporalValue};

/// Pure posting-membership descriptor. This does not certify a claim producer,
/// interpret referenced record IDs, or constitute a persisted dependency index.
pub(crate) struct ActivityPredicateV1<'a> {
    accounts: &'a [AccountId],
    basis: DateBasis,
    start: Date,
    end: Date,
}

impl<'a> ActivityPredicateV1<'a> {
    pub(crate) fn new(
        accounts: &'a [AccountId],
        basis: DateBasis,
        start: Date,
        end: Date,
    ) -> Result<Self, EngineError> {
        if accounts.is_empty() || start > end {
            return Err(EngineError::version());
        }
        Ok(Self {
            accounts,
            basis,
            start,
            end,
        })
    }

    pub(crate) fn posting_date_in_scope(
        &self,
        account: AccountId,
        effective: &TemporalValue,
        posting: &TemporalValue,
    ) -> Result<Option<Date>, EngineError> {
        if !self.accounts.contains(&account) {
            return Ok(None);
        }
        let civil = civil_date(self.basis, effective, posting).ok_or_else(EngineError::version)?;
        Ok(within(self.start, self.end, civil).then_some(civil))
    }
}

/// The date `basis` selects, when it is a civil date.
pub(crate) fn civil_date(
    basis: DateBasis,
    effective: &TemporalValue,
    posting: &TemporalValue,
) -> Option<Date> {
    let date = match basis {
        DateBasis::Effective => effective,
        DateBasis::Posting => posting,
    };
    match date {
        TemporalValue::Date { civil, .. } => Some(*civil),
        _ => None,
    }
}

/// `day` falls inside the inclusive interval from `start` to `end`.
pub(crate) fn within(start: Date, end: Date, day: Date) -> bool {
    start <= day && day <= end
}
