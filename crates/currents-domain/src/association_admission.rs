//! Complete association admission, separate from source description/arithmetic.
use crate::{associations::IdentityEngine, catalog::CatalogEngine};
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};
fn invalid() -> EngineError {
    EngineError::invalid()
}
fn origin(
    state: &DomainState,
    account: AccountId,
    unit: UnitId,
    effective: &TemporalValue,
) -> Result<AccountOriginV1, EngineError> {
    let mut found = state.records.values().filter_map(|r| match r {
        AuthoritativeRecordV1::AccountComponent(c) if c.account_id == account => Some(c),
        _ => None,
    });
    let component = found.next().ok_or_else(invalid)?;
    if found.next().is_some() || component.unit_id != unit {
        return Err(invalid());
    }
    let catalog = CatalogEngine::current(state, component.id)?;
    let index = state.accounting.as_ref().ok_or_else(invalid)?;
    let mapping = index.accounts.get(&account).ok_or_else(invalid)?;
    if catalog.catalog.lifecycle != Lifecycle::Open
        || mapping.account_id != account
        || mapping.unit_id != unit
        || mapping.account_class != component.account_class
    {
        return Err(invalid());
    }
    if let AccountOriginV1::Internal(control) = &catalog.origin {
        match (&control.created_at, effective) {
            (TemporalValue::Date { civil: start, .. }, TemporalValue::Date { civil: at, .. })
                if at >= start => {}
            _ => return Err(invalid()),
        }
    }
    Ok(catalog.origin)
}
fn role_is(origin: &AccountOriginV1, roles: &[ControlRoleV1]) -> bool {
    matches!(origin,AccountOriginV1::Internal(control) if roles.contains(&control.role))
}
fn add(map: &mut BTreeMap<UnitId, Amount>, amount: Amount) -> Result<(), EngineError> {
    let current = map
        .entry(amount.unit_id)
        .or_insert(Amount::new(0, 0, amount.unit_id)?);
    *current = current.checked_add(&amount)?;
    Ok(())
}
pub(crate) fn initial(
    state: &DomainState,
    request: &CompleteAssociationRequestV1,
) -> Result<DecisionPlan, EngineError> {
    let revision = &request.revision;
    let mut families = BTreeMap::<RecordId, Vec<AssociationRevisionV1>>::new();
    for record in state.records.values() {
        if let AuthoritativeRecordV1::AssociationRevision(existing) = record {
            families
                .entry(existing.root)
                .or_default()
                .push(existing.clone());
        }
    }
    let mut heads = BTreeMap::new();
    for (root, revisions) in &families {
        let chain = IdentityEngine::revision_chain(revisions, *root)?;
        heads.insert(*root, chain.last().ok_or_else(invalid)?.clone());
    }
    let previous = heads.get(&revision.root);
    match (previous, revision.prior_revision) {
        (None, None) if revision.retired_recognitions.is_empty() => {}
        (Some(old), Some(prior)) if old.id == prior => {
            let retired: BTreeSet<_> = revision.retired_recognitions.iter().copied().collect();
            if retired.len() != revision.retired_recognitions.len()
                || retired != old.recognitions.iter().copied().collect()
                || retired.is_empty()
            {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    let mut chain = families.get(&revision.root).cloned().unwrap_or_default();
    chain.push(revision.clone());
    IdentityEngine::revision_chain(&chain, revision.root)?;
    if !revision.allocations.is_empty()
        || !revision.superseded_adoptions.is_empty()
        || revision.survivor.is_some()
    {
        return Err(EngineError::version());
    }
    if !matches!(
        revision.kind,
        RelationshipKind::Transfer | RelationshipKind::Payroll
    ) || request.legs.len() != revision.legs.len()
        || request.recognitions.len() != request.legs.len()
        || request.events.is_empty()
        || request.events.len() > 1024
        || revision.captured_allocations.is_empty()
        || revision.captured_allocations.len() > 16384
    {
        return Err(invalid());
    }
    let index = state.accounting.as_ref().ok_or_else(invalid)?;
    if index.prefix.through != state.through || !index.calendar_declared {
        return Err(invalid());
    }
    for id in &revision.retired_recognitions {
        let Some(AuthoritativeRecordV1::Recognition(original)) = state.records.get(&RecordId(id.0))
        else {
            return Err(invalid());
        };
        let entries = index.recognitions.get(id).ok_or_else(invalid)?;
        if original.id != *id
            || entries.is_empty()
            || index.reversed.contains(id)
            || entries
                .iter()
                .any(|e| e.recognition_id != *id || e.reverses.is_some())
        {
            return Err(invalid());
        }
    }

    let policy = state
        .dataset_policies
        .get(&(
            revision.governing_policy.id,
            revision.governing_policy.version,
        ))
        .ok_or_else(invalid)?;
    if policy.id != revision.governing_policy.id
        || policy.version != revision.governing_policy.version
    {
        return Err(invalid());
    }
    for evidence in &revision.interpretation.evidence {
        if !matches!(
            state.records.get(evidence),
            Some(
                AuthoritativeRecordV1::RelationshipCapture(_)
                    | AuthoritativeRecordV1::ImportReceipt(_)
            )
        ) {
            return Err(invalid());
        }
    }
    let mut ids = BTreeSet::new();
    let mut insert = |id: RecordId| -> Result<(), EngineError> {
        if id.0.is_nil() || !ids.insert(id) || state.records.contains_key(&id) {
            Err(invalid())
        } else {
            Ok(())
        }
    };
    insert(revision.id)?;
    let mut event_legs = BTreeSet::new();
    for event in &request.events {
        insert(event.id)?;
        if !event.relationships.is_empty()
            || event.interpretation != SourceValue::Known(revision.interpretation.clone())
            || event.legs.is_empty()
        {
            return Err(invalid());
        }
        for leg in &event.legs {
            if !event_legs.insert(*leg)
                || !request
                    .legs
                    .iter()
                    .any(|l| l.id == *leg && l.event_id == event.id)
            {
                return Err(invalid());
            }
        }
    }
    if event_legs != revision.legs.iter().copied().collect() {
        return Err(invalid());
    }
    let mut roles = BTreeMap::new();
    let mut total = BTreeMap::new();
    let mut seen_edges = BTreeSet::new();
    let mut capacity = BTreeMap::<String, (Amount, Amount)>::new();
    for leg in &request.legs {
        insert(RecordId(leg.id.0))?;
        if !revision.legs.contains(&leg.id)
            || !leg.allocations.is_empty()
            || !leg.relationships.is_empty()
            || leg.interpretation != SourceValue::Known(revision.interpretation.clone())
            || leg.unresolved_residual != SourceValue::Absent
            || leg.movement.coefficient == 0
            || !matches!(leg.posting, TemporalValue::Date { .. })
        {
            return Err(invalid());
        }
        let mut additive = Amount::new(0, 0, leg.movement.unit_id)?;
        let mut has_additive = false;
        let mut corroborated = false;
        let mut role = None;
        for edge in revision
            .captured_allocations
            .iter()
            .filter(|e| e.leg == leg.id)
        {
            let fact = IdentityEngine::captured_facet(state, &edge.source, leg.movement.unit_id)?;
            if fact.status != RelationshipSourceStatusV1::Posted
                || fact.effective != leg.effective
                || role.is_some_and(|old| old != fact.role)
                || !seen_edges.insert((fact.fact_key.clone(), leg.id))
            {
                return Err(invalid());
            }
            role = Some(fact.role);
            let oriented = if fact.role == RelationshipFacetRoleV1::GrossPay {
                fact.amount.checked_neg()?
            } else {
                fact.amount
            };
            let amount = edge.allocated.normalized()?;
            if amount.unit_id != oriented.unit_id
                || amount.coefficient == 0
                || amount.coefficient.signum() != oriented.coefficient.signum()
                || amount.coefficient.signum() != leg.movement.coefficient.signum()
            {
                return Err(invalid());
            }
            let entry = capacity
                .entry(fact.fact_key)
                .or_insert((oriented, Amount::new(0, 0, oriented.unit_id)?));
            if entry.0 != oriented {
                return Err(invalid());
            }
            entry.1 = entry.1.checked_add(&amount)?;
            match edge.mode {
                FacetAllocationModeV1::Additive => {
                    has_additive = true;
                    additive = additive.checked_add(&amount)?;
                }
                FacetAllocationModeV1::Corroborating => {
                    if amount != leg.movement.normalized()? {
                        return Err(invalid());
                    }
                    corroborated = true;
                }
            }
        }
        if (has_additive && additive.normalized() != leg.movement.normalized())
            || (!has_additive && !corroborated)
        {
            return Err(invalid());
        }
        let role = role.ok_or_else(invalid)?;
        let account = origin(state, leg.account_id, leg.movement.unit_id, &leg.effective)?;
        let allowed = match (revision.kind, role) {
            (RelationshipKind::Transfer, RelationshipFacetRoleV1::Movement) => {
                account == AccountOriginV1::External
            }
            (RelationshipKind::Transfer, RelationshipFacetRoleV1::Fee) => {
                account == AccountOriginV1::External && leg.movement.coefficient < 0
            }
            (RelationshipKind::Payroll, RelationshipFacetRoleV1::GrossPay) => {
                role_is(&account, &[ControlRoleV1::IncomeFlow]) && leg.movement.coefficient < 0
            }
            (RelationshipKind::Payroll, RelationshipFacetRoleV1::Withholding) => {
                role_is(
                    &account,
                    &[ControlRoleV1::ExpenseFlow, ControlRoleV1::TaxPrepayment],
                ) && leg.movement.coefficient > 0
            }
            (
                RelationshipKind::Payroll,
                RelationshipFacetRoleV1::Retirement
                | RelationshipFacetRoleV1::HealthSavings
                | RelationshipFacetRoleV1::NetPay,
            ) => account == AccountOriginV1::External && leg.movement.coefficient > 0,
            _ => false,
        };
        if !allowed {
            return Err(invalid());
        }
        if role != RelationshipFacetRoleV1::Fee {
            add(&mut total, leg.movement)?;
        }
        roles.insert(leg.id, role);
    }
    if seen_edges.len() != revision.captured_allocations.len() {
        return Err(invalid());
    }
    for (limit, used) in capacity.values() {
        let remaining = limit.checked_add(&used.checked_neg()?)?;
        if remaining.coefficient != 0
            && remaining.coefficient.signum() != limit.coefficient.signum()
        {
            return Err(invalid());
        }
    }
    let residuals: Vec<_> = total
        .values()
        .map(|a| a.normalized())
        .collect::<Result<_, _>>()?;
    if residuals != revision.residuals {
        return Err(invalid());
    }
    if revision.kind == RelationshipKind::Payroll
        && (residuals.iter().any(|a| a.coefficient != 0)
            || [
                RelationshipFacetRoleV1::GrossPay,
                RelationshipFacetRoleV1::Withholding,
                RelationshipFacetRoleV1::Retirement,
                RelationshipFacetRoleV1::HealthSavings,
                RelationshipFacetRoleV1::NetPay,
            ]
            .iter()
            .any(|role| !roles.values().any(|r| r == role)))
    {
        return Err(invalid());
    }
    let mut recognized = BTreeSet::new();
    let mut recognition_ids = BTreeSet::new();
    for recognition in &request.recognitions {
        insert(RecordId(recognition.id.0))?;
        insert(recognition.lineage)?;
        if !recognition_ids.insert(recognition.id)
            || !revision.recognitions.contains(&recognition.id)
            || recognition.leg_ids.len() != 1
            || !recognized.insert(recognition.leg_ids[0])
            || recognition.kind != RecognitionKind::Activity
            || !recognition.evidence_allocations.is_empty()
            || recognition.postings.len() != 2
            || recognition.scope.basis != DateBasis::Effective
            || recognition.scope.start.is_some()
            || recognition.scope.end.is_some()
        {
            return Err(invalid());
        }
        let leg = request
            .legs
            .iter()
            .find(|l| l.id == recognition.leg_ids[0])
            .ok_or_else(invalid)?;
        let date_policy = index
            .date_policies
            .get(&(recognition.date_policy.id, recognition.date_policy.version))
            .ok_or_else(invalid)?;
        if date_policy.conversion != DateConversionV1::CivilOnly
            || !matches!(recognition.bookkeeping_date, TemporalValue::Date { .. })
            || (date_policy.bookkeeping == BookkeepingFallbackV1::RequirePosting
                && recognition.bookkeeping_date != leg.posting)
        {
            return Err(invalid());
        }
        let anchor = &recognition.postings[0];
        let counter = &recognition.postings[1];
        if anchor.account_id != leg.account_id
            || anchor.amount.normalized()? != leg.movement.normalized()?
            || counter.account_id == leg.account_id
            || counter.amount.normalized()? != leg.movement.checked_neg()?.normalized()?
        {
            return Err(invalid());
        }
        for posting in &recognition.postings {
            insert(posting.id)?;
            if posting.effective != leg.effective || posting.posting != leg.posting {
                return Err(invalid());
            }
            origin(
                state,
                posting.account_id,
                posting.amount.unit_id,
                &posting.effective,
            )?;
            crate::catalog::unit(state, posting.amount.unit_id)?.validate()?;
        }
        let counter_origin = origin(
            state,
            counter.account_id,
            counter.amount.unit_id,
            &counter.effective,
        )?;
        let expected = if roles[&leg.id] == RelationshipFacetRoleV1::Fee {
            ControlRoleV1::ExpenseFlow
        } else {
            ControlRoleV1::Clearing
        };
        if !role_is(&counter_origin, &[expected]) {
            return Err(invalid());
        }
    }
    if recognized != revision.legs.iter().copied().collect()
        || recognition_ids != revision.recognitions.iter().copied().collect()
        || revision.recognitions.len() != recognition_ids.len()
    {
        return Err(invalid());
    }
    // Only current family heads own capacity. Immutable prior snapshots stay
    // available for history/replay, but never consume capacity a second time.
    for existing in heads.values().filter(|v| v.root != revision.root) {
        for edge in &existing.captured_allocations {
            let fact = IdentityEngine::retained_facet(state, &edge.source, edge.allocated.unit_id)?;
            if capacity.contains_key(&fact.fact_key) {
                return Err(invalid());
            }
        }
    }
    let accounting = if revision.retired_recognitions.is_empty() {
        AccountingIntent::Recognitions(request.recognitions.clone())
    } else {
        // One bounded closure reverses every previous active entry and installs
        // all replacements. Grouping in the first correction is deterministic,
        // not a chain of commands or an intermediate partially active state.
        AccountingIntent::Corrections(
            revision
                .retired_recognitions
                .iter()
                .enumerate()
                .map(|(position, id)| CorrectionV1 {
                    active_recognition: *id,
                    replacement: if position == 0 {
                        request.recognitions.clone()
                    } else {
                        vec![]
                    },
                    remapping: vec![],
                    reason: revision.interpretation.reason.clone(),
                })
                .collect(),
        )
    };
    let mut records: Vec<_> = request
        .events
        .iter()
        .cloned()
        .map(AuthoritativeRecordV1::FinancialEvent)
        .collect();
    records.extend(
        request
            .legs
            .iter()
            .cloned()
            .map(AuthoritativeRecordV1::FinancialLeg),
    );
    records.extend(
        request
            .recognitions
            .iter()
            .cloned()
            .map(AuthoritativeRecordV1::Recognition),
    );
    records.push(AuthoritativeRecordV1::AssociationRevision(revision.clone()));
    Ok(DecisionPlan {
        records,
        invalidations: revision.legs.iter().map(|l| RecordId(l.0)).collect(),
        accounting,
    })
}
