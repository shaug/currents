//! Explicit civil conversion; original source values never change.
use ::time::{Date, UtcOffset, format_description::well_known::Rfc3339};
use currents_types::*;
pub(crate) fn parse(value: &str) -> Result<Date, EngineError> {
    let f = ::time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
        .map_err(|_| EngineError::invalid())?;
    let d = Date::parse(value, &f).map_err(|_| EngineError::invalid())?;
    if !(1..=9999).contains(&d.year()) {
        return Err(EngineError::invalid());
    }
    Ok(d)
}
fn offset(value: &str) -> Result<UtcOffset, EngineError> {
    let f = ::time::format_description::parse_borrowed::<2>(
        "[offset_hour sign:mandatory]:[offset_minute]",
    )
    .map_err(|_| EngineError::invalid())?;
    UtcOffset::parse(value, &f).map_err(|_| EngineError::invalid())
}
pub(crate) fn civil(
    value: &TemporalValue,
    policy: &DateConversionPolicyV1,
) -> Result<Option<Date>, EngineError> {
    if policy.version == 0 {
        return Err(EngineError::version());
    }
    let date = match value {
        TemporalValue::Unknown { .. } => return Ok(None),
        TemporalValue::Date { civil, .. } => *civil,
        TemporalValue::Instant {
            utc, original_zone, ..
        } => {
            let off = match &policy.conversion {
                DateConversionV1::CivilOnly => return Err(EngineError::version()),
                DateConversionV1::Instants(InstantConversionV1::FixedOffset(v)) => {
                    offset(&v.offset)?
                }
                DateConversionV1::Instants(InstantConversionV1::Resolved(values)) => {
                    let text = utc.format(&Rfc3339).map_err(|_| EngineError::invalid())?;
                    let mut matches = values
                        .iter()
                        .filter(|v| v.utc == text && original_zone.as_ref() == Some(&v.zone));
                    let resolved = matches.next().ok_or_else(EngineError::invalid)?;
                    if matches.next().is_some()
                        || resolved.zone.is_empty()
                        || resolved.data_version.is_empty()
                    {
                        return Err(EngineError::invalid());
                    }
                    offset(&resolved.offset)?
                }
            };
            utc.checked_to_offset(off)
                .ok_or_else(EngineError::invalid)?
                .date()
        }
    };
    if !(1..=9999).contains(&date.year()) {
        return Err(EngineError::invalid());
    }
    Ok(Some(date))
}
pub(crate) fn mapping(
    recognition: &RecognitionV1,
    policy: &DateConversionPolicyV1,
) -> Result<(String, String), EngineError> {
    let booking = civil(&recognition.bookkeeping_date, policy)?.ok_or_else(EngineError::invalid)?;
    let mut effective = None;
    let mut posting = None;
    for (i, p) in recognition.postings.iter().enumerate() {
        let e = civil(&p.effective, policy)?;
        let b = civil(&p.posting, policy)?;
        if i == 0 {
            effective = e;
            posting = b;
        } else if effective != e || posting != b {
            return Err(EngineError::invalid());
        }
    }
    match posting {
        Some(p) if p != booking => return Err(EngineError::invalid()),
        Some(_) => {}
        None => match policy.bookkeeping {
            BookkeepingFallbackV1::RequirePosting => return Err(EngineError::invalid()),
            BookkeepingFallbackV1::Effective if effective != Some(booking) => {
                return Err(EngineError::invalid());
            }
            _ => {}
        },
    }
    Ok((
        booking.to_string(),
        effective.unwrap_or(booking).to_string(),
    ))
}
