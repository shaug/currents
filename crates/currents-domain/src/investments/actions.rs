//! Reinvested dividends and corporate actions. A reinvested dividend is an
//! income adoption and a buy that commit together. A split applies its exact
//! ratio to each position's quantity on its effective day, posts no currency
//! and starts the security's next quantity basis; cash paid for a fraction
//! it leaves is a separate recognition. Any other corporate action stays
//! evidence and an unresolved record, and changes no quantity.
use super::InvestmentEngine;
use super::quantity::{Expected, Slot, account, day, entry, known_leg, owner_interpretation};
use crate::catalog::{incomplete, version};
use crate::lookup::Lookup;
use crate::recognition::{RecognitionEngine, fact_in, restore};
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

fn invalid() -> EngineError {
    EngineError::invalid()
}

fn actions(state: &DomainState) -> impl Iterator<Item = &CorporateActionV1> {
    state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::CorporateAction(action) => Some(action),
        _ => None,
    })
}

/// The effective day of each committed split of `security`.
fn split_days(state: &DomainState, security: RecordId) -> Result<Vec<Date>, EngineError> {
    let mut days = Vec::new();
    for action in actions(state) {
        if action.security == security && matches!(action.terms, CorporateActionTermsV1::Split(_)) {
            days.push(day(&action.effective)?);
        }
    }
    Ok(days)
}

/// The quantity basis of `security` on `on`: 1, plus each split effective
/// by then.
pub(crate) fn basis(state: &DomainState, security: RecordId, on: Date) -> Result<u32, EngineError> {
    let splits = split_days(state, security)?
        .into_iter()
        .filter(|effective| *effective <= on)
        .count();
    u32::try_from(splits)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(invalid)
}

/// Each unsupported corporate action on `security` effective by `through`.
pub(crate) fn unresolved(
    state: &DomainState,
    security: RecordId,
    through: Date,
) -> Result<Vec<RecordId>, EngineError> {
    let mut found = Vec::new();
    for action in actions(state) {
        if action.security == security
            && matches!(action.terms, CorporateActionTermsV1::Unsupported(_))
            && day(&action.effective)? <= through
        {
            found.push(action.id);
        }
    }
    Ok(found)
}

/// Refuse a quantity of `security` dated before its latest split: that
/// split already posted the quantity held before it.
pub(crate) fn after_splits(
    state: &DomainState,
    security: RecordId,
    effective: &TemporalValue,
) -> Result<(), EngineError> {
    let Some(latest) = split_days(state, security)?.into_iter().max() else {
        return Ok(());
    };
    if day(effective)? < latest {
        return Err(invalid());
    }
    Ok(())
}

/// The quantity live recognitions post to `position` before `effective`.
/// A posting on or after it is refused: its basis would be unclear.
fn held_before(
    state: &DomainState,
    position: &PositionV1,
    unit: UnitId,
    effective: Date,
) -> Result<Amount, EngineError> {
    let index = state.accounting.as_ref().ok_or_else(incomplete)?;
    let mut held = Amount::new(0, 0, unit)?;
    for (id, entries) in &index.recognitions {
        if index.reversed.contains(id) {
            continue;
        }
        for posting in entries.iter().flat_map(|e| &e.postings) {
            let posting = &posting.original;
            if posting.account_id != position.quantity_account {
                continue;
            }
            if day(&posting.effective)? >= effective {
                return Err(invalid());
            }
            held = held.checked_add(&posting.amount)?;
        }
    }
    held.normalized()
}

/// `held` times the split's ratio, exactly, and that quantity truncated to
/// `exponent` places. An exact quantity with no decimal form within 18
/// places is refused; the engine never rounds it.
fn ratio(held: Amount, split: &SplitV1, exponent: u8) -> Result<(Amount, Amount), EngineError> {
    let (n, d) = (i128::from(split.numerator), i128::from(split.denominator));
    let exact = (exponent..=18)
        .find_map(|scale| held.checked_ratio(n, d, scale).ok())
        .ok_or_else(EngineError::precision)?;
    let places = exact.scale.checked_sub(exponent).ok_or_else(invalid)?;
    let divisor = 10i128
        .checked_pow(u32::from(places))
        .ok_or_else(EngineError::precision)?;
    let kept = exact
        .coefficient
        .checked_div(divisor)
        .ok_or_else(EngineError::precision)?;
    Ok((exact, Amount::new(kept, exponent, held.unit_id)?))
}

/// Each item of `items` by its position; a position named twice is refused.
fn by_position<T>(
    items: &[T],
    position: impl Fn(&T) -> RecordId,
) -> Result<BTreeMap<RecordId, &T>, EngineError> {
    let mut found = BTreeMap::new();
    for item in items {
        if found.insert(position(item), item).is_some() {
            return Err(invalid());
        }
    }
    Ok(found)
}

impl InvestmentEngine {
    /// Plan a reinvested dividend: the posted adoption of the dividend row
    /// on a cash account against an income account, and the buy whose cash
    /// leg pays exactly that dividend out of the same account on the same
    /// day. The plan holds both plans' records, a version 1
    /// `ReinvestmentV1` that links them, and both recognitions, so the cash
    /// account's balance does not change.
    pub fn reinvest(
        state: &DomainState,
        request: &ReinvestmentRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        version(request.schema_version)?;
        let lookup = Lookup::new(state);
        let dividend = &request.dividend;
        let purchase = &request.purchase;
        let adopted = RecognitionEngine::adopt(state, dividend)?;
        let bought = Self::recognize(state, purchase)?;
        let (AccountingIntent::Recognition(income), AccountingIntent::Recognition(buy)) =
            (&adopted.accounting, &bought.accounting)
        else {
            return Err(invalid());
        };
        let [received] = dividend.legs.as_slice() else {
            return Err(invalid());
        };
        let (SourceValue::Known(paid), SourceValue::Known(quantity)) =
            (&purchase.cash, &purchase.quantity)
        else {
            return Err(invalid());
        };
        let movement = received.leg.movement;
        if purchase.side != TradeSideV1::Buy
            || movement.coefficient <= 0
            || paid.leg.account_id != received.leg.account_id
            || paid.leg.movement.normalized()? != movement.checked_neg()?.normalized()?
            || paid.leg.effective != received.leg.effective
        {
            return Err(invalid());
        }
        // The dividend posts the cash it received against one income account.
        let [cash, earned] = income.postings.as_slice() else {
            return Err(invalid());
        };
        let income_role = matches!(account(state, earned.account_id)?.1.origin,
            AccountOriginV1::Internal(control) if control.role == ControlRoleV1::IncomeFlow);
        if cash.account_id != received.leg.account_id
            || cash.amount.normalized()? != movement.normalized()?
            || !income_role
        {
            return Err(invalid());
        }
        // The pair binds three distinct facts and uses each identity once.
        let mut facts = BTreeSet::new();
        for source in [received, paid, quantity]
            .into_iter()
            .flat_map(|leg| &leg.sources)
        {
            if !facts.insert(fact_in(&lookup, source)?) {
                return Err(invalid());
            }
        }
        let mut ids = BTreeSet::new();
        let identities = [
            request.id,
            dividend.adoption_id,
            dividend.event.id,
            purchase.id,
            purchase.event.id,
        ]
        .into_iter()
        .chain(
            dividend
                .event
                .legs
                .iter()
                .chain(&purchase.event.legs)
                .map(|id| RecordId(id.0)),
        )
        .chain([income, buy].into_iter().flat_map(|r| {
            [RecordId(r.id.0), r.lineage]
                .into_iter()
                .chain(r.postings.iter().map(|p| p.id))
        }));
        for id in identities {
            if id.0.is_nil() || !ids.insert(id) || state.records.contains_key(&id) {
                return Err(invalid());
            }
        }
        let mut records = adopted.records.clone();
        records.extend(bought.records.iter().cloned());
        records.push(AuthoritativeRecordV1::Reinvestment(ReinvestmentV1 {
            id: request.id,
            schema_version: 1,
            adoption: dividend.adoption_id,
            trade: purchase.id,
        }));
        let mut invalidations = adopted.invalidations.clone();
        invalidations.extend(bought.invalidations.iter().copied());
        Ok(DecisionPlan {
            records,
            invalidations,
            accounting: AccountingIntent::Recognitions(vec![income.clone(), buy.clone()]),
        })
    }

    /// Plan a corporate action on a security, effective on one day.
    ///
    /// A split names a positive numerator and denominator that differ, the
    /// security's current quantity basis and the next one, and an effective
    /// day after the security's latest split and after every quantity posted
    /// to its positions. For each position, the quantity held before that day
    /// times the ratio, truncated to the security's exponent, is the new
    /// quantity: a position whose quantity changes needs one adjustment
    /// that posts the change against quantity control on that day and
    /// nothing else, and a position left with a fraction needs one cash in
    /// lieu of exactly that fraction, whose own event and leg post the cash
    /// received against disposal clearing. Nothing else is accepted.
    ///
    /// Any other action is recorded with its evidence and no quantity
    /// change; a request that names an adjustment or cash in lieu for it is
    /// refused.
    pub fn act(
        state: &DomainState,
        request: &CorporateActionRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        version(request.schema_version)?;
        let lookup = Lookup::new(state);
        let security = Self::current(state, request.security)?;
        let interpretation = &request.interpretation;
        owner_interpretation(state, interpretation)?;
        let TemporalValue::Date { civil, .. } = request.effective else {
            return Err(invalid());
        };
        if request.id.0.is_nil() || state.records.contains_key(&request.id) {
            return Err(invalid());
        }
        let mut ids = BTreeSet::from([request.id]);
        let mut record = CorporateActionV1 {
            id: request.id,
            schema_version: 1,
            security: request.security,
            effective: request.effective.clone(),
            terms: request.terms.clone(),
            adjustments: vec![],
            cash_in_lieu: vec![],
            interpretation: interpretation.clone(),
        };
        let split = match &request.terms {
            CorporateActionTermsV1::Split(split) => split,
            CorporateActionTermsV1::Unsupported(kind) => {
                if kind.trim().is_empty()
                    || kind.len() > 256
                    || interpretation.evidence.is_empty()
                    || !request.adjustments.is_empty()
                    || !request.cash_in_lieu.is_empty()
                {
                    return Err(invalid());
                }
                return Ok(DecisionPlan {
                    records: vec![AuthoritativeRecordV1::CorporateAction(record)],
                    invalidations: vec![],
                    accounting: AccountingIntent::NoEffect(NoEffectV1 {
                        reason: "Unsupported corporate action kept as evidence".into(),
                    }),
                });
            }
        };
        let prior = split_days(state, request.security)?;
        if split.numerator <= 0
            || split.denominator <= 0
            || split.numerator == split.denominator
            || prior.iter().any(|d| *d >= civil)
            || split.prior_basis != basis(state, request.security, civil)?
            || Some(split.resulting_basis) != split.prior_basis.checked_add(1)
        {
            return Err(invalid());
        }
        let exponent = lookup.unit(security.unit_id)?.exponent;
        let adjustments = by_position(&request.adjustments, |a| a.position)?;
        let cash_in_lieu = by_position(&request.cash_in_lieu, |c| c.position)?;
        let mut recognitions = Vec::new();
        let mut records = Vec::new();
        let mut facts = BTreeSet::new();
        for found in state.records.values() {
            let AuthoritativeRecordV1::Position(position) = found else {
                continue;
            };
            if position.security != request.security {
                continue;
            }
            let held = held_before(state, position, security.unit_id, civil)?;
            if held.coefficient < 0 {
                return Err(invalid());
            }
            let (exact, kept) = ratio(held, split, exponent)?;
            let change = kept.checked_add(&held.checked_neg()?)?.normalized()?;
            let fraction = exact.checked_add(&kept.checked_neg()?)?.normalized()?;
            if change.coefficient != 0 {
                let adjustment = adjustments.get(&position.id).ok_or_else(invalid)?;
                let on = &request.effective;
                let required = Expected::from([
                    (Slot::Quantity, (change, on, on)),
                    (Slot::QuantityControl, (change.checked_neg()?, on, on)),
                ]);
                let r = entry(
                    &lookup,
                    position,
                    None,
                    &[],
                    &adjustment.terms,
                    &required,
                    &mut ids,
                )?;
                record.adjustments.push(SplitAdjustmentV1 {
                    position: position.id,
                    recognition: r.id,
                });
                recognitions.push(r);
            }
            if fraction.coefficient != 0 {
                let paid = cash_in_lieu.get(&position.id).ok_or_else(invalid)?;
                let (carrying, _) = account(state, position.carrying_account)?;
                let leg = &paid.cash.leg;
                let event = &paid.event;
                if paid.fraction.normalized()? != fraction
                    || event.interpretation != SourceValue::Known(interpretation.clone())
                    || !event.relationships.is_empty()
                    || event.legs != [leg.id]
                    || leg.movement.unit_id != carrying.unit_id
                    || leg.movement.coefficient <= 0
                {
                    return Err(invalid());
                }
                let fact = known_leg(
                    &lookup,
                    event.id,
                    interpretation,
                    &paid.cash,
                    UnitKind::Currency,
                )?;
                if !facts.insert(fact) {
                    return Err(invalid());
                }
                for id in [event.id, RecordId(leg.id.0)] {
                    if id.0.is_nil() || !ids.insert(id) || state.records.contains_key(&id) {
                        return Err(invalid());
                    }
                }
                let required = Expected::from([
                    (Slot::Cash, (leg.movement, &leg.effective, &leg.posting)),
                    (
                        Slot::Disposal,
                        (leg.movement.checked_neg()?, &leg.effective, &leg.posting),
                    ),
                ]);
                let r = entry(
                    &lookup,
                    position,
                    Some(&paid.cash),
                    &[&paid.cash],
                    &paid.terms,
                    &required,
                    &mut ids,
                )?;
                record.cash_in_lieu.push(CashInLieuV1 {
                    position: position.id,
                    event_id: event.id,
                    cash: AdoptedFacetV1 {
                        leg_id: leg.id,
                        source: paid.cash.sources[0].clone(),
                    },
                    fraction,
                    recognition: r.id,
                });
                records.push(AuthoritativeRecordV1::FinancialEvent(event.clone()));
                records.push(AuthoritativeRecordV1::FinancialLeg(leg.clone()));
                recognitions.push(r);
            }
        }
        // Every adjustment and cash in lieu belongs to a position that needs it.
        if record.adjustments.len() != adjustments.len()
            || record.cash_in_lieu.len() != cash_in_lieu.len()
        {
            return Err(invalid());
        }
        records.extend(
            recognitions
                .iter()
                .cloned()
                .map(AuthoritativeRecordV1::Recognition),
        );
        records.push(AuthoritativeRecordV1::CorporateAction(record));
        Ok(DecisionPlan {
            records,
            invalidations: vec![],
            accounting: if recognitions.is_empty() {
                AccountingIntent::NoEffect(NoEffectV1 {
                    reason: "No position holds the security before the split".into(),
                })
            } else {
                AccountingIntent::Recognitions(recognitions)
            },
        })
    }
}

/// The plan replay derives again for `link`: the adoption and trade it
/// names are new in `next` and form the pair the planner gives from the
/// prior prefix. Their own records readmit themselves, so the plan holds
/// only the link.
pub(crate) fn readmit_reinvestment(
    prior: &DomainState,
    next: &DomainState,
    link: &ReinvestmentV1,
) -> Result<DecisionPlan, EngineError> {
    if prior.records.contains_key(&link.adoption) || prior.records.contains_key(&link.trade) {
        return Err(invalid());
    }
    let (
        Some(AuthoritativeRecordV1::ActivityAdoption(adoption)),
        Some(AuthoritativeRecordV1::Trade(trade)),
    ) = (
        next.records.get(&link.adoption),
        next.records.get(&link.trade),
    )
    else {
        return Err(invalid());
    };
    let request = ReinvestmentRequestV1 {
        schema_version: link.schema_version,
        id: link.id,
        dividend: restore(next, adoption)?,
        purchase: super::replay::trade_request(next, trade)?,
    };
    let plan = InvestmentEngine::reinvest(prior, &request)?;
    let record = AuthoritativeRecordV1::Reinvestment(link.clone());
    if plan.records.last() != Some(&record) {
        return Err(invalid());
    }
    Ok(DecisionPlan {
        records: vec![record],
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "The adoption and trade readmit their own records".into(),
        }),
    })
}

/// The request whose plan holds `action`, from the records `next` holds
/// for it: each adjustment's recognition, and each cash in lieu's event,
/// leg and recognition.
pub(crate) fn action_request(
    next: &DomainState,
    action: &CorporateActionV1,
) -> Result<CorporateActionRequestV1, EngineError> {
    let terms = |id: RecognitionId| {
        let Some(AuthoritativeRecordV1::Recognition(r)) = next.records.get(&RecordId(id.0)) else {
            return Err(incomplete());
        };
        Ok(RecognitionTermsV1 {
            recognition_id: r.id,
            lineage: r.lineage,
            postings: r.postings.clone(),
            bookkeeping_date: r.bookkeeping_date.clone(),
            date_policy: r.date_policy.clone(),
            unresolved_account: None,
            opening: None,
        })
    };
    let mut adjustments = Vec::new();
    for adjustment in &action.adjustments {
        adjustments.push(SplitAdjustmentRequestV1 {
            position: adjustment.position,
            terms: terms(adjustment.recognition)?,
        });
    }
    let mut cash_in_lieu = Vec::new();
    for paid in &action.cash_in_lieu {
        let (
            Some(AuthoritativeRecordV1::FinancialEvent(event)),
            Some(AuthoritativeRecordV1::FinancialLeg(leg)),
        ) = (
            next.records.get(&paid.event_id),
            next.records.get(&RecordId(paid.cash.leg_id.0)),
        )
        else {
            return Err(incomplete());
        };
        cash_in_lieu.push(CashInLieuRequestV1 {
            position: paid.position,
            event: event.clone(),
            cash: AdoptedLegV1 {
                leg: leg.clone(),
                sources: vec![paid.cash.source.clone()],
            },
            fraction: paid.fraction,
            terms: terms(paid.recognition)?,
        });
    }
    Ok(CorporateActionRequestV1 {
        schema_version: action.schema_version,
        id: action.id,
        security: action.security,
        effective: action.effective.clone(),
        terms: action.terms.clone(),
        adjustments,
        cash_in_lieu,
        interpretation: action.interpretation.clone(),
    })
}
