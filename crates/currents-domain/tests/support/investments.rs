//! Synthetic securities and investment accounts for CUR-021.
//!
//! Unit USD has exponent 2. Security SECA has source identifier `SYN-0001`
//! from source `synthetic-broker`, display symbol `SYNA` and quantity
//! exponent 4. Brokerage account X holds cash account B (external, USD,
//! the recognition fixture's account 10); X itself is not a record.
use super::fixture::*;
use currents_domain::{catalog::CatalogEngine, investments::InvestmentEngine};
use currents_types::*;

pub const BROKER: &str = "synthetic-broker";
pub const OTHER_SOURCE: &str = "synthetic-aggregator";
pub const SECA: u32 = 6000;
pub const SECA_UNIT: u32 = 6001;
pub const SECB: u32 = 6010;
pub const SECB_UNIT: u32 = 6011;
/// External quantity account Q, in SECA.
pub const Q: u32 = 6100;
/// Carrying-amount account K, in USD.
pub const K: u32 = 6110;
/// Quantity control in SECA.
pub const QUANTITY_CONTROL: u32 = 6120;
/// Disposal clearing in USD.
pub const DISPOSAL_CLEARING: u32 = 6130;
/// An external account in USD.
pub const USD_HOLDING: u32 = 6140;
/// An external quantity account in SECB.
pub const QB: u32 = 6150;

pub fn unit_id(n: u32) -> UnitId {
    UnitId(id(n).0)
}

pub fn account_id(n: u32) -> AccountId {
    AccountId(id(n).0)
}

pub fn source_id(source: &str, identifier: &str) -> SecuritySourceIdV1 {
    SecuritySourceIdV1 {
        source: source.into(),
        identifier: identifier.into(),
    }
}

/// A new security with its own unit, declared by catalog record `n + 2`.
pub fn declaration(n: u32, code: &str, identifier: &str, exponent: u8) -> SecurityDeclarationV1 {
    SecurityDeclarationV1 {
        security: SecurityV1 {
            id: id(n),
            schema_version: 1,
            root: id(n),
            prior_revision: None,
            unit_id: unit_id(n + 1),
            source_identifiers: vec![source_id(BROKER, identifier)],
            display_symbol: Some("SYNA".into()),
            local_mappings: vec![],
        },
        unit: UnitDefinition {
            id: unit_id(n + 1),
            exponent,
            native_code: code.into(),
            kind: UnitKind::Security,
        },
        unit_catalog: id(n + 2),
    }
}

pub fn seca() -> SecurityDeclarationV1 {
    declaration(SECA, "SCA", "SYN-0001", 4)
}

pub fn secb() -> SecurityDeclarationV1 {
    declaration(SECB, "SCB", "SYN-0002", 4)
}

/// An account `n` in unit `unit`, with component `n + 1` and revision `n + 2`.
pub fn account(n: u32, unit: UnitId, origin: AccountOriginV1) -> CatalogRequestV1 {
    let (class, sign) = match &origin {
        AccountOriginV1::External => (AccountClass::Asset, NormalSign::Debit),
        AccountOriginV1::Internal(control) => match control.role {
            ControlRoleV1::IncomeFlow => (AccountClass::Income, NormalSign::Credit),
            ControlRoleV1::ExpenseFlow => (AccountClass::Expense, NormalSign::Debit),
            ControlRoleV1::Clearing
            | ControlRoleV1::TaxPrepayment
            | ControlRoleV1::InvestmentCarrying => (AccountClass::Asset, NormalSign::Debit),
            ControlRoleV1::UnresolvedEquity
            | ControlRoleV1::OpeningEquity
            | ControlRoleV1::QuantityControl
            | ControlRoleV1::DisposalClearing => (AccountClass::Equity, NormalSign::Credit),
        },
    };
    let mut request = catalog();
    request.revision_id = id(n + 2);
    request.catalog.component_id = id(n + 1);
    request.catalog.account_id = account_id(n);
    request.catalog.account_class = class;
    request.catalog.normal_sign = sign;
    request.catalog.unit_id = unit;
    request.catalog.display_name = format!("Synthetic account {n}");
    request.ownership.account_id = account_id(n);
    request.origin = origin;
    request
}

pub fn control(role: ControlRoleV1) -> AccountOriginV1 {
    AccountOriginV1::Internal(InternalAccountV1 {
        role,
        created_at: date("2026-01-01"),
    })
}

/// Apply `plan` and declare its metadata in the native index, as a
/// committed metadata declaration would.
pub fn commit_metadata(state: &mut DomainState, plan: &DecisionPlan) {
    apply(state, plan);
    let AccountingIntent::MetadataDeclaration(declared) = &plan.accounting else {
        return;
    };
    let index = state.accounting.as_mut().unwrap();
    for unit in &declared.units {
        index.units.insert(unit.id, unit.clone());
    }
    for account in &declared.accounts {
        index.accounts.insert(
            account.account_id,
            NativeAccountMappingV1 {
                account_id: account.account_id,
                unit_id: account.unit_id,
                account_class: account.account_class,
                path: format!("fixture/{}", account.account_id.0),
            },
        );
    }
}

/// The recognition fixture's monetary state with SECA declared.
pub fn with_seca() -> DomainState {
    let mut state = monetary_state();
    let plan = InvestmentEngine::declare_security(&state, &seca()).unwrap();
    commit_metadata(&mut state, &plan);
    state
}

/// SECA, quantity account Q, carrying account K, quantity control and
/// disposal clearing, each committed.
pub fn world() -> DomainState {
    let mut state = with_seca();
    for request in [
        account(Q, unit_id(SECA_UNIT), AccountOriginV1::External),
        account(K, unit_id(1), control(ControlRoleV1::InvestmentCarrying)),
        account(
            QUANTITY_CONTROL,
            unit_id(SECA_UNIT),
            control(ControlRoleV1::QuantityControl),
        ),
        account(
            DISPOSAL_CLEARING,
            unit_id(1),
            control(ControlRoleV1::DisposalClearing),
        ),
    ] {
        let plan = CatalogEngine::set_complete(&state, &request).unwrap();
        commit_metadata(&mut state, &plan);
    }
    state
}

pub fn position(n: u32, security: u32, quantity: u32, carrying: u32) -> PositionV1 {
    PositionV1 {
        id: id(n),
        schema_version: 1,
        security: id(security),
        quantity_account: account_id(quantity),
        carrying_account: account_id(carrying),
    }
}

/// `amount` whole units of `unit`.
pub fn units(amount: i128, unit: u32) -> Amount {
    Amount::new(amount, 0, unit_id(unit)).unwrap()
}

/// Source evidence `n` for a movement of `amount`, in its own unit.
pub fn quantity_evidence(state: &mut DomainState, n: u32, amount: Amount) -> SourceFacetBindingV1 {
    let mut binding = evidence(state, n, 1, None);
    binding.amount = amount;
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get_mut(&binding.parse_product)
    else {
        panic!("evidence product")
    };
    product.product.facets[0].amount = SourceValue::Known(amount);
    binding
}
