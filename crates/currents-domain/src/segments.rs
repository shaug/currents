//! Initial evidence-supported opening only. Replacement remains CUR016.
use crate::{
    catalog::{CatalogEngine, incomplete},
    recognition::fact,
};
use currents_types::*;
use std::collections::BTreeSet;
fn civil(value: &TemporalValue) -> Result<Date, EngineError> {
    match value {
        TemporalValue::Date { civil, .. } => Ok(*civil),
        _ => Err(incomplete()),
    }
}
pub(crate) fn source_time(
    state: &DomainState,
    binding: &SourceFacetBindingV1,
    kind: ObservationKind,
) -> Result<Date, EngineError> {
    fact(state, binding)?;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(p)) =
        state.records.get(&binding.parse_product)
    else {
        return Err(incomplete());
    };
    let f = p
        .product
        .facets
        .iter()
        .find(|f| f.lineage_key == binding.facet)
        .ok_or_else(incomplete)?;
    if f.kind != kind {
        return Err(EngineError::invalid());
    }
    civil(&f.temporal)
}
pub(crate) fn initial(
    state: &DomainState,
    request: &ActivityRequestV1,
    admission: bool,
) -> Result<Option<AccountingSegmentV1>, EngineError> {
    let Some(terms) = &request.financial else {
        return Ok(None);
    };
    let Some(opening) = &terms.opening else {
        return Ok(None);
    };
    if request.status != ActivityStatusV1::Posted
        || request.prior_pending.is_some()
        || terms.unresolved_account.is_some()
        || request.legs.len() != 1
        || opening.included_activity.len() > 4096
        || opening.segment_id.0.is_nil()
    {
        return Err(EngineError::invalid());
    }
    let leg = &request.legs[0];
    let Some(AuthoritativeRecordV1::AccountComponent(c)) = state.records.get(&opening.component_id)
    else {
        return Err(incomplete());
    };
    if c.id != opening.component_id
        || c.account_id != leg.leg.account_id
        || c.unit_id != opening.amount.unit_id
        || !matches!(c.opening_boundary, SourceValue::Unknown)
        || CatalogEngine::current(state, c.id)?.origin != AccountOriginV1::External
        || opening.amount.normalized()? != leg.leg.movement.normalized()?
        || opening.evidence.amount.normalized()? != opening.amount.normalized()?
        || leg.sources != vec![opening.evidence.clone()]
    {
        return Err(EngineError::invalid());
    }
    let cutoff = civil(&opening.cutoff)?;
    if source_time(state, &opening.evidence, ObservationKind::Balance)? != cutoff
        || civil(&leg.leg.effective)? != cutoff
        || civil(&terms.bookkeeping_date)? != cutoff
    {
        return Err(EngineError::invalid());
    }
    let mut facts = BTreeSet::new();
    for source in &opening.included_activity {
        if source.amount.unit_id != c.unit_id
            || source_time(state, source, ObservationKind::Movement)? > cutoff
            || !facts.insert(fact(state, source)?)
        {
            return Err(EngineError::invalid());
        }
    }
    if admission
        && state.records.values().any(|r| match r {
            AuthoritativeRecordV1::AccountingSegment(s) => s.component == c.id,
            AuthoritativeRecordV1::Recognition(r) => {
                r.postings.iter().any(|p| p.account_id == c.account_id)
            }
            _ => false,
        })
    {
        return Err(incomplete());
    }
    if !admission {
        let mut anchors = state.records.values().filter_map(|r| match r {
            AuthoritativeRecordV1::AccountingSegment(s) if s.component == c.id => Some(s),
            _ => None,
        });
        let anchor = anchors.next().ok_or_else(incomplete)?;
        if anchors.next().is_some()
            || anchor.id != opening.segment_id
            || anchor.opening != terms.recognition_id
        {
            return Err(EngineError::invalid());
        }
    }
    Ok(Some(AccountingSegmentV1 {
        id: opening.segment_id,
        component: c.id,
        prior_segment: None,
        opening: terms.recognition_id,
        included_activity: vec![],
        replacement_closure: vec![],
        approved_accounts: terms.postings.iter().map(|p| p.account_id).collect(),
        residual_disposition: SourceValue::Absent,
    }))
}
/// Activity at/before a supported opening needs explicit reconciliation, not a
/// second monetary entry. The exact included facts are also ownership checked.
pub(crate) fn activity_boundary(
    state: &DomainState,
    request: &ActivityRequestV1,
) -> Result<(), EngineError> {
    if request
        .financial
        .as_ref()
        .is_some_and(|f| f.opening.is_some())
    {
        return Ok(());
    }
    for record in state.records.values() {
        let AuthoritativeRecordV1::ActivityAdoption(a) = record else {
            continue;
        };
        let Some(opening) = &a.opening else { continue };
        let Some(AuthoritativeRecordV1::AccountComponent(c)) =
            state.records.get(&opening.component_id)
        else {
            return Err(incomplete());
        };
        for leg in &request.legs {
            if leg.leg.account_id == c.account_id
                && civil(&leg.leg.effective)? <= civil(&opening.cutoff)?
            {
                return Err(incomplete());
            }
        }
    }
    Ok(())
}
