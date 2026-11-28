//! Household aggregation (C09). The aggregate counts each in-perimeter
//! component once, at its exact governing ownership share, in the reporting
//! currency. A component whose amount, share or value is not known is
//! omitted with one named reason and never counts as zero. A brokerage total
//! counts, or its cash and holdings parts do, never both.
use crate::catalog::unit;
use crate::policy::{PolicyEngine, dataset};
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

/// A component's amount as the position query found it.
pub(crate) enum Found {
    /// The amount in the component's unit, and the records it rests on.
    Known(Amount, Vec<RecordId>),
    /// Why the query found no amount.
    Missing(&'static str),
}

/// Where each component's ownership share comes from on the requested date.
enum Perimeter {
    /// No revision sets the perimeter, so each catalog share applies.
    Catalog,
    /// The governing perimeter is not known.
    Unknown,
    /// The governing perimeter; an account it leaves out is outside the
    /// household.
    Known(BTreeMap<AccountId, (i128, i128)>),
}

/// The reporting currency: the governing value, or the dataset definition's
/// when no revision sets it. `None` when the governing value is not known.
fn reporting_currency(
    state: &DomainState,
    setting: &PolicySettingV1<UnitId>,
) -> Result<Option<UnitId>, EngineError> {
    Ok(match setting {
        PolicySettingV1::NotSet => Some(dataset(state)?.reporting_currency),
        PolicySettingV1::Set {
            value: SourceValue::Known(currency),
            ..
        } => Some(*currency),
        PolicySettingV1::Set { .. } => None,
    })
}

fn perimeter(setting: &PolicySettingV1<Vec<OwnershipV1>>) -> Perimeter {
    match setting {
        PolicySettingV1::NotSet => Perimeter::Catalog,
        PolicySettingV1::Set {
            value: SourceValue::Known(entries),
            ..
        } => Perimeter::Known(entries.iter().map(|e| (e.account_id, ratio(e))).collect()),
        PolicySettingV1::Set { .. } => Perimeter::Unknown,
    }
}

fn ratio(ownership: &OwnershipV1) -> (i128, i128) {
    (
        i128::from(ownership.numerator.0),
        i128::from(ownership.denominator.0),
    )
}

/// `amount` times `numerator / denominator` at the smallest scale that holds
/// it exactly, or `None` when no scale up to the limit does.
fn owned(amount: Amount, (numerator, denominator): (i128, i128)) -> Option<Amount> {
    (amount.scale..=18).find_map(|scale| amount.checked_ratio(numerator, denominator, scale).ok())
}

/// The signed amount one component adds to the total, or the reason it is
/// omitted. A liability's amount is the positive amount owed, so it
/// subtracts.
fn contribution(
    state: &DomainState,
    root: &AccountComponentV1,
    found: &Found,
    share: Option<(i128, i128)>,
    currency: Option<UnitId>,
) -> Result<Result<Amount, &'static str>, EngineError> {
    let Some(share) = share else {
        return Ok(Err("unknown_ownership"));
    };
    let Some(currency) = currency else {
        return Ok(Err("unknown_reporting_currency"));
    };
    let negate = match root.account_class {
        AccountClass::Asset => false,
        AccountClass::Liability => true,
        AccountClass::Income | AccountClass::Expense | AccountClass::Equity => {
            return Ok(Err("unsupported_class"));
        }
    };
    let amount = match found {
        Found::Known(amount, _) => *amount,
        Found::Missing(reason) => return Ok(Err(reason)),
    };
    if amount.unit_id != currency {
        return Ok(Err(match unit(state, amount.unit_id)?.kind {
            UnitKind::Currency => "missing_rate",
            UnitKind::Security | UnitKind::Analytical => "missing_value",
        }));
    }
    let Some(owned) = owned(amount, share) else {
        return Ok(Err("inexact_share"));
    };
    Ok(Ok(if negate { owned.checked_neg()? } else { owned }))
}

/// How one in-perimeter component takes part in the total.
enum Count {
    /// It adds this signed amount.
    Adds(Amount),
    /// Its total or its parts count in its place, so it adds nothing.
    Represented,
    /// It is omitted for this reason.
    Omitted(&'static str),
}

/// Count each brokerage total or its parts, never both. A total that counts
/// represents its parts. A total that does not count is represented by its
/// parts when every part is in the household and counts. Otherwise the parts
/// that count still count, and the total and its other parts are omitted
/// with `overlap_unresolved`.
fn resolve_overlaps(counts: &mut BTreeMap<RecordId, Count>, roots: &[&AccountComponentV1]) {
    for total in roots
        .iter()
        .filter(|r| !r.overlapping_components.is_empty())
    {
        let parts = &total.overlapping_components;
        if matches!(counts.get(&total.id), Some(Count::Adds(_))) {
            for part in parts {
                if let Some(count) = counts.get_mut(part) {
                    *count = Count::Represented;
                }
            }
        } else if parts
            .iter()
            .all(|p| matches!(counts.get(p), Some(Count::Adds(_))))
        {
            counts.insert(total.id, Count::Represented);
        } else {
            for id in parts.iter().chain([&total.id]) {
                if let Some(count) = counts.get_mut(id)
                    && !matches!(count, Count::Adds(_))
                {
                    *count = Count::Omitted("overlap_unresolved");
                }
            }
        }
    }
}

/// The household aggregate of `found` on `day`, one entry per component in
/// scope, in component order.
pub(crate) fn aggregate(
    state: &DomainState,
    day: Date,
    found: Vec<(RecordId, Found)>,
) -> Result<AggregateV1, EngineError> {
    let governing = PolicyEngine::governing(state, day)?;
    let perimeter = perimeter(&governing.ownership_perimeter);
    let currency = reporting_currency(state, &governing.reporting_currency)?;
    let mut roots = Vec::new();
    let mut counts = BTreeMap::new();
    let mut components = Vec::new();
    for (component, found) in found {
        let Some(AuthoritativeRecordV1::AccountComponent(root)) = state.records.get(&component)
        else {
            return Err(EngineError::invalid());
        };
        let share = match &perimeter {
            Perimeter::Catalog => Some(ratio(&root.ownership)),
            Perimeter::Unknown => None,
            Perimeter::Known(entries) => match entries.get(&root.account_id) {
                Some(share) => Some(*share),
                None => continue,
            },
        };
        let count = match contribution(state, root, &found, share, currency)? {
            Ok(add) => Count::Adds(add),
            Err(reason) => Count::Omitted(reason),
        };
        let (amount, evidence) = match found {
            Found::Known(amount, evidence) => (SourceValue::Known(amount), evidence),
            Found::Missing(_) => (SourceValue::Unknown, Vec::new()),
        };
        roots.push(root);
        counts.insert(component, count);
        components.push(AggregateComponentV1 {
            component,
            amount,
            evidence,
        });
    }
    // Without a share or a reporting currency nothing counts, so no total
    // and its parts can both count, and each keeps its own reason.
    if currency.is_some() && !matches!(perimeter, Perimeter::Unknown) {
        resolve_overlaps(&mut counts, &roots);
    }
    let mut known = currency.map(|c| Amount::new(0, 0, c)).transpose()?;
    let mut omitted = Vec::new();
    for c in &components {
        match counts[&c.component] {
            Count::Adds(add) => known = known.map(|k| k.checked_add(&add)).transpose()?,
            Count::Represented => {}
            Count::Omitted(reason) => omitted.push(AggregateOmissionV1 {
                component: c.component,
                amount: c.amount.clone(),
                evidence: c.evidence.clone(),
                reason: reason.into(),
            }),
        }
    }
    let obstructions: Vec<String> = omitted
        .iter()
        .map(|o| o.reason.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(match known {
        Some(known) if omitted.is_empty() => AggregateV1::Exact(known.normalized()?),
        Some(known) if omitted.len() < components.len() => AggregateV1::KnownSubtotal {
            known: known.normalized()?,
            omitted,
            obstructions,
        },
        // Without a reporting currency, nothing counts, even when the
        // perimeter leaves out every component in scope.
        None if components.is_empty() => AggregateV1::Indeterminate {
            components,
            obstructions: vec!["unknown_reporting_currency".into()],
        },
        _ => AggregateV1::Indeterminate {
            components,
            obstructions,
        },
    })
}
