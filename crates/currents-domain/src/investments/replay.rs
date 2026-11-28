//! Replay derives each trade commit again from the prior prefix, from the
//! request its records restore.
use crate::catalog::incomplete;
use currents_types::*;

/// The request whose plan holds `trade` and the records `next` holds for it:
/// its event, each known side's leg and its recognition.
pub(crate) fn trade_request(
    next: &DomainState,
    trade: &TradeV1,
) -> Result<TradeRequestV1, EngineError> {
    let Some(AuthoritativeRecordV1::FinancialEvent(event)) = next.records.get(&trade.event_id)
    else {
        return Err(incomplete());
    };
    let side = |facet: &SourceValue<AdoptedFacetV1>| match facet {
        SourceValue::Known(facet) => {
            let Some(AuthoritativeRecordV1::FinancialLeg(leg)) =
                next.records.get(&RecordId(facet.leg_id.0))
            else {
                return Err(incomplete());
            };
            Ok(SourceValue::Known(AdoptedLegV1 {
                leg: leg.clone(),
                sources: vec![facet.source.clone()],
            }))
        }
        SourceValue::Missing => Ok(SourceValue::Missing),
        _ => Err(EngineError::invalid()),
    };
    let Some(AuthoritativeRecordV1::Recognition(recognition)) =
        next.records.get(&RecordId(trade.recognition.0))
    else {
        return Err(incomplete());
    };
    Ok(TradeRequestV1 {
        schema_version: trade.schema_version,
        id: trade.id,
        position: trade.position,
        side: trade.side,
        event: event.clone(),
        cash: side(&trade.cash)?,
        quantity: side(&trade.quantity)?,
        fee: trade.fee,
        terms: RecognitionTermsV1 {
            recognition_id: trade.recognition,
            lineage: recognition.lineage,
            postings: recognition.postings.clone(),
            bookkeeping_date: recognition.bookkeeping_date.clone(),
            date_policy: recognition.date_policy.clone(),
            unresolved_account: None,
            opening: None,
        },
        interpretation: trade.interpretation.clone(),
        governing_policy: trade.governing_policy.clone(),
    })
}
