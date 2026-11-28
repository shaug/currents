//! Holdings snapshots: what a source reports an account scope held on one
//! day. A snapshot never changes a recognized quantity. A complete snapshot
//! asserts that each position in its scope whose security it omits holds
//! none, unless it also lists a position whose identifier names no
//! security; a partial or unknown snapshot asserts nothing about what it
//! omits.
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

fn snapshots(state: &DomainState) -> impl Iterator<Item = &HoldingsSnapshotV1> {
    state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::HoldingsSnapshot(snapshot) => Some(snapshot),
        _ => None,
    })
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

/// The snapshot record `draft` publishes after `state`, with the absences
/// it asserts. Each holding names its observation, the security its
/// identifier names at `state` and its quantity in that security's unit, or
/// no security when the identifier names none; a display symbol names
/// nothing. A scope whose positions hold one security twice is refused.
///
/// An unmapped holding could be any security in scope, so a complete
/// snapshot that lists one records no absence. An absence's version counts
/// the absences recorded for its position in publication order, and `prior`
/// names the snapshot that holds the previous version, whatever their as-of
/// dates: a statement imported late takes the next version.
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
    // The position each security in scope belongs to.
    let mut held = BTreeMap::new();
    for record in state.records.values() {
        if let AuthoritativeRecordV1::Position(p) = record
            && scope.contains(&p.quantity_account)
            && held.insert(p.security, p.id).is_some()
        {
            return Err(invalid());
        }
    }
    let mut snapshot = draft.clone();
    snapshot.absences = vec![];
    let unmapped = draft.holdings.iter().any(|h| h.security.is_none());
    if draft.completeness == CoverageStatus::Complete && !unmapped {
        for (security, position) in held {
            if listed.contains(&security) {
                continue;
            }
            let prior = snapshots(state)
                .flat_map(|s| s.absences.iter().map(move |a| (s.id, a)))
                .filter(|(_, a)| a.position == position)
                .max_by_key(|(_, a)| a.version);
            let version = match prior {
                Some((_, a)) => a.version.checked_add(1).ok_or_else(invalid)?,
                None => 1,
            };
            snapshot.absences.push(HoldingAbsenceV1 {
                position,
                security,
                version,
                prior: prior.map(|(id, _)| id),
            });
        }
    }
    Ok(snapshot)
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

/// The latest snapshot that lists a position's security: its quantity in
/// the security's unit and its source market value. A read result, not a
/// record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotQuantityV1 {
    pub snapshot: RecordId,
    pub as_of: Date,
    pub quantity: Amount,
    pub market_value: Option<Amount>,
}

/// An absence assertion no later snapshot contradicts by listing the
/// security. A read result, not a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotAbsenceV1 {
    pub snapshot: RecordId,
    pub as_of: Date,
    pub version: u32,
}

/// The positions a snapshot lists with no security. Any of them could be a
/// position's security. A read result, not a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotUnmappedV1 {
    pub snapshot: RecordId,
    pub as_of: Date,
    pub observations: Vec<RecordId>,
}

/// What snapshots report for one position. See `latest`.
pub(crate) type Latest = (
    Option<SnapshotQuantityV1>,
    Option<SnapshotAbsenceV1>,
    Option<SnapshotUnmappedV1>,
);

/// For `position` through `through`: the latest snapshot whose scope covers
/// its quantity account and lists its security; the latest absence
/// assertion for it when no snapshot at least as late lists the security;
/// and the unmapped positions of the latest snapshot whose scope covers its
/// quantity account, if it lists any. Snapshots order by as-of date, then
/// by the commit of their receipt. The current absence is therefore the one
/// with the latest as-of date, not the highest version.
pub(crate) fn latest(
    state: &DomainState,
    position: &PositionV1,
    through: Date,
) -> Result<Latest, EngineError> {
    type Keyed<T> = Option<((Date, Option<CommitSeq>), T)>;
    let mut listed: Keyed<SnapshotQuantityV1> = None;
    let mut absent: Keyed<SnapshotAbsenceV1> = None;
    let mut covering: Keyed<(&HoldingsSnapshotV1, Date)> = None;
    for snapshot in snapshots(state) {
        let TemporalValue::Date { civil, .. } = snapshot.as_of else {
            return Err(invalid());
        };
        if civil > through || !snapshot.scope.contains(&position.quantity_account) {
            continue;
        }
        let key = (civil, state.receipt_commits.get(&snapshot.receipt).copied());
        if covering.as_ref().is_none_or(|(old, _)| key > *old) {
            covering = Some((key, (snapshot, civil)));
        }
        let holding = snapshot
            .holdings
            .iter()
            .find(|h| h.security == Some(position.security));
        if let Some(holding) = holding
            && listed.as_ref().is_none_or(|(old, _)| key > *old)
        {
            let value = SnapshotQuantityV1 {
                snapshot: snapshot.id,
                as_of: civil,
                quantity: holding.quantity.ok_or_else(invalid)?,
                market_value: holding.market_value,
            };
            listed = Some((key, value));
        }
        let absence = snapshot.absences.iter().find(|a| a.position == position.id);
        if let Some(absence) = absence
            && absent.as_ref().is_none_or(|(old, _)| key > *old)
        {
            let value = SnapshotAbsenceV1 {
                snapshot: snapshot.id,
                as_of: civil,
                version: absence.version,
            };
            absent = Some((key, value));
        }
    }
    let current = match (&listed, absent) {
        (Some((seen, _)), Some((key, absence))) if key > *seen => Some(absence),
        (None, Some((_, absence))) => Some(absence),
        _ => None,
    };
    let unmapped = covering.and_then(|(_, (snapshot, as_of))| {
        let observations: Vec<RecordId> = snapshot
            .holdings
            .iter()
            .filter(|h| h.security.is_none())
            .map(|h| h.observation)
            .collect();
        (!observations.is_empty()).then_some(SnapshotUnmappedV1 {
            snapshot: snapshot.id,
            as_of,
            observations,
        })
    });
    Ok((listed.map(|(_, value)| value), current, unmapped))
}
