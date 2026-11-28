//! Reinvested dividends. A reinvested dividend is an income adoption and a
//! buy that commit together.
use super::InvestmentEngine;
use super::quantity::account;
use crate::catalog::version;
use crate::lookup::Lookup;
use crate::recognition::{RecognitionEngine, fact_in, restore};
use currents_types::*;
use std::collections::BTreeSet;

fn invalid() -> EngineError {
    EngineError::invalid()
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
