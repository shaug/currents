//! Immutable explicit catalog mapping. Never truncate a ticker or UUID.
use currents_types::*;
pub(crate) fn check_unit(
    unit: &UnitDefinition,
    known: &std::collections::BTreeMap<UnitId, UnitDefinition>,
) -> Result<(), EngineError> {
    unit.validate()?;
    if (unit.native_code == "USD" && (unit.exponent != 2 || unit.kind != UnitKind::Currency))
        || unit.kind == UnitKind::Analytical
    {
        return Err(EngineError::precision());
    }
    if let Some(old) = known.get(&unit.id)
        && old != unit
    {
        return Err(EngineError::version());
    }
    if known
        .values()
        .any(|old| old.id != unit.id && old.native_code == unit.native_code)
    {
        return Err(EngineError::invalid());
    }
    Ok(())
}
pub(crate) fn mapping(account: &AccountComponentV1) -> Result<NativeAccountMappingV1, EngineError> {
    let expected = match account.account_class {
        AccountClass::Asset | AccountClass::Expense => NormalSign::Debit,
        _ => NormalSign::Credit,
    };
    if account.normal_sign != expected {
        return Err(EngineError::invalid());
    }
    Ok(NativeAccountMappingV1 {
        account_id: account.account_id,
        unit_id: account.unit_id,
        account_class: account.account_class,
        path: format!("Currents:A{}", account.account_id.0.simple()),
    })
}
