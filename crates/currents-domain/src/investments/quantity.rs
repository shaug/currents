//! Security identity, security units, positions and trades. Source
//! identifiers and explicit local mappings identify a security; a display
//! symbol never does.
use crate::catalog::{CatalogEngine, incomplete, known_time, version};
use crate::lookup::Lookup;
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

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

/// The one component of `account` and its current catalog revision.
fn account(
    state: &DomainState,
    account: AccountId,
) -> Result<(AccountComponentV1, AccountCatalogRevisionV1), EngineError> {
    let mut found = state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::AccountComponent(c) if c.account_id == account => Some(c),
        _ => None,
    });
    let component = found.next().ok_or_else(incomplete)?;
    if found.next().is_some() {
        return Err(invalid());
    }
    let current = CatalogEngine::current(state, component.id)?;
    Ok((component.clone(), current))
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
        let (quantity, quantity_catalog) = account(state, position.quantity_account)?;
        let (carrying, carrying_catalog) = account(state, position.carrying_account)?;
        let carrying_unit = crate::catalog::unit(state, carrying.unit_id)?;
        if quantity.unit_id != security.unit_id
            || quantity_catalog.origin != AccountOriginV1::External
            || carrying_unit.kind != UnitKind::Currency
            || !matches!(carrying_catalog.origin, AccountOriginV1::Internal(control)
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

/// What one position holds through a date: its recognized quantity and
/// carrying amount, the trades by then whose quantity or cash side is
/// missing, the latest snapshot quantity, any current absence assertion and
/// the unmapped positions of the latest snapshot that covers it. A snapshot
/// never changes the recognized quantity. A read result, not a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionHoldingsV1 {
    pub position: RecordId,
    pub through: Date,
    pub quantity: Amount,
    pub carrying: Amount,
    pub missing_quantity: Vec<RecordId>,
    pub missing_cash: Vec<RecordId>,
    pub snapshot: Option<super::holdings::SnapshotQuantityV1>,
    pub absence: Option<super::holdings::SnapshotAbsenceV1>,
    pub unmapped: Option<super::holdings::SnapshotUnmappedV1>,
}

/// Whether `recognition` posts a security quantity. Until CUR-022 (#31)
/// defines how a trade's quantity is corrected, corrections and segment
/// replacements refuse such a recognition.
pub(crate) fn holds_quantity(
    state: &DomainState,
    recognition: &RecognitionV1,
) -> Result<bool, EngineError> {
    for posting in &recognition.postings {
        if crate::catalog::unit(state, posting.amount.unit_id)?.kind == UnitKind::Security {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Where a trade posting goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    /// The cash leg's external account.
    Cash,
    /// The position's carrying-amount account.
    Carrying,
    /// A fee expense account.
    Fee,
    /// A disposal-clearing account.
    Disposal,
    /// The position's quantity account.
    Quantity,
    /// A quantity-control account.
    QuantityControl,
}

/// The amount and dates each slot must post.
type Expected<'a> = BTreeMap<Slot, (Amount, &'a TemporalValue, &'a TemporalValue)>;

/// A side's leg when it is known; refused unless known or missing.
fn side(value: &SourceValue<AdoptedLegV1>) -> Result<Option<&AdoptedLegV1>, EngineError> {
    match value {
        SourceValue::Known(leg) => Ok(Some(leg)),
        SourceValue::Missing => Ok(None),
        _ => Err(invalid()),
    }
}

/// The unit of `amount`, which must fit the unit's exponent. The engine
/// never rounds.
fn exact(lookup: &Lookup, amount: &Amount) -> Result<UnitDefinition, EngineError> {
    let unit = lookup.unit(amount.unit_id)?;
    amount.checked_rescale(unit.exponent)?;
    Ok(unit)
}

/// Whether an adoption, an opening's inclusion map or a recognition already
/// binds the fact `(root, facet)`.
fn owned(lookup: &Lookup, fact: (RecordId, RecordId)) -> Result<bool, EngineError> {
    let binds = |observation: RecordId, facet: RecordId| -> Result<bool, EngineError> {
        Ok(facet == fact.1 && lookup.root(observation)?.0 == fact.0)
    };
    for record in lookup.state.records.values() {
        match record {
            AuthoritativeRecordV1::ActivityAdoption(a) => {
                let opening = a.opening.iter().flat_map(|o| &o.included_activity);
                for source in a.facets.iter().map(|f| &f.source).chain(opening) {
                    if binds(source.observation, source.facet)? {
                        return Ok(true);
                    }
                }
            }
            AuthoritativeRecordV1::Recognition(r) => {
                for allocation in &r.evidence_allocations {
                    if binds(allocation.observation, allocation.facet)? {
                        return Ok(true);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(false)
}

/// Check one known side's leg: it belongs to the request's event, moves an
/// open external account in a unit of `kind` by exactly its one movement
/// source, and binds a fact nothing else binds. Returns that fact.
fn known_leg(
    lookup: &Lookup,
    request: &TradeRequestV1,
    item: &AdoptedLegV1,
    kind: UnitKind,
) -> Result<(RecordId, RecordId), EngineError> {
    let leg = &item.leg;
    let [source] = item.sources.as_slice() else {
        return Err(invalid());
    };
    let allocation = EvidenceAllocationV1 {
        observation: source.observation,
        facet: source.facet,
        amount: source.amount,
    };
    if leg.id.0.is_nil()
        || leg.event_id != request.event.id
        || !leg.relationships.is_empty()
        || leg.unresolved_residual != SourceValue::Absent
        || leg.interpretation != SourceValue::Known(request.interpretation.clone())
        || leg.allocations != [allocation]
        || leg.movement.coefficient == 0
        || leg.movement.normalized()? != source.amount.normalized()?
        || !known_time(&leg.effective)
        || !known_time(&leg.posting)
    {
        return Err(invalid());
    }
    let unit = exact(lookup, &leg.movement)?;
    let (component, catalog) = account(lookup.state, leg.account_id)?;
    if unit.kind != kind
        || component.unit_id != unit.id
        || catalog.origin != AccountOriginV1::External
        || catalog.catalog.lifecycle != Lifecycle::Open
    {
        return Err(invalid());
    }
    let (fact, _) = crate::recognition::facet_of(lookup, source, true)?;
    if lookup.root(source.observation)?.1 != ObservationKind::Movement {
        return Err(invalid());
    }
    if owned(lookup, fact)? {
        return Err(EngineError {
            class: ErrorClass::DuplicateRecognition,
            ..invalid()
        });
    }
    Ok(fact)
}

/// The slot a posting to `account` fills.
fn slot(
    state: &DomainState,
    position: &PositionV1,
    cash: Option<&AdoptedLegV1>,
    account_id: AccountId,
) -> Result<Slot, EngineError> {
    if cash.is_some_and(|c| c.leg.account_id == account_id) {
        return Ok(Slot::Cash);
    }
    if account_id == position.carrying_account {
        return Ok(Slot::Carrying);
    }
    if account_id == position.quantity_account {
        return Ok(Slot::Quantity);
    }
    match account(state, account_id)?.1.origin {
        AccountOriginV1::Internal(control) => match control.role {
            ControlRoleV1::ExpenseFlow => Ok(Slot::Fee),
            ControlRoleV1::DisposalClearing => Ok(Slot::Disposal),
            ControlRoleV1::QuantityControl => Ok(Slot::QuantityControl),
            _ => Err(invalid()),
        },
        AccountOriginV1::External => Err(invalid()),
    }
}

/// The postings each known side requires. A buy's cash moves to the
/// carrying amount, fee included. A sale's gross proceeds go to disposal
/// clearing and its fee to expense once. A quantity moves against quantity
/// control.
fn expected<'a>(
    request: &'a TradeRequestV1,
    cash: Option<&'a AdoptedLegV1>,
    quantity: Option<&'a AdoptedLegV1>,
) -> Result<Expected<'a>, EngineError> {
    let buy = request.side == TradeSideV1::Buy;
    let fee = request.fee;
    if fee.coefficient < 0 || (cash.is_none() && fee.coefficient != 0) {
        return Err(invalid());
    }
    let mut expected = Expected::new();
    if let Some(cash) = cash {
        let leg = &cash.leg;
        let dates = (&leg.effective, &leg.posting);
        let movement = leg.movement;
        let gross = movement.checked_add(&fee)?;
        if buy {
            // The cost, the cash paid less the fee, is positive.
            if movement.coefficient >= 0 || gross.coefficient >= 0 {
                return Err(invalid());
            }
            expected.insert(Slot::Carrying, (movement.checked_neg()?, dates.0, dates.1));
        } else {
            if movement.coefficient <= 0 {
                return Err(invalid());
            }
            if fee.coefficient != 0 {
                expected.insert(Slot::Fee, (fee, dates.0, dates.1));
            }
            expected.insert(Slot::Disposal, (gross.checked_neg()?, dates.0, dates.1));
        }
        expected.insert(Slot::Cash, (movement, dates.0, dates.1));
    }
    if let Some(quantity) = quantity {
        let leg = &quantity.leg;
        let movement = leg.movement;
        if (movement.coefficient > 0) != buy {
            return Err(invalid());
        }
        expected.insert(Slot::Quantity, (movement, &leg.effective, &leg.posting));
        expected.insert(
            Slot::QuantityControl,
            (movement.checked_neg()?, &leg.effective, &leg.posting),
        );
    }
    Ok(expected)
}

/// The civil day of `value`; an instant or unknown time has none here.
fn day(value: &TemporalValue) -> Result<Date, EngineError> {
    match value {
        TemporalValue::Date { civil, .. } => Ok(*civil),
        _ => Err(incomplete()),
    }
}

impl InvestmentEngine {
    /// Plan a buy or sale of a position's security. The known sides post in
    /// one recognition, so they freeze as one native entry and commit
    /// together. A missing side posts nothing and stays explicit in the
    /// trade record. Each unit balances on its own; the engine refuses a
    /// posting in another unit than its account's, an amount beyond its
    /// unit's exponent and a request with neither side known.
    pub fn recognize(
        state: &DomainState,
        request: &TradeRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        let lookup = Lookup::new(state);
        version(request.schema_version)?;
        let Some(AuthoritativeRecordV1::Position(position)) = state.records.get(&request.position)
        else {
            return Err(incomplete());
        };
        let security = Self::current(state, position.security)?;
        let (carrying, _) = account(state, position.carrying_account)?;
        let index = state.accounting.as_ref().ok_or_else(incomplete)?;
        if index.prefix.through != state.through || !index.calendar_declared {
            return Err(incomplete());
        }
        let terms = &request.terms;
        let policy = state
            .dataset_policies
            .get(&(
                request.governing_policy.id,
                request.governing_policy.version,
            ))
            .ok_or_else(incomplete)?;
        let dates = index
            .date_policies
            .get(&(terms.date_policy.id, terms.date_policy.version))
            .ok_or_else(incomplete)?;
        if policy.id != request.governing_policy.id
            || policy.version != request.governing_policy.version
            || dates.id != terms.date_policy.id
            || dates.version != terms.date_policy.version
            || !known_time(&terms.bookkeeping_date)
        {
            return Err(incomplete());
        }
        let interpretation = &request.interpretation;
        if interpretation.authority != AuthorityKind::Owner
            || interpretation.kind != DecisionKind::Interpretation
            || interpretation.reason.trim().is_empty()
            || interpretation.reason.len() > 4096
            || request.event.interpretation != SourceValue::Known(interpretation.clone())
            || !request.event.relationships.is_empty()
            || terms.unresolved_account.is_some()
            || terms.opening.is_some()
        {
            return Err(invalid());
        }
        for id in &interpretation.evidence {
            if !matches!(
                state.records.get(id),
                Some(
                    AuthoritativeRecordV1::ObservationVersion(_)
                        | AuthoritativeRecordV1::ImportReceipt(_)
                        | AuthoritativeRecordV1::AddressedParseProduct(_)
                )
            ) {
                return Err(incomplete());
            }
        }

        let cash = side(&request.cash)?;
        let quantity = side(&request.quantity)?;
        let known: Vec<&AdoptedLegV1> = cash.iter().chain(&quantity).copied().collect();
        if known.is_empty()
            || request.event.legs != known.iter().map(|l| l.leg.id).collect::<Vec<_>>()
        {
            return Err(invalid());
        }
        let mut facts = BTreeSet::new();
        if let Some(cash) = cash {
            let fact = known_leg(&lookup, request, cash, UnitKind::Currency)?;
            if cash.leg.movement.unit_id != carrying.unit_id || !facts.insert(fact) {
                return Err(invalid());
            }
        }
        if let Some(quantity) = quantity {
            let fact = known_leg(&lookup, request, quantity, UnitKind::Security)?;
            if quantity.leg.account_id != position.quantity_account
                || quantity.leg.movement.unit_id != security.unit_id
                || !facts.insert(fact)
            {
                return Err(invalid());
            }
        }
        if request.fee.unit_id != carrying.unit_id {
            return Err(invalid());
        }
        exact(&lookup, &request.fee)?;

        // Every identity is new, and no other recognition holds the lineage
        // or a posting ID.
        let mut ids = BTreeSet::new();
        let fresh = [request.id, request.event.id, terms.lineage]
            .into_iter()
            .chain([RecordId(terms.recognition_id.0)])
            .chain(known.iter().map(|l| RecordId(l.leg.id.0)))
            .chain(terms.postings.iter().map(|p| p.id));
        for id in fresh {
            if id.0.is_nil() || !ids.insert(id) || state.records.contains_key(&id) {
                return Err(invalid());
            }
        }
        let taken =
            lookup.recognitions().iter().any(|r| {
                r.lineage == terms.lineage || r.postings.iter().any(|p| ids.contains(&p.id))
            }) || index.recognitions.contains_key(&terms.recognition_id)
                || index
                    .recognitions
                    .values()
                    .flatten()
                    .any(|entry| entry.postings.iter().any(|p| ids.contains(&p.original.id)));
        if taken {
            return Err(invalid());
        }

        // Each posting is in its account's unit and fills one slot once; the
        // slots are exactly those the known sides require.
        let mut sums = BTreeMap::<UnitId, Amount>::new();
        let mut actual = Expected::new();
        for posting in &terms.postings {
            let (component, catalog) = account(state, posting.account_id)?;
            let mapping = index
                .accounts
                .get(&posting.account_id)
                .ok_or_else(incomplete)?;
            if posting.amount.coefficient == 0
                || posting.amount.unit_id != component.unit_id
                || mapping.unit_id != component.unit_id
                || catalog.catalog.lifecycle != Lifecycle::Open
                || !known_time(&posting.effective)
                || !known_time(&posting.posting)
            {
                return Err(invalid());
            }
            exact(&lookup, &posting.amount)?;
            let sum = sums.entry(posting.amount.unit_id).or_insert(Amount::new(
                0,
                0,
                posting.amount.unit_id,
            )?);
            *sum = sum.checked_add(&posting.amount)?;
            let slot = slot(state, position, cash, posting.account_id)?;
            let filled = (
                posting.amount.normalized()?,
                &posting.effective,
                &posting.posting,
            );
            if actual.insert(slot, filled).is_some() {
                return Err(invalid());
            }
        }
        if sums.values().any(|sum| sum.coefficient != 0) {
            return Err(EngineError {
                class: ErrorClass::Unbalanced,
                ..invalid()
            });
        }
        let mut required = expected(request, cash, quantity)?;
        for value in required.values_mut() {
            value.0 = value.0.normalized()?;
        }
        if actual != required {
            return Err(invalid());
        }
        // Activity at or before a supported opening is already in it.
        crate::segments::movement_boundary(
            state,
            terms
                .postings
                .iter()
                .map(|p| (p.account_id, &p.effective))
                .chain(known.iter().map(|l| (l.leg.account_id, &l.leg.effective))),
        )?;

        let days: Vec<Date> = terms
            .postings
            .iter()
            .filter_map(|p| day(&p.effective).ok())
            .collect();
        let recognition = RecognitionV1 {
            id: terms.recognition_id,
            lineage: terms.lineage,
            postings: terms.postings.clone(),
            kind: RecognitionKind::Activity,
            evidence_allocations: known
                .iter()
                .flat_map(|l| l.leg.allocations.clone())
                .collect(),
            leg_ids: request.event.legs.clone(),
            scope: DateScope {
                start: days.iter().min().copied(),
                end: days.iter().max().copied(),
                basis: DateBasis::Effective,
            },
            bookkeeping_date: terms.bookkeeping_date.clone(),
            date_policy: terms.date_policy.clone(),
        };
        let bound = |leg: Option<&AdoptedLegV1>| match leg {
            Some(item) => SourceValue::Known(AdoptedFacetV1 {
                leg_id: item.leg.id,
                source: item.sources[0].clone(),
            }),
            None => SourceValue::Missing,
        };
        let trade = TradeV1 {
            id: request.id,
            schema_version: 1,
            position: request.position,
            side: request.side,
            event_id: request.event.id,
            cash: bound(cash),
            quantity: bound(quantity),
            fee: request.fee,
            recognition: terms.recognition_id,
            interpretation: interpretation.clone(),
            governing_policy: request.governing_policy.clone(),
        };
        let mut records = vec![AuthoritativeRecordV1::FinancialEvent(request.event.clone())];
        records.extend(
            known
                .iter()
                .map(|l| AuthoritativeRecordV1::FinancialLeg(l.leg.clone())),
        );
        records.push(AuthoritativeRecordV1::Trade(trade));
        records.push(AuthoritativeRecordV1::Recognition(recognition.clone()));
        Ok(DecisionPlan {
            records,
            invalidations: vec![],
            accounting: AccountingIntent::Recognition(recognition),
        })
    }

    /// What `position` holds through `through`: the quantity and carrying
    /// amount its recognitions not reversed post on or before that day, the
    /// trades by then whose quantity or cash side is missing, and what
    /// snapshots as of that day or earlier report for it.
    pub fn holdings(
        state: &DomainState,
        position: RecordId,
        through: Date,
    ) -> Result<PositionHoldingsV1, EngineError> {
        let index = state.accounting.as_ref().ok_or_else(incomplete)?;
        if index.prefix.through != state.through {
            return Err(incomplete());
        }
        let Some(AuthoritativeRecordV1::Position(p)) = state.records.get(&position) else {
            return Err(incomplete());
        };
        let security = Self::current(state, p.security)?;
        let (carrying, _) = account(state, p.carrying_account)?;
        let mut quantity = Amount::new(0, 0, security.unit_id)?;
        let mut carried = Amount::new(0, 0, carrying.unit_id)?;
        for (id, entries) in &index.recognitions {
            if index.reversed.contains(id) {
                continue;
            }
            for posting in entries.iter().flat_map(|e| &e.postings) {
                let posting = &posting.original;
                let total = if posting.account_id == p.quantity_account {
                    &mut quantity
                } else if posting.account_id == p.carrying_account {
                    &mut carried
                } else {
                    continue;
                };
                if day(&posting.effective)? <= through {
                    *total = total.checked_add(&posting.amount)?;
                }
            }
        }
        let mut missing_quantity = Vec::new();
        let mut missing_cash = Vec::new();
        for record in state.records.values() {
            let AuthoritativeRecordV1::Trade(trade) = record else {
                continue;
            };
            if trade.position != position || index.reversed.contains(&trade.recognition) {
                continue;
            }
            let Some(AuthoritativeRecordV1::Recognition(r)) =
                state.records.get(&RecordId(trade.recognition.0))
            else {
                return Err(incomplete());
            };
            let mut by_then = false;
            for posting in &r.postings {
                by_then |= day(&posting.effective)? <= through;
            }
            if !by_then {
                continue;
            }
            if trade.quantity == SourceValue::Missing {
                missing_quantity.push(trade.id);
            }
            if trade.cash == SourceValue::Missing {
                missing_cash.push(trade.id);
            }
        }
        let (snapshot, absence, unmapped) = super::holdings::latest(state, p, through)?;
        Ok(PositionHoldingsV1 {
            position,
            through,
            quantity: quantity.normalized()?,
            carrying: carried.normalized()?,
            missing_quantity,
            missing_cash,
            snapshot,
            absence,
            unmapped,
        })
    }
}
