//! Deterministic civil expansion from explicitly retained calendar evidence.
use currents_types::*;
use jiff::{Span, civil, civil::Weekday, tz::TimeZone};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const MAX_TIMEZONE_BYTES: usize = 1_048_576;
const MAX_ORDINAL: u64 = 7_400_000;
/// Every timestamp conversion requires an explicit gap/overlap policy.
#[derive(Clone, Copy, Debug)]
pub enum ConversionPolicy {
    Reject,
    Earlier,
    Later,
}
/// Holds verified bytes, not the process or OS timezone database.
pub struct CalendarEngine {
    zone: TimeZone,
    data: ForecastCalendarDataV1,
}
impl CalendarEngine {
    /// New snapshots must match the reviewed bundled IANA release exactly.
    pub fn new(zone: &str, version: &str, bytes: &[u8]) -> Result<Self, EngineError> {
        if Some(version) != jiff_tzdb::VERSION
            || jiff_tzdb::get(zone).is_none_or(|(_, pinned)| pinned != bytes)
        {
            return Err(EngineError::version());
        }
        Self::parse(zone, version, bytes)
    }
    /// Restore only a retained, already accepted snapshot; the bundled release
    /// is deliberately irrelevant to reproducing its saved rules.
    pub fn from_saved(data: &ForecastCalendarDataV1, bytes: &[u8]) -> Result<Self, EngineError> {
        let engine = Self::parse(&data.timezone, &data.timezone_data_version, bytes)?;
        if engine.data != *data {
            return Err(EngineError::invalid());
        }
        Ok(engine)
    }
    fn parse(zone: &str, version: &str, bytes: &[u8]) -> Result<Self, EngineError> {
        if bytes.is_empty()
            || bytes.len() > MAX_TIMEZONE_BYTES
            || version.is_empty()
            || zone.is_empty()
        {
            return Err(EngineError::invalid());
        }
        let zone_value = TimeZone::tzif(zone, bytes).map_err(|_| EngineError::invalid())?;
        Ok(Self {
            zone: zone_value,
            data: ForecastCalendarDataV1 {
                timezone: zone.into(),
                timezone_data_version: version.into(),
                calendar_version: 1,
                holidays: vec![],
                timezone_data: ContentReferenceV1 {
                    digest: Digest(Sha256::digest(bytes).into()),
                    byte_count: CommitSeq(
                        u64::try_from(bytes.len()).map_err(|_| EngineError::precision())?,
                    ),
                    media_type: "application/tzif".into(),
                },
            },
        })
    }
    pub fn calendar_data(&self) -> ForecastCalendarDataV1 {
        self.data.clone()
    }
    /// Resolve an explicitly supplied local datetime. Civil due dates never call this.
    pub fn resolve_local(
        &self,
        local: civil::DateTime,
        policy: ConversionPolicy,
    ) -> Result<OffsetDateTime, EngineError> {
        let ambiguous = self.zone.to_ambiguous_timestamp(local);
        let timestamp = match policy {
            ConversionPolicy::Reject => ambiguous.unambiguous(),
            ConversionPolicy::Earlier => ambiguous.earlier(),
            ConversionPolicy::Later => ambiguous.later(),
        }
        .map_err(|_| EngineError::invalid())?;
        OffsetDateTime::from_unix_timestamp_nanos(timestamp.as_nanosecond())
            .map_err(|_| EngineError::precision())
    }
    pub fn expand(
        &self,
        c: &CommitmentV1,
        start: Date,
        months: u8,
    ) -> Result<Vec<OccurrenceV1>, EngineError> {
        validate(c)?;
        if *c.calendar_data != self.data
            || c.timezone != self.data.timezone
            || c.timezone_data_version != self.data.timezone_data_version
            || c.calendar_version != self.data.calendar_version
        {
            return Err(EngineError::version());
        }
        if !matches!(months, 1 | 3 | 12) {
            return Err(EngineError::invalid());
        }
        let end = from_civil(
            to_civil(start)?
                .checked_add(
                    Span::new()
                        .try_months(months)
                        .map_err(|_| EngineError::precision())?,
                )
                .map_err(|_| EngineError::precision())?,
        )?;
        let lower = start.max(c.start.0);
        let upper = c.end.map_or(end, |d| end.min(d.0));
        if lower > upper {
            return Ok(vec![]);
        }
        // Binary search the civil sequence, so decades of past daily occurrences
        // do not require materializing or scanning historical expectations.
        let mut lo = 0_u64;
        let mut hi = MAX_ORDINAL;
        while lo < hi {
            let mid = lo
                .checked_add(
                    hi.checked_sub(lo)
                        .and_then(|distance| distance.checked_div(2))
                        .ok_or_else(EngineError::precision)?,
                )
                .ok_or_else(EngineError::precision)?;
            if nominal(c, mid)?.is_some_and(|d| d < lower) {
                lo = mid.checked_add(1).ok_or_else(EngineError::precision)?;
            } else {
                hi = mid;
            }
        }
        let mut ordinals = BTreeSet::new();
        let mut n = lo;
        while let Some(d) = nominal(c, n)? {
            if d > upper {
                break;
            }
            ordinals.insert(n);
            if ordinals.len() > 4000 {
                return Err(EngineError::invalid());
            }
            n = n.checked_add(1).ok_or_else(EngineError::precision)?;
        }
        // Date exceptions can move an otherwise out-of-window occurrence in.
        for e in &c.exceptions {
            ordinals.insert(e.ordinal.0);
        }
        let mut result = vec![];
        for n in ordinals {
            let Some(mut due) = nominal(c, n)? else {
                return Err(EngineError::invalid());
            };
            let mut legs = c.legs.clone();
            if let Some(e) = c.exceptions.iter().find(|e| e.ordinal.0 == n) {
                match e.action {
                    PlanningExceptionActionV1::Skip | PlanningExceptionActionV1::Cancel => continue,
                    PlanningExceptionActionV1::Replace => {
                        if let Some(d) = e.due_date {
                            due = d.0;
                        }
                        if !e.legs.is_empty() {
                            legs = e.legs.clone();
                        }
                    }
                }
            }
            if due < lower || due > upper {
                continue;
            }
            let range = settlement(due, &c.settlement, c.due_date.0)?;
            for leg in &mut legs {
                leg.settlement =
                    SettlementRuleV1::Range(settlement(due, &leg.settlement, c.due_date.0)?);
            }
            result.push(OccurrenceV1 {
                key: OccurrenceKeyV1 {
                    commitment_id: c.id,
                    series_generation: c.series_generation,
                    ordinal: CommitSeq(n),
                },
                version: c.version,
                legs,
                due_date: CivilDateV1(due),
                settlement_range: range,
            });
        }
        result.sort_by_key(|o| (o.due_date.0, o.key.ordinal));
        Ok(result)
    }
}
fn to_civil(d: Date) -> Result<civil::Date, EngineError> {
    civil::Date::new(
        i16::try_from(d.year()).map_err(|_| EngineError::precision())?,
        d.month() as i8,
        i8::try_from(d.day()).map_err(|_| EngineError::precision())?,
    )
    .map_err(|_| EngineError::precision())
}
fn from_civil(d: civil::Date) -> Result<Date, EngineError> {
    Date::from_ordinal_date(
        i32::from(d.year()),
        u16::try_from(d.day_of_year()).map_err(|_| EngineError::precision())?,
    )
    .map_err(|_| EngineError::precision())
}
pub(crate) fn nominal(c: &CommitmentV1, ordinal: u64) -> Result<Option<Date>, EngineError> {
    if ordinal > MAX_ORDINAL {
        return Ok(None);
    }
    if c.recurrence.cadence == RecurrenceCadenceV1::Once && ordinal != 0 {
        return Ok(None);
    }
    let count = ordinal
        .checked_mul(u64::from(c.recurrence.interval))
        .and_then(|n| i64::try_from(n).ok())
        .ok_or_else(EngineError::precision)?;
    let anchor = to_civil(c.due_date.0)?;
    let span = match c.recurrence.cadence {
        RecurrenceCadenceV1::Once => Ok(Span::new()),
        RecurrenceCadenceV1::Daily => Span::new().try_days(count),
        RecurrenceCadenceV1::Weekly => Span::new().try_weeks(count),
        RecurrenceCadenceV1::Monthly => Span::new().try_months(count),
        RecurrenceCadenceV1::Annual => Span::new().try_years(count),
    };
    let Ok(span) = span else {
        return Ok(None);
    };
    let Ok(mut date) = anchor.checked_add(span) else {
        return Ok(None);
    };
    if matches!(
        c.recurrence.cadence,
        RecurrenceCadenceV1::Monthly | RecurrenceCadenceV1::Annual
    ) {
        if let Some(month) = c.recurrence.month {
            date = date
                .with()
                .day(1)
                .month(i8::try_from(month).map_err(|_| EngineError::invalid())?)
                .build()
                .map_err(|_| EngineError::invalid())?;
        }
        let day = match &c.recurrence.day {
            Some(RecurrenceDayV1::LastDay(_)) => date.days_in_month(),
            Some(RecurrenceDayV1::Day(d)) => i8::try_from(*d)
                .map_err(|_| EngineError::invalid())?
                .min(date.days_in_month()),
            None => anchor.day().min(date.days_in_month()),
        };
        date = date
            .with()
            .day(day)
            .build()
            .map_err(|_| EngineError::invalid())?;
    }
    from_civil(date).map(Some)
}
fn settlement(
    due: Date,
    rule: &SettlementRuleV1,
    anchor: Date,
) -> Result<SettlementRangeV1, EngineError> {
    let mut d = to_civil(due)?;
    match rule {
        SettlementRuleV1::SameDay(_) => {}
        SettlementRuleV1::NextWeekday(_) | SettlementRuleV1::PreviousWeekday(_) => {
            let step = if matches!(rule, SettlementRuleV1::NextWeekday(_)) {
                1
            } else {
                -1
            };
            while matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday) {
                d = d
                    .checked_add(
                        Span::new()
                            .try_days(step)
                            .map_err(|_| EngineError::precision())?,
                    )
                    .map_err(|_| EngineError::precision())?;
            }
        }
        SettlementRuleV1::Range(r) => {
            if r.earliest.0 > r.latest.0 {
                return Err(EngineError::invalid());
            }
            let shifted = |date: Date| {
                let days = date
                    .to_julian_day()
                    .checked_sub(anchor.to_julian_day())
                    .ok_or_else(EngineError::precision)?;
                let span = Span::new()
                    .try_days(days)
                    .map_err(|_| EngineError::precision())?;
                from_civil(
                    to_civil(due)?
                        .checked_add(span)
                        .map_err(|_| EngineError::precision())?,
                )
                .map(CivilDateV1)
            };
            return Ok(SettlementRangeV1 {
                earliest: shifted(r.earliest.0)?,
                latest: shifted(r.latest.0)?,
            });
        }
        SettlementRuleV1::HolidayCalendar(_) => return Err(EngineError::version()),
    }
    let date = CivilDateV1(from_civil(d)?);
    Ok(SettlementRangeV1 {
        earliest: date,
        latest: date,
    })
}
pub(crate) fn validate(c: &CommitmentV1) -> Result<(), EngineError> {
    if c.recurrence_version != 1 || c.calendar_version != 1 {
        return Err(EngineError::version());
    }
    if c.version == 0
        || c.series_generation == 0
        || c.recurrence.interval == 0
        || c.legs.is_empty()
        || c.legs.len() > 1000
        || c.exceptions.len() > 4000
        || c.end.is_some_and(|end| end.0 < c.start.0)
        || c.timezone.is_empty()
        || c.timezone_data_version.is_empty()
        || c.calendar_data.timezone_data.byte_count.0 == 0
        || c.calendar_data.timezone_data.byte_count.0 > MAX_TIMEZONE_BYTES as u64
        || c.provenance.authority != AuthorityKind::Owner
        || c.provenance.kind != DecisionKind::Planning
        || c.calendar_data.timezone != c.timezone
        || c.calendar_data.timezone_data_version != c.timezone_data_version
        || c.calendar_data.calendar_version != c.calendar_version
        || c.calendar_data.timezone_data.media_type != "application/tzif"
        || !c.calendar_data.holidays.is_empty()
    {
        return Err(EngineError::invalid());
    }
    match c.recurrence.cadence {
        RecurrenceCadenceV1::Once | RecurrenceCadenceV1::Daily | RecurrenceCadenceV1::Weekly
            if c.recurrence.day.is_some() || c.recurrence.month.is_some() =>
        {
            return Err(EngineError::invalid());
        }
        RecurrenceCadenceV1::Monthly if c.recurrence.month.is_some() => {
            return Err(EngineError::invalid());
        }
        _ => {}
    }
    if c.recurrence.month.is_some_and(|m| !(1..=12).contains(&m))
        || matches!(c.recurrence.day, Some(RecurrenceDayV1::Day(d)) if !(1..=31).contains(&d))
    {
        return Err(EngineError::invalid());
    }
    validate_amounts(&c.amounts)?;
    settlement(c.due_date.0, &c.settlement, c.due_date.0)?;
    let mut ids = BTreeSet::new();
    for leg in &c.legs {
        validate_amounts(&leg.amounts)?;
        settlement(c.due_date.0, &leg.settlement, c.due_date.0)?;
        if !ids.insert(leg.id) {
            return Err(EngineError::invalid());
        }
    }
    let mut ordinals = BTreeSet::new();
    for e in &c.exceptions {
        let mut replacement_ids = BTreeSet::new();
        for leg in &e.legs {
            if !replacement_ids.insert(leg.id) {
                return Err(EngineError::invalid());
            }
            validate_amounts(&leg.amounts)?;
            settlement(
                e.due_date.unwrap_or(c.due_date).0,
                &leg.settlement,
                c.due_date.0,
            )?;
        }
        if !ordinals.insert(e.ordinal) || nominal(c, e.ordinal.0)?.is_none() {
            return Err(EngineError::invalid());
        }
    }
    Ok(())
}

fn validate_amounts(a: &PlanningAmountsV1) -> Result<(), EngineError> {
    let nominal = match &a.nominal {
        SourceValue::Known(a) => Some(a),
        _ => None,
    };
    let range = match (&a.low, &a.base, &a.high) {
        (SourceValue::Known(l), SourceValue::Known(b), SourceValue::Known(h)) => Some((l, b, h)),
        _ => None,
    };
    if range.is_none()
        && [&a.low, &a.base, &a.high]
            .iter()
            .any(|value| matches!(value, SourceValue::Known(_)))
    {
        return Err(EngineError::invalid());
    }
    if nominal.is_none() && range.is_none() {
        return Err(EngineError::invalid());
    }
    if let Some(a) = nominal {
        a.checked_rescale(a.scale)?;
    }
    if let Some((l, b, h)) = range {
        if l.unit_id != b.unit_id
            || b.unit_id != h.unit_id
            || nominal.is_some_and(|n| n.unit_id != b.unit_id)
        {
            return Err(EngineError::invalid());
        }
        let scale = l.scale.max(b.scale).max(h.scale);
        if l.checked_rescale(scale)?.coefficient > b.checked_rescale(scale)?.coefficient
            || b.checked_rescale(scale)?.coefficient > h.checked_rescale(scale)?.coefficient
        {
            return Err(EngineError::invalid());
        }
    }
    Ok(())
}
