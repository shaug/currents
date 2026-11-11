//! Pure commitment decisions and explicit series retirement, never journal writes.
use crate::calendar::{CalendarEngine, nominal, validate};
use currents_types::*;
use std::collections::BTreeSet;

pub struct CommitmentEngine {
    calendar: CalendarEngine,
}
impl CommitmentEngine {
    pub fn new(calendar: CalendarEngine) -> Self {
        Self { calendar }
    }
    pub fn set(
        &self,
        state: &DomainState,
        commitment: &CommitmentV1,
    ) -> Result<DecisionPlan, EngineError> {
        self.calendar.expand(commitment, commitment.start.0, 1)?;
        let mut datasets = state.records.values().filter_map(|r| match r {
            AuthoritativeRecordV1::Dataset(d) => Some(d),
            _ => None,
        });
        let confirmed = datasets.next().is_some_and(|d| {
            d.household_timezone == SourceValue::Known(commitment.timezone.clone())
        });
        if !confirmed || datasets.next().is_some() {
            return Err(EngineError::invalid());
        }
        for leg in commitment
            .legs
            .iter()
            .chain(commitment.exceptions.iter().flat_map(|e| &e.legs))
        {
            let account = state
                .records
                .values()
                .find_map(|r| match r {
                    AuthoritativeRecordV1::AccountComponent(a)
                        if a.account_id == leg.account_id =>
                    {
                        Some(a)
                    }
                    _ => None,
                })
                .ok_or_else(EngineError::invalid)?;
            for amount in [
                &leg.amounts.nominal,
                &leg.amounts.low,
                &leg.amounts.base,
                &leg.amounts.high,
            ] {
                if matches!(amount, SourceValue::Known(amount) if amount.unit_id != account.unit_id)
                {
                    return Err(EngineError::invalid());
                }
            }
        }
        let id = RecordId(commitment.id.0);
        let prior = match state.records.get(&id) {
            Some(AuthoritativeRecordV1::Commitment(c)) => Some(c),
            Some(_) => return Err(EngineError::invalid()),
            None => None,
        };
        if let Some(previous) = prior {
            validate(previous)?;
        }
        validate_revision(prior, commitment)?;
        Ok(DecisionPlan {
            records: vec![AuthoritativeRecordV1::Commitment(commitment.clone())],
            invalidations: prior.map_or_else(Vec::new, |_| vec![id]),
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Commitment changes planning expectations only".into(),
            }),
        })
    }
    /// Expand each latest immutable series revision, removing superseded keys.
    /// Remaps preserve allocation identity for the later matching owner; they
    /// never add a second expectation alongside the target's own occurrence.
    pub fn expand_history(
        &self,
        history: &[CommitmentV1],
        start: Date,
        months: u8,
    ) -> Result<Vec<OccurrenceV1>, EngineError> {
        validate_history(history)?;
        let mut out = vec![];
        for (i, c) in history.iter().enumerate() {
            if history
                .get(i.checked_add(1).ok_or_else(EngineError::precision)?)
                .is_some_and(|next| next.series_generation == c.series_generation)
            {
                continue;
            }
            for o in self.calendar.expand(c, start, months)? {
                if resolve_validated(history, &o.key)? == Some(o.key.clone()) {
                    out.push(o);
                }
            }
        }
        out.sort_by_key(|o| (o.due_date.0, o.key.series_generation, o.key.ordinal));
        Ok(out)
    }
}
/// Resolve keys and their attached matches through all accepted transitions.
/// None is explicit retirement; no allocation amount is changed here.
pub fn resolve_occurrence(
    history: &[CommitmentV1],
    key: &OccurrenceKeyV1,
) -> Result<Option<OccurrenceKeyV1>, EngineError> {
    validate_history(history)?;
    resolve_validated(history, key)
}
fn resolve_validated(
    history: &[CommitmentV1],
    key: &OccurrenceKeyV1,
) -> Result<Option<OccurrenceKeyV1>, EngineError> {
    let origin = history
        .iter()
        .rev()
        .find(|c| c.id == key.commitment_id && c.series_generation == key.series_generation)
        .ok_or_else(EngineError::invalid)?;
    if occurrence_date(origin, key.ordinal.0)?.is_none() {
        return Err(EngineError::invalid());
    }
    let mut current = key.clone();
    let mut visited = BTreeSet::new();
    for c in history {
        if !visited.insert(c.series_generation) {
            continue;
        }
        if let Some(s) = &c.supersession
            && current.series_generation == s.prior_generation
            && current.ordinal >= s.from_ordinal
        {
            match s.remaps.iter().find(|r| r.old == current) {
                Some(OccurrenceRemapV1 {
                    new: Some(target), ..
                }) => current = target.clone(),
                _ => return Ok(None),
            }
        }
    }
    Ok(Some(current))
}
fn validate_history(history: &[CommitmentV1]) -> Result<(), EngineError> {
    if history.is_empty() {
        return Err(EngineError::invalid());
    }
    let mut prior = None;
    for c in history {
        validate(c)?;
        validate_revision(prior, c)?;
        prior = Some(c);
    }
    Ok(())
}
fn validate_revision(prior: Option<&CommitmentV1>, c: &CommitmentV1) -> Result<(), EngineError> {
    let Some(p) = prior else {
        return if c.version == 1 && c.series_generation == 1 && c.supersession.is_none() {
            Ok(())
        } else {
            Err(EngineError::invalid())
        };
    };
    if c.id != p.id
        || c.version
            != p.version
                .checked_add(1)
                .ok_or_else(EngineError::precision)?
    {
        return Err(EngineError::invalid());
    }
    if c.recurrence == p.recurrence {
        if c.series_generation != p.series_generation || c.supersession != p.supersession {
            return Err(EngineError::invalid());
        }
        return Ok(());
    }
    if c.series_generation
        != p.series_generation
            .checked_add(1)
            .ok_or_else(EngineError::precision)?
    {
        return Err(EngineError::invalid());
    }
    let s = c.supersession.as_ref().ok_or_else(EngineError::invalid)?;
    if s.prior_version != p.version
        || s.prior_generation != p.series_generation
        || s.remaps.len() > 4000
    {
        return Err(EngineError::invalid());
    }
    let cutoff = nominal(p, s.from_ordinal.0)?.ok_or_else(EngineError::invalid)?;
    if c.start.0 < cutoff || c.due_date.0 < c.start.0 {
        return Err(EngineError::invalid());
    }
    for exception in &p.exceptions {
        if exception.ordinal < s.from_ordinal
            && occurrence_date(p, exception.ordinal.0)?.is_some_and(|d| d >= c.start.0)
        {
            return Err(EngineError::invalid());
        }
    }
    let mut sources = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for map in &s.remaps {
        if map.old.commitment_id != c.id
            || map.old.series_generation != p.series_generation
            || map.old.ordinal < s.from_ordinal
            || !sources.insert(map.old.ordinal)
            || occurrence_date(p, map.old.ordinal.0)?.is_none()
        {
            return Err(EngineError::invalid());
        }
        if let Some(new) = &map.new
            && (new.commitment_id != c.id
                || new.series_generation != c.series_generation
                || !targets.insert(new.ordinal)
                || occurrence_date(c, new.ordinal.0)?.is_none())
        {
            return Err(EngineError::invalid());
        }
    }
    Ok(())
}

fn occurrence_date(c: &CommitmentV1, ordinal: u64) -> Result<Option<Date>, EngineError> {
    let Some(mut date) = nominal(c, ordinal)? else {
        return Ok(None);
    };
    if let Some(e) = c.exceptions.iter().find(|e| e.ordinal.0 == ordinal) {
        if matches!(
            e.action,
            PlanningExceptionActionV1::Skip | PlanningExceptionActionV1::Cancel
        ) {
            return Ok(None);
        }
        if let Some(d) = e.due_date {
            date = d.0;
        }
    }
    Ok((date >= c.start.0 && c.end.is_none_or(|end| date <= end.0)).then_some(date))
}
