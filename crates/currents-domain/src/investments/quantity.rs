//! Security identity, security units and positions. Source identifiers and
//! explicit local mappings identify a security; a display symbol never does.
use crate::catalog::{CatalogEngine, incomplete, version};
use currents_types::*;
use std::collections::BTreeSet;

pub struct InvestmentEngine;

const MAX_IDENTIFIERS: usize = 256;

fn invalid() -> EngineError {
    EngineError::invalid()
}

/// The ordinary activity and association routes refuse a security unit;
/// only the investment engine moves quantities.
pub(crate) fn ordinary_unit(unit: &UnitDefinition) -> Result<(), EngineError> {
    if unit.kind == UnitKind::Security {
        Err(invalid())
    } else {
        Ok(())
    }
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}

fn valid_identifier(identifier: &SecuritySourceIdV1) -> bool {
    text(&identifier.source, 128) && text(&identifier.identifier, 1024)
}

/// Every identifier a revision names: its source identifiers, then its
/// mapped identifiers.
fn identifiers(security: &SecurityV1) -> impl Iterator<Item = &SecuritySourceIdV1> {
    security
        .source_identifiers
        .iter()
        .chain(security.local_mappings.iter().map(|m| &m.identifier))
}

fn revisions(state: &DomainState) -> impl Iterator<Item = &SecurityV1> {
    state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::Security(security) => Some(security),
        _ => None,
    })
}

/// The root of every declared security.
fn roots(state: &DomainState) -> Vec<RecordId> {
    revisions(state)
        .filter(|s| s.prior_revision.is_none())
        .map(|s| s.root)
        .collect()
}

/// A revision keeps every identifier and mapping of the revision it replaces.
fn extends(old: &SecurityV1, new: &SecurityV1) -> bool {
    old.source_identifiers
        .iter()
        .all(|i| new.source_identifiers.contains(i))
        && old
            .local_mappings
            .iter()
            .all(|m| new.local_mappings.contains(m))
}

/// A new revision's own terms, and that no other security already holds any
/// of its identifiers.
fn validate(state: &DomainState, security: &SecurityV1) -> Result<(), EngineError> {
    version(security.schema_version)?;
    if security.id.0.is_nil()
        || security.root.0.is_nil()
        || security.unit_id.0.is_nil()
        || state.records.contains_key(&security.id)
        || security.source_identifiers.is_empty()
        || identifiers(security).count() > MAX_IDENTIFIERS
        || !security.source_identifiers.iter().all(valid_identifier)
        || !security
            .local_mappings
            .iter()
            .all(|m| valid_identifier(&m.identifier) && text(&m.reason, 4096))
        || security
            .display_symbol
            .as_deref()
            .is_some_and(|symbol| !text(symbol, 64))
    {
        return Err(invalid());
    }
    let mut named = BTreeSet::new();
    for identifier in identifiers(security) {
        if !named.insert((identifier.source.as_str(), identifier.identifier.as_str())) {
            return Err(invalid());
        }
    }
    for root in roots(state) {
        if root == security.root {
            continue;
        }
        let other = InvestmentEngine::current(state, root)?;
        if identifiers(&other).any(|i| named.contains(&(i.source.as_str(), i.identifier.as_str())))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// The one component of `account` and its current origin.
fn account(
    state: &DomainState,
    account: AccountId,
) -> Result<(AccountComponentV1, AccountOriginV1), EngineError> {
    let mut found = state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::AccountComponent(c) if c.account_id == account => Some(c),
        _ => None,
    });
    let component = found.next().ok_or_else(incomplete)?;
    if found.next().is_some() {
        return Err(invalid());
    }
    let current = CatalogEngine::current(state, component.id)?;
    Ok((component.clone(), current.origin))
}

/// The declaration replay derives again for `security`, the first revision
/// of a security: the one unit catalog in `next` that declares its unit.
pub(crate) fn declaration_in(
    next: &DomainState,
    security: &SecurityV1,
) -> Result<SecurityDeclarationV1, EngineError> {
    let mut found = next.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::UnitCatalog(catalog) => catalog
            .units
            .iter()
            .find(|unit| unit.id == security.unit_id)
            .map(|unit| (catalog.id, unit)),
        _ => None,
    });
    let (unit_catalog, unit) = found.next().ok_or_else(incomplete)?;
    if found.next().is_some() {
        return Err(invalid());
    }
    Ok(SecurityDeclarationV1 {
        security: security.clone(),
        unit: unit.clone(),
        unit_catalog,
    })
}

impl InvestmentEngine {
    /// Declare a new security and its own security unit. The plan is
    /// metadata only, so it commits before any financial preview that uses
    /// the unit.
    pub fn declare_security(
        state: &DomainState,
        request: &SecurityDeclarationV1,
    ) -> Result<DecisionPlan, EngineError> {
        let security = &request.security;
        let unit = &request.unit;
        validate(state, security)?;
        unit.validate()?;
        if security.prior_revision.is_some()
            || security.id != security.root
            || unit.kind != UnitKind::Security
            || security.unit_id != unit.id
            || request.unit_catalog.0.is_nil()
            || request.unit_catalog == security.id
            || state.records.contains_key(&request.unit_catalog)
        {
            return Err(invalid());
        }
        for record in state.records.values() {
            if let AuthoritativeRecordV1::UnitCatalog(catalog) = record
                && catalog
                    .units
                    .iter()
                    .any(|old| old.id == unit.id || old.native_code == unit.native_code)
            {
                return Err(invalid());
            }
        }
        Ok(DecisionPlan {
            records: vec![
                AuthoritativeRecordV1::UnitCatalog(UnitCatalogV1 {
                    id: request.unit_catalog,
                    version: 1,
                    units: vec![unit.clone()],
                }),
                AuthoritativeRecordV1::Security(security.clone()),
            ],
            invalidations: vec![],
            accounting: AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
                accounts: vec![],
                units: vec![unit.clone()],
            }),
        })
    }

    /// Replace a security's head with a revision that keeps its unit,
    /// identifiers and mappings, and may add identifiers and mappings.
    pub fn revise_security(
        state: &DomainState,
        security: &SecurityV1,
    ) -> Result<DecisionPlan, EngineError> {
        validate(state, security)?;
        let head = Self::current(state, security.root)?;
        if security.prior_revision != Some(head.id)
            || security.id == security.root
            || security.unit_id != head.unit_id
            || !extends(&head, security)
        {
            return Err(invalid());
        }
        Ok(DecisionPlan {
            records: vec![AuthoritativeRecordV1::Security(security.clone())],
            invalidations: vec![],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Security identity revision only".into(),
            }),
        })
    }

    /// The head of the security whose first revision is `root`. Its chain
    /// cannot fork, change unit or drop an identifier or mapping.
    pub fn current(state: &DomainState, root: RecordId) -> Result<SecurityV1, EngineError> {
        let chain: Vec<&SecurityV1> = revisions(state).filter(|s| s.root == root).collect();
        let mut head: Option<&SecurityV1> = None;
        let mut seen = BTreeSet::new();
        loop {
            let prior = head.map(|s| s.id);
            let mut next = chain.iter().filter(|s| s.prior_revision == prior);
            let Some(revision) = next.next() else {
                break;
            };
            if next.next().is_some()
                || !seen.insert(revision.id)
                || prior.is_none() != (revision.id == root)
                || head
                    .is_some_and(|old| old.unit_id != revision.unit_id || !extends(old, revision))
            {
                return Err(invalid());
            }
            version(revision.schema_version)?;
            head = Some(revision);
        }
        if seen.len() != chain.len() {
            return Err(invalid());
        }
        head.map(|s| (*s).clone()).ok_or_else(incomplete)
    }

    /// The security a source identifier names, by its source identifiers or
    /// an explicit local mapping. A display symbol resolves nothing.
    pub fn resolve(
        state: &DomainState,
        identifier: &SecuritySourceIdV1,
    ) -> Result<Option<SecurityV1>, EngineError> {
        let mut found = None;
        for root in roots(state) {
            let head = Self::current(state, root)?;
            if identifiers(&head).any(|i| i == identifier) {
                if found.is_some() {
                    return Err(invalid());
                }
                found = Some(head);
            }
        }
        Ok(found)
    }

    /// Link one security to the external account that holds its quantity,
    /// in its unit, and to one carrying-amount control account. Each account
    /// belongs to at most one position.
    pub fn declare_position(
        state: &DomainState,
        position: &PositionV1,
    ) -> Result<DecisionPlan, EngineError> {
        version(position.schema_version)?;
        if position.id.0.is_nil()
            || state.records.contains_key(&position.id)
            || position.quantity_account == position.carrying_account
        {
            return Err(invalid());
        }
        let security = Self::current(state, position.security)?;
        let (quantity, quantity_origin) = account(state, position.quantity_account)?;
        let (carrying, carrying_origin) = account(state, position.carrying_account)?;
        let carrying_unit = crate::catalog::unit(state, carrying.unit_id)?;
        if quantity.unit_id != security.unit_id
            || quantity_origin != AccountOriginV1::External
            || carrying_unit.kind != UnitKind::Currency
            || !matches!(carrying_origin, AccountOriginV1::Internal(control)
                if control.role == ControlRoleV1::InvestmentCarrying)
        {
            return Err(invalid());
        }
        for record in state.records.values() {
            if let AuthoritativeRecordV1::Position(other) = record
                && (other.quantity_account == position.quantity_account
                    || other.carrying_account == position.carrying_account)
            {
                return Err(invalid());
            }
        }
        Ok(DecisionPlan {
            records: vec![AuthoritativeRecordV1::Position(position.clone())],
            invalidations: vec![],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Position metadata only".into(),
            }),
        })
    }
}
