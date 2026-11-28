//! Household aggregation (C09). The aggregate counts each in-perimeter
//! component once, at its exact governing ownership share, in the reporting
//! currency. A component whose amount, share or value is not known is
//! omitted with one named reason and never counts as zero. A brokerage total
//! counts, or its cash and holdings parts do, never both. Funds a transfer
//! moves between counted components count once: in flight, they are added,
//! and counted by both balances, they are removed.
use crate::activity_predicate::civil_date;
use crate::associations::{IdentityEngine, current_heads};
use crate::catalog::{incomplete, unit};
use crate::policy::{PolicyEngine, dataset};
use crate::position::Boundary;
use crate::reconciliation::facets;
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

/// A component's amount as the position query found it.
pub(crate) enum Found {
    /// The amount in the component's unit, the records it rests on, and
    /// what it counts.
    Known(Amount, Vec<RecordId>, Basis),
    /// Why the query found no amount.
    Missing(&'static str),
}

/// What a known amount counts.
pub(crate) enum Basis {
    /// The selected source balances, each by its source and as-of.
    Reported(Vec<(SourceIdentityV1, Boundary)>),
    /// The anchored balance, which adds the legs its evidence lists.
    Anchored,
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
        Found::Known(amount, ..) => *amount,
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

/// Whether a counted amount includes a transfer leg.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Member {
    Excluded,
    Included,
    Unknown,
}

impl Member {
    /// The least and the most of the leg it can include: none or all.
    fn bounds(self) -> (i8, i8) {
        match self {
            Self::Excluded => (0, 0),
            Self::Included => (1, 1),
            Self::Unknown => (0, 1),
        }
    }

    /// One membership for two balances, known only when they agree.
    fn and(self, other: Self) -> Self {
        if self == other { self } else { Self::Unknown }
    }
}

/// The source lines allocated to `leg`, by the leg itself or by its
/// association's current revision, each with its source and its one reported
/// date, `None` when that date is unknown.
fn lines<'a>(
    state: &'a DomainState,
    head: &AssociationRevisionV1,
    leg: &FinancialLegV1,
) -> Result<Vec<(&'a SourceIdentityV1, Option<Boundary>)>, EngineError> {
    let allocated = leg
        .allocations
        .iter()
        .map(|a| (a.observation, a.facet))
        .chain(
            head.allocations
                .iter()
                .filter(|a| a.leg == leg.id)
                .map(|a| (a.source.observation, a.source.facet)),
        );
    let mut found = Vec::new();
    for (observation, facet) in allocated {
        let Some(AuthoritativeRecordV1::ObservationVersion(version)) =
            state.records.get(&observation)
        else {
            return Err(incomplete());
        };
        let date = facets(state, observation)?
            .into_iter()
            .find(|(f, _)| f.lineage_key == facet)
            .and_then(|(f, _)| Boundary::of(&f.temporal));
        found.push((&version.source, date));
    }
    Ok(found)
}

/// A receipt from `source` reports complete posting-date coverage from
/// `leg`'s posting date through `as_of`.
fn covered(
    state: &DomainState,
    source: &SourceIdentityV1,
    leg: &FinancialLegV1,
    as_of: Boundary,
) -> bool {
    let posted = Boundary::of(&leg.posting);
    state.records.values().any(|record| {
        let AuthoritativeRecordV1::ImportReceipt(receipt) = record else {
            return false;
        };
        &receipt.source == source
            && receipt.reported_coverage.iter().any(|c| {
                c.status == CoverageStatus::Complete
                    && c.scope.basis == DateBasis::Posting
                    && c.scope
                        .start
                        .is_none_or(|start| posted.is_some_and(|p| Boundary::Date(start).by(p)))
                    && c.scope.end.is_none_or(|end| as_of.by(Boundary::Date(end)))
            })
    })
}

/// Whether a counted amount includes `leg` of transfer `head` (ruling G7 on
/// #27). A selected source balance includes it when a line allocated to it
/// from that source is dated on or before the balance's as-of. It excludes
/// it when every such line is dated after the as-of and a receipt from that
/// source reports complete posting-date coverage from the leg's posting date
/// through the as-of. Several selected balances must agree. The anchored
/// balance includes the legs it adds and excludes a leg dated after `day`.
/// A component that another counts in place of, `None`, has no counted
/// amount to decide from. Anything else is unknown.
fn member(
    state: &DomainState,
    day: Date,
    head: &AssociationRevisionV1,
    leg: &FinancialLegV1,
    evidence: &[RecordId],
    basis: Option<&Basis>,
) -> Result<Member, EngineError> {
    let reported = match basis {
        None => return Ok(Member::Unknown),
        Some(Basis::Reported(reported)) => reported,
        Some(Basis::Anchored) if evidence.contains(&RecordId(leg.id.0)) => {
            return Ok(Member::Included);
        }
        Some(Basis::Anchored) => {
            let later = civil_date(DateBasis::Effective, &leg.effective, &leg.posting)
                .is_some_and(|leg_day| leg_day > day);
            return Ok(if later {
                Member::Excluded
            } else {
                Member::Unknown
            });
        }
    };
    let lines = lines(state, head, leg)?;
    let mut joined: Option<Member> = None;
    for (source, as_of) in reported {
        let dates = || lines.iter().filter(|(s, _)| *s == source).map(|(_, d)| *d);
        let each = if dates().any(|d| d.is_some_and(|d| d.by(*as_of))) {
            Member::Included
        } else if dates().all(|d| d.is_some_and(|d| d.after(*as_of)))
            && covered(state, source, leg, *as_of)
        {
            Member::Excluded
        } else {
            Member::Unknown
        };
        joined = Some(joined.map_or(each, |j| j.and(each)));
    }
    Ok(joined.unwrap_or(Member::Unknown))
}

/// A component that counts, or that another counts in place of, by its
/// account. Only a component that counts has a `basis`. `by` is the
/// component whose amount holds its funds: itself, or the total that counts
/// in its place.
struct Counted<'a> {
    share: (i128, i128),
    evidence: &'a [RecordId],
    basis: Option<&'a Basis>,
    by: RecordId,
}

/// A leg that carries a transfer's fee rather than funds moving between
/// accounts: one of its captured allocations has the `Fee` role.
fn fee(
    state: &DomainState,
    head: &AssociationRevisionV1,
    leg: &FinancialLegV1,
) -> Result<bool, EngineError> {
    for edge in head.captured_allocations.iter().filter(|e| e.leg == leg.id) {
        let facet = IdentityEngine::retained_facet(state, &edge.source, leg.movement.unit_id)?;
        if facet.role == RelationshipFacetRoleV1::Fee {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What the current transfers between counted components add to the total:
/// the least and the most, or `None` when an amount that matters is not
/// known. For each receiving leg, the sending side's balances include the
/// outflow or not, and the receiving balance includes the receipt or not. An
/// outflow without its receipt adds the receipt, a receipt without its
/// outflow removes it, and agreement adds nothing. The amount is the
/// receiving leg's movement at the receiving component's share. Fee legs
/// move no funds between accounts, and funds that stay inside one counted
/// amount adjust nothing.
fn transfers(
    state: &DomainState,
    day: Date,
    currency: UnitId,
    counted: &BTreeMap<AccountId, Counted<'_>>,
) -> Result<Option<(Amount, Amount)>, EngineError> {
    let mut low = Amount::new(0, 0, currency)?;
    let mut high = low;
    if counted.is_empty() {
        return Ok(Some((low, high)));
    }
    let heads = current_heads(state)?;
    for head in heads
        .values()
        .filter(|h| h.kind == RelationshipKind::Transfer)
    {
        let mut sending: Option<Member> = None;
        let mut senders = BTreeSet::new();
        let mut receiving = Vec::new();
        for id in &head.legs {
            let Some(AuthoritativeRecordV1::FinancialLeg(leg)) = state.records.get(&RecordId(id.0))
            else {
                return Err(incomplete());
            };
            let Some(c) = counted.get(&leg.account_id) else {
                continue;
            };
            if fee(state, head, leg)? {
                continue;
            }
            let m = member(state, day, head, leg, c.evidence, c.basis)?;
            match leg.movement.coefficient.signum() {
                -1 => {
                    sending = Some(sending.map_or(m, |s| s.and(m)));
                    senders.insert(c.by);
                }
                1 => receiving.push((leg, c.share, m, c.by)),
                _ => {}
            }
        }
        let Some(sending) = sending else {
            continue;
        };
        for (leg, share, received, by) in receiving {
            if senders.len() == 1 && senders.contains(&by) {
                continue;
            }
            // Each bound is 0 or 1, so each difference is -1, 0 or 1.
            let (s, r) = (sending.bounds(), received.bounds());
            let (least, most) = (s.0.saturating_sub(r.1), s.1.saturating_sub(r.0));
            if (least, most) == (0, 0) {
                continue;
            }
            let known = matches!(leg.unresolved_residual, SourceValue::Absent)
                && leg.movement.unit_id == currency;
            let Some(amount) = owned(leg.movement, share).filter(|_| known) else {
                return Ok(None);
            };
            let times = |k: i8| match k {
                1 => Ok(Some(amount)),
                -1 => amount.checked_neg().map(Some),
                _ => Ok(None),
            };
            if let Some(add) = times(least)? {
                low = low.checked_add(&add)?;
            }
            if let Some(add) = times(most)? {
                high = high.checked_add(&add)?;
            }
        }
    }
    Ok(Some((low, high)))
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
    let mut held = Vec::new();
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
        let (amount, evidence, basis) = match found {
            Found::Known(amount, evidence, basis) => {
                (SourceValue::Known(amount), evidence, Some(basis))
            }
            Found::Missing(_) => (SourceValue::Unknown, Vec::new(), None),
        };
        roots.push(root);
        counts.insert(component, count);
        held.push((root.account_id, share, basis));
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
    let mut counted = BTreeMap::new();
    for (c, (account, share, basis)) in components.iter().zip(&held) {
        match counts[&c.component] {
            Count::Adds(add) => {
                known = known.map(|k| k.checked_add(&add)).transpose()?;
                if let (Some(share), Some(basis)) = (share, basis) {
                    counted.insert(
                        *account,
                        Counted {
                            share: *share,
                            evidence: &c.evidence,
                            basis: Some(basis),
                            by: c.component,
                        },
                    );
                }
            }
            // Another component counts its funds, so a transfer leg on it
            // still decides the total, though its membership is unknown.
            Count::Represented => {
                let total = roots.iter().find(|t| {
                    t.overlapping_components.contains(&c.component)
                        && matches!(counts.get(&t.id), Some(Count::Adds(_)))
                });
                if let Some(share) = share {
                    counted.insert(
                        *account,
                        Counted {
                            share: *share,
                            evidence: &c.evidence,
                            basis: None,
                            by: total.map_or(c.component, |t| t.id),
                        },
                    );
                }
            }
            Count::Omitted(reason) => omitted.push(AggregateOmissionV1 {
                component: c.component,
                amount: c.amount.clone(),
                evidence: c.evidence.clone(),
                reason: reason.into(),
            }),
        }
    }
    let mut obstructions: Vec<String> = omitted
        .iter()
        .map(|o| o.reason.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let (Some(known), Some(currency)) = (known, currency) else {
        // Without a reporting currency, nothing counts, even when the
        // perimeter leaves out every component in scope.
        if components.is_empty() {
            obstructions = vec!["unknown_reporting_currency".into()];
        }
        return Ok(AggregateV1::Indeterminate {
            components,
            obstructions,
        });
    };
    let adjusted = transfers(state, day, currency, &counted)?;
    Ok(match adjusted {
        Some((low, high)) if omitted.is_empty() => {
            let low = known.checked_add(&low)?.normalized()?;
            let high = known.checked_add(&high)?.normalized()?;
            if low == high {
                AggregateV1::Exact(low)
            } else {
                AggregateV1::Range { low, high }
            }
        }
        Some((low, high)) if low == high && omitted.len() < components.len() => {
            AggregateV1::KnownSubtotal {
                known: known.checked_add(&low)?.normalized()?,
                omitted,
                obstructions,
            }
        }
        // Every component is omitted, so no transfer adjusts anything.
        Some((low, high)) if low == high => AggregateV1::Indeterminate {
            components,
            obstructions,
        },
        // A transfer's alternatives are not one amount beside an omission,
        // or an amount they need is not known.
        _ => {
            obstructions.push("transfer_membership".into());
            obstructions.sort();
            AggregateV1::Indeterminate {
                components,
                obstructions,
            }
        }
    })
}
