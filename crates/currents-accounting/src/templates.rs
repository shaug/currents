//! The sole typed frozen-to-native mapping; no memo or ambient ID/time sources.
use crate::{PreparedNativeOperation, dates, numbers};
use currents_types::*;
use doubleentry::{
    AccountKind, AccountSpec, Currency, Document, Entry, EntryId, IdempotencyKey, LedgerId,
    LedgerPolicy, store::Op,
};
pub(crate) fn entry(e: &FrozenEntryV1) -> Result<Entry<18>, EngineError> {
    let mut draft = Entry::new(e.key.as_str(), dates::parse(&e.booking_date)?)
        .value_date(dates::parse(&e.value_date)?)
        .document(
            Document::new("currents-decision", e.key.clone())
                .map_err(|_| EngineError::invalid())?
                .with_sha256(e.generated_document.0),
        );
    for p in &e.postings {
        let value = numbers::native_amount(&p.original.amount, &p.unit)?;
        if value.to_string() != p.native_coefficient || value == 0 {
            return Err(EngineError::precision());
        }
        let amount = doubleentry::Amount::<18>::from_minor(
            value.checked_abs().ok_or_else(EngineError::precision)?,
        )
        .map_err(|_| EngineError::precision())?;
        let currency = Currency::new(&p.unit.native_code).map_err(|_| EngineError::invalid())?;
        draft = if value > 0 {
            draft.debit(p.path.as_str(), amount, currency)
        } else {
            draft.credit(p.path.as_str(), amount, currency)
        };
    }
    if let Some(key) = &e.reverses {
        let id = EntryId::derive(
            &LedgerId::new("currents/dataset-v1").map_err(|_| EngineError::invalid())?,
            &IdempotencyKey::new(key).map_err(|_| EngineError::invalid())?,
        );
        draft = draft.reverses(id);
    }
    if !doubleentry::validate(&draft).is_empty() || draft.encoded_len() > 65_535 {
        return Err(EngineError::invalid());
    }
    Ok(draft)
}
pub(crate) fn prepare(
    op: &FrozenNativeOperationV1,
) -> Result<PreparedNativeOperation, EngineError> {
    let operation = match op {
        FrozenNativeOperationV1::Calendar(v) => Op::Period {
            id: "currents-v1".into(),
            start: dates::parse(&v.start)?,
            end: dates::parse(&v.end)?,
        },
        FrozenNativeOperationV1::Policy(v) => Op::Policy {
            policy: LedgerPolicy::default(),
            effective: dates::parse(&v.effective)?,
            actor: "currents-v1".into(),
        },
        FrozenNativeOperationV1::Unit(v) => Op::Currency {
            code: Currency::new(&v.native_code).map_err(|_| EngineError::invalid())?,
            exponent: v.exponent,
            effective: dates::parse("0001-01-01")?,
            actor: "currents-v1".into(),
        },
        FrozenNativeOperationV1::Account(v) => {
            let kind = match v.account_class {
                AccountClass::Asset => AccountKind::Asset,
                AccountClass::Liability => AccountKind::Liability,
                AccountClass::Income => AccountKind::Income,
                AccountClass::Expense => AccountKind::Expense,
                AccountClass::Equity => AccountKind::Equity,
            };
            Op::Register {
                spec: AccountSpec::new(&v.path, kind, dates::parse("0001-01-01")?),
                actor: "currents-v1".into(),
            }
        }
        FrozenNativeOperationV1::Entry(v) => Op::Record(Box::new(entry(v)?)),
    };
    Ok(PreparedNativeOperation { operation })
}
