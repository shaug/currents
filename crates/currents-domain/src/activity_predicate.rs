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
        let date = match self.basis {
            DateBasis::Effective => effective,
            DateBasis::Posting => posting,
        };
        let TemporalValue::Date { civil, .. } = date else {
            return Err(EngineError::version());
        };
        Ok((self.start <= *civil && *civil <= self.end).then_some(*civil))
    }
}
