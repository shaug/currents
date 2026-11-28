//! Holdings snapshots: what a source reports an account scope held on one
//! day. A snapshot never changes a recognized quantity.
use super::InvestmentEngine;
use crate::catalog::unit;
use crate::evidence::decimal;
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

/// Each listed observation and its one facet.
pub(crate) type Rows<'a> = BTreeMap<RecordId, (&'a ObservationVersionV1, &'a MovementFacetV1)>;

fn invalid() -> EngineError {
    EngineError::invalid()
}

/// The fields a holdings row reports, as the profile records them.
fn fields(row: &SyntheticHoldingRowV1) -> Vec<ReportedFieldV1> {
    let known = |value: &String| SourceValue::Known(value.clone());
    let optional = |value: &Option<String>| value.as_ref().map_or(SourceValue::Absent, known);
    [
        ("source", known(&row.source)),
        ("identifier", known(&row.identifier)),
        ("symbol", optional(&row.symbol)),
        ("quantity", known(&row.quantity)),
        ("market_value", optional(&row.market_value)),
    ]
    .into_iter()
    .map(|(name, value)| ReportedFieldV1 {
        name: name.into(),
        value,
    })
    .collect()
}

/// The snapshot record `draft` publishes after `state`. Each holding names
/// its observation, the security its identifier names at `state` and its
/// quantity in that security's unit, or no security when the identifier
/// names none; a display symbol names nothing.
pub(crate) fn derive(
    state: &DomainState,
    rows: &Rows,
    draft: &HoldingsSnapshotV1,
) -> Result<HoldingsSnapshotV1, EngineError> {
    if draft.schema_version != 1 {
        return Err(EngineError::version());
    }
    let TemporalValue::Date {
        civil,
        original_text,
    } = &draft.as_of
    else {
        return Err(invalid());
    };
    let currency = unit(state, draft.currency)?;
    let scope: BTreeSet<AccountId> = draft.scope.iter().copied().collect();
    if draft.id.0.is_nil()
        || state.records.contains_key(&draft.id)
        || civil.to_string() != *original_text
        || currency.kind != UnitKind::Currency
        || scope.is_empty()
        || scope.len() > 256
        || scope.len() != draft.scope.len()
        || rows.len() != draft.holdings.len()
        || draft
            .market_value
            .is_some_and(|m| m.unit_id != currency.id || m.scale > currency.exponent)
    {
        return Err(invalid());
    }
    for account in &scope {
        let declared = state.records.values().any(|record| {
            matches!(record, AuthoritativeRecordV1::AccountComponent(c) if c.account_id == *account)
        });
        if !declared {
            return Err(crate::catalog::incomplete());
        }
    }
    let mut listed = BTreeSet::new();
    for holding in &draft.holdings {
        let (observation, facet) = rows.get(&holding.observation).ok_or_else(invalid)?;
        let row: SyntheticHoldingRowV1 =
            serde_json::from_str(&observation.sanitized_representation).map_err(|_| invalid())?;
        let identifier = SecuritySourceIdV1 {
            source: row.source.clone(),
            identifier: row.identifier.clone(),
        };
        let (security, quantity) = match InvestmentEngine::resolve(state, &identifier)? {
            Some(security) => (
                Some(security.root),
                Some(decimal(&row.quantity, &unit(state, security.unit_id)?)?),
            ),
            None => (None, None),
        };
        let market_value = row
            .market_value
            .as_deref()
            .map(|text| decimal(text, &currency))
            .transpose()?;
        let expected = SnapshotHoldingV1 {
            observation: observation.id,
            security,
            quantity,
            market_value,
        };
        if observation.semantic_kind != ObservationKind::Holding
            || observation.temporal != draft.as_of
            || observation.reported_fields != fields(&row)
            || facet.kind != ObservationKind::Holding
            || facet.temporal != draft.as_of
            || facet.amount != quantity.map_or(SourceValue::Unknown, SourceValue::Known)
            || *holding != expected
            || security.is_some_and(|s| !listed.insert(s))
        {
            return Err(invalid());
        }
    }
    Ok(draft.clone())
}

/// The plan replay derives again for `snapshot`: its receipt and each
/// listed observation are new in `next`, and the record is the one `derive`
/// gives from the prior prefix.
pub(crate) fn readmit(
    prior: &DomainState,
    next: &DomainState,
    snapshot: &HoldingsSnapshotV1,
) -> Result<DecisionPlan, EngineError> {
    if prior.records.contains_key(&snapshot.receipt)
        || !matches!(
            next.records.get(&snapshot.receipt),
            Some(AuthoritativeRecordV1::ImportReceipt(_))
        )
    {
        return Err(invalid());
    }
    let wanted: BTreeSet<RecordId> = snapshot.holdings.iter().map(|h| h.observation).collect();
    let mut rows = Rows::new();
    for record in next.records.values() {
        let AuthoritativeRecordV1::AddressedParseProduct(product) = record else {
            continue;
        };
        let source = product.product.source_version_id;
        if !wanted.contains(&source) {
            continue;
        }
        let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
            next.records.get(&source)
        else {
            return Err(invalid());
        };
        let [facet] = product.product.facets.as_slice() else {
            return Err(invalid());
        };
        if prior.records.contains_key(&source)
            || rows.insert(source, (observation, facet)).is_some()
        {
            return Err(invalid());
        }
    }
    Ok(DecisionPlan {
        records: vec![AuthoritativeRecordV1::HoldingsSnapshot(derive(
            prior, &rows, snapshot,
        )?)],
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "Holdings snapshot evidence only; no quantity change.".into(),
        }),
    })
}
