//! Validated immutable account catalog revisions; native identity never changes.
use currents_types::*;
use std::collections::BTreeSet;

pub struct CatalogEngine;
pub(crate) fn incomplete() -> EngineError {
    EngineError {
        class: ErrorClass::IncompleteBasis,
        next_action: SafeNextAction::InspectEvidence,
        affected_scope: vec![],
    }
}
pub(crate) fn version(version: u32) -> Result<(), EngineError> {
    if version == 1 {
        Ok(())
    } else {
        Err(EngineError::version())
    }
}
/// `amount` converted between a source's convention and the journal's by
/// the account's normal sign. The journal is debit-positive, and a source
/// reports a credit-normal balance, such as a card's amount owed, as
/// positive. The conversion is its own inverse.
pub(crate) fn journal_sign(sign: NormalSign, amount: Amount) -> Result<Amount, EngineError> {
    match sign {
        NormalSign::Debit => Ok(amount),
        NormalSign::Credit => amount.checked_neg(),
    }
}
pub(crate) fn known_time(time: &TemporalValue) -> bool {
    matches!(
        time,
        TemporalValue::Date { .. } | TemporalValue::Instant { .. }
    )
}
pub(crate) fn unit(state: &DomainState, id: UnitId) -> Result<UnitDefinition, EngineError> {
    let mut found = None;
    for record in state.records.values() {
        if let AuthoritativeRecordV1::UnitCatalog(catalog) = record {
            for unit in &catalog.units {
                if unit.id == id {
                    unit.validate()?;
                    if found.as_ref().is_some_and(|old| old != unit) {
                        return Err(EngineError::invalid());
                    }
                    found = Some(unit.clone());
                }
            }
        }
    }
    found.ok_or_else(incomplete)
}
fn origin(
    origin: &AccountOriginV1,
    class: AccountClass,
) -> Result<SourceValue<TemporalValue>, EngineError> {
    match origin {
        AccountOriginV1::External => {
            if !matches!(class, AccountClass::Asset | AccountClass::Liability) {
                return Err(EngineError::invalid());
            }
            Ok(SourceValue::Unknown)
        }
        AccountOriginV1::Internal(control) => {
            let expected = match control.role {
                ControlRoleV1::UnresolvedEquity
                | ControlRoleV1::OpeningEquity
                | ControlRoleV1::QuantityControl
                | ControlRoleV1::DisposalClearing => AccountClass::Equity,
                ControlRoleV1::IncomeFlow => AccountClass::Income,
                ControlRoleV1::ExpenseFlow => AccountClass::Expense,
                ControlRoleV1::Clearing
                | ControlRoleV1::TaxPrepayment
                | ControlRoleV1::InvestmentCarrying => AccountClass::Asset,
            };
            if class != expected || !known_time(&control.created_at) {
                return Err(EngineError::invalid());
            }
            Ok(SourceValue::Known(control.created_at.clone()))
        }
    }
}
/// Each control role accepts only the unit kind it is for: quantity control
/// a security unit, carrying amount and disposal clearing a currency, and
/// every other role any unit but a security.
fn role_unit(origin: &AccountOriginV1, unit: &UnitDefinition) -> Result<(), EngineError> {
    let AccountOriginV1::Internal(control) = origin else {
        return Ok(());
    };
    let fits = match control.role {
        ControlRoleV1::QuantityControl => unit.kind == UnitKind::Security,
        ControlRoleV1::InvestmentCarrying | ControlRoleV1::DisposalClearing => {
            unit.kind == UnitKind::Currency
        }
        _ => unit.kind != UnitKind::Security,
    };
    if fits {
        Ok(())
    } else {
        Err(EngineError::invalid())
    }
}
fn same_identity(root: &AccountComponentV1, catalog: &SetAccountCatalogV1) -> bool {
    root.id == catalog.component_id
        && root.account_id == catalog.account_id
        && root.account_class == catalog.account_class
        && root.unit_id == catalog.unit_id
        && root.normal_sign == catalog.normal_sign
}
fn validate_aliases(state: &DomainState, request: &SetAccountCatalogV1) -> Result<(), EngineError> {
    if request.aliases.len() > 256 {
        return Err(EngineError::invalid());
    }
    for (i, alias) in request.aliases.iter().enumerate() {
        if alias.source.source.is_empty()
            || alias.source.connection_id.0.is_nil()
            || alias.original_id.is_empty()
            || alias.original_id.len() > 1024
            || alias.source.source.len() > 128
            || request.aliases[..i].contains(alias)
        {
            return Err(EngineError::invalid());
        }
        for record in state.records.values() {
            let binding = match record {
                AuthoritativeRecordV1::AccountComponent(a) => {
                    Some((a.account_id, a.aliases.as_slice()))
                }
                AuthoritativeRecordV1::AccountCatalogRevision(r) => {
                    Some((r.account_id, r.catalog.aliases.as_slice()))
                }
                AuthoritativeRecordV1::AccountAlias(a)
                    if a.alias == *alias && a.account_id != request.account_id =>
                {
                    return Err(EngineError::invalid());
                }
                _ => None,
            };
            if binding.is_some_and(|(account, aliases)| {
                account != request.account_id && aliases.contains(alias)
            }) {
                return Err(EngineError::invalid());
            }
        }
    }
    Ok(())
}
/// The parts a new total component overlaps, sorted. Each part is an
/// already declared, open component that is not itself a total, that no
/// other total lists, and whose ownership share equals the total's.
fn overlaps(state: &DomainState, request: &CatalogRequestV1) -> Result<Vec<RecordId>, EngineError> {
    let parts: BTreeSet<RecordId> = request.overlaps.iter().copied().collect();
    if parts.len() != request.overlaps.len() || parts.len() > 256 {
        return Err(EngineError::invalid());
    }
    let share = |o: &OwnershipV1| (u128::from(o.numerator.0), u128::from(o.denominator.0));
    let (numerator, denominator) = share(&request.ownership);
    for part in &parts {
        let Some(AuthoritativeRecordV1::AccountComponent(root)) = state.records.get(part) else {
            return Err(EngineError::invalid());
        };
        let current = CatalogEngine::current(state, *part)?;
        let (n, d) = share(&root.ownership);
        let listed = state.records.values().any(|r| {
            matches!(r, AuthoritativeRecordV1::AccountComponent(t) if t.overlapping_components.contains(part))
        });
        if current.catalog.lifecycle == Lifecycle::Closed
            || !root.overlapping_components.is_empty()
            || listed
            || n * denominator != numerator * d
        {
            return Err(EngineError::invalid());
        }
    }
    Ok(parts.into_iter().collect())
}
impl CatalogEngine {
    /// Legacy incomplete route cannot establish creation or revision provenance.
    pub fn set(_: &DomainState, _: &SetAccountCatalogV1) -> Result<DecisionPlan, EngineError> {
        Err(EngineError::version())
    }

    pub fn current(
        state: &DomainState,
        component: RecordId,
    ) -> Result<AccountCatalogRevisionV1, EngineError> {
        let Some(AuthoritativeRecordV1::AccountComponent(root)) = state.records.get(&component)
        else {
            return Err(EngineError::invalid());
        };
        let revisions: Vec<_> = state
            .records
            .iter()
            .filter_map(|(id, record)| match record {
                AuthoritativeRecordV1::AccountCatalogRevision(r) if r.component_id == component => {
                    Some((id, r))
                }
                _ => None,
            })
            .collect();
        if revisions.is_empty() {
            return Err(incomplete());
        }
        let mut prior = None;
        let mut seen = BTreeSet::new();
        let mut current: Option<&AccountCatalogRevisionV1> = None;
        loop {
            let mut next = revisions.iter().filter(|(_, r)| r.prior_revision == prior);
            let Some((id, revision)) = next.next() else {
                break;
            };
            if next.next().is_some()
                || !seen.insert(revision.id)
                || **id != revision.id
                || revision.id == component
                || revision.id.0.is_nil()
                || revision.account_id != root.account_id
                || !same_identity(root, &revision.catalog)
            {
                return Err(EngineError::invalid());
            }
            version(revision.schema_version)?;
            let boundary = origin(&revision.origin, root.account_class)?;
            if matches!(revision.origin, AccountOriginV1::Internal(_))
                && root.opening_boundary != boundary
            {
                return Err(EngineError::invalid());
            }
            if let Some(old) = current {
                if old.origin != revision.origin
                    || old
                        .catalog
                        .aliases
                        .iter()
                        .any(|a| !revision.catalog.aliases.contains(a))
                {
                    return Err(EngineError::invalid());
                }
            } else if root
                .aliases
                .iter()
                .any(|a| !revision.catalog.aliases.contains(a))
            {
                return Err(EngineError::invalid());
            }
            validate_aliases(state, &revision.catalog)?;
            current = Some(revision);
            prior = Some(revision.id);
        }
        if seen.len() != revisions.len() {
            return Err(EngineError::invalid());
        }
        current.cloned().ok_or_else(incomplete)
    }
    pub fn set_complete(
        state: &DomainState,
        request: &CatalogRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        match request.schema_version {
            1 if request.overlaps.is_empty() => {}
            1 => return Err(EngineError::invalid()),
            2 => {}
            _ => return Err(EngineError::version()),
        }
        let c = &request.catalog;
        let expected_sign = match c.account_class {
            AccountClass::Asset | AccountClass::Expense => NormalSign::Debit,
            _ => NormalSign::Credit,
        };
        if c.component_id.0.is_nil()
            || c.account_id.0.is_nil()
            || request.revision_id.0.is_nil()
            || c.component_id == request.revision_id
            || state.records.contains_key(&request.revision_id)
            || c.normal_sign != expected_sign
            || c.display_name.trim().is_empty()
            || c.display_name.len() > 1024
            || request.ownership.account_id != c.account_id
            || request.ownership.denominator.0 == 0
            || request.ownership.numerator.0 > request.ownership.denominator.0
        {
            return Err(EngineError::invalid());
        }
        let u = unit(state, c.unit_id)?;
        let boundary = origin(&request.origin, c.account_class)?;
        role_unit(&request.origin, &u)?;
        validate_aliases(state, c)?;
        let mut records = vec![];
        let accounting = if let Some(record) = state.records.get(&c.component_id) {
            let AuthoritativeRecordV1::AccountComponent(root) = record else {
                return Err(EngineError::invalid());
            };
            // Overlaps are declared only when the total is created.
            if !same_identity(root, c)
                || root.ownership != request.ownership
                || !request.overlaps.is_empty()
            {
                return Err(EngineError::invalid());
            }
            let has_revision = state.records.values().any(|r| matches!(r, AuthoritativeRecordV1::AccountCatalogRevision(r) if r.component_id == c.component_id));
            if has_revision {
                let current = Self::current(state, c.component_id)?;
                if request.prior_revision != Some(current.id)
                    || request.origin != current.origin
                    || current
                        .catalog
                        .aliases
                        .iter()
                        .any(|a| !c.aliases.contains(a))
                {
                    return Err(EngineError::invalid());
                }
            } else if request.prior_revision.is_some()
                || root.aliases.iter().any(|a| !c.aliases.contains(a))
                || matches!(request.origin, AccountOriginV1::Internal(_))
                    && root.opening_boundary != boundary
            {
                return Err(EngineError::invalid());
            }
            AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Immutable catalog metadata revision only".into(),
            })
        } else {
            if request.prior_revision.is_some() || state.records.values().any(|r| matches!(r, AuthoritativeRecordV1::AccountComponent(a) if a.account_id == c.account_id)) { return Err(EngineError::invalid()); }
            if request.origin == AccountOriginV1::External {
                let datasets: Vec<_> = state
                    .records
                    .values()
                    .filter_map(|r| match r {
                        AuthoritativeRecordV1::Dataset(d) => Some(d),
                        _ => None,
                    })
                    .collect();
                if datasets.len() != 1 {
                    return Err(incomplete());
                }
                // The owner-approved complete creation request is itself an
                // explicit new assertion. Prior account FKs cannot predeclare a
                // nonexistent account. Every applicable accepted assertion must
                // still agree; no default share or latest-perimeter selection.
                let assertions = state
                    .records
                    .values()
                    .filter_map(|r| match r {
                        AuthoritativeRecordV1::Dataset(d) => Some(&d.ownership_perimeter),
                        AuthoritativeRecordV1::OwnershipPerimeter(p) => Some(&p.ownership),
                        _ => None,
                    })
                    .flatten()
                    .filter(|o| o.account_id == c.account_id);
                for assertion in assertions {
                    if assertion != &request.ownership {
                        return Err(incomplete());
                    }
                }
            }
            let account = AccountComponentV1 {
                id: c.component_id,
                account_id: c.account_id,
                account_class: c.account_class,
                unit_id: c.unit_id,
                normal_sign: c.normal_sign,
                aliases: c.aliases.clone(),
                ownership: request.ownership.clone(),
                coverage: vec![],
                opening_boundary: boundary,
                overlapping_components: overlaps(state, request)?,
                lifecycle: c.lifecycle,
                display_name: c.display_name.clone(),
            };
            records.push(AuthoritativeRecordV1::AccountComponent(account.clone()));
            AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
                accounts: vec![account],
                units: vec![u],
            })
        };
        records.push(AuthoritativeRecordV1::AccountCatalogRevision(
            AccountCatalogRevisionV1 {
                id: request.revision_id,
                schema_version: 1,
                component_id: c.component_id,
                account_id: c.account_id,
                prior_revision: request.prior_revision,
                catalog: c.clone(),
                origin: request.origin.clone(),
            },
        ));
        Ok(DecisionPlan {
            records,
            invalidations: vec![c.component_id],
            accounting,
        })
    }
}
