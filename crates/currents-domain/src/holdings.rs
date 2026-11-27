//! The pure holdings comparison: security quantities at one boundary.
use currents_types::{Amount, Date, EngineError, RecordId, UnitId};
use std::collections::{BTreeMap, BTreeSet};

/// One security position: a quantity in the security's unit, under one
/// corporate-action basis, at the end of one day. It carries no price, so
/// market value never enters the comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldingPositionV1 {
    pub quantity: Amount,
    pub basis: RecordId,
    pub boundary: Date,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldingDifferenceV1 {
    /// The source quantity minus the journal quantity, in the security's
    /// unit.
    Quantity(Amount),
    /// The two sides state the security under different corporate-action
    /// bases, so their quantities are not compared.
    IncompatibleBasis(UnitId),
    /// The two sides state the security at different boundaries, so their
    /// quantities are not compared.
    IncompatibleBoundary(UnitId),
}

/// Compare source positions with journal positions, one security at a
/// time in unit order. Each side names a security at most once. A security
/// on one side only compares with a zero quantity. An empty list is
/// agreement.
pub fn compare_holdings(
    source: &[HoldingPositionV1],
    journal: &[HoldingPositionV1],
) -> Result<Vec<HoldingDifferenceV1>, EngineError> {
    let source = by_security(source)?;
    let journal = by_security(journal)?;
    let mut differences = Vec::new();
    for security in source.keys().chain(journal.keys()).collect::<BTreeSet<_>>() {
        let zero = Amount::new(0, 0, *security)?;
        let quantity = |p: Option<&&HoldingPositionV1>| p.map_or(zero, |p| p.quantity);
        let (s, j) = (source.get(security), journal.get(security));
        if let (Some(s), Some(j)) = (s, j) {
            if s.basis != j.basis {
                differences.push(HoldingDifferenceV1::IncompatibleBasis(*security));
            }
            if s.boundary != j.boundary {
                differences.push(HoldingDifferenceV1::IncompatibleBoundary(*security));
            }
            if s.basis != j.basis || s.boundary != j.boundary {
                continue;
            }
        }
        let difference = quantity(s)
            .checked_add(&quantity(j).checked_neg()?)?
            .normalized()?;
        if difference.coefficient != 0 {
            differences.push(HoldingDifferenceV1::Quantity(difference));
        }
    }
    Ok(differences)
}

fn by_security(
    positions: &[HoldingPositionV1],
) -> Result<BTreeMap<UnitId, &HoldingPositionV1>, EngineError> {
    let mut map = BTreeMap::new();
    for position in positions {
        if map.insert(position.quantity.unit_id, position).is_some() {
            return Err(EngineError::invalid());
        }
    }
    Ok(map)
}
