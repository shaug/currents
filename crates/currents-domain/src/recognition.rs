//! Explicit evidence adoption and recognition, with no ambient clock or identity generation.
use crate::catalog::{CatalogEngine, incomplete, known_time, unit, version};
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

pub struct RecognitionEngine;
fn duplicate() -> EngineError {
    EngineError {
        class: ErrorClass::DuplicateRecognition,
        ..EngineError::invalid()
    }
}
fn equal(a: Amount, b: Amount) -> Result<bool, EngineError> {
    Ok(a.normalized()? == b.normalized()?)
}
/// An explicit source revision preserves fact identity. Similar provider strings
/// across independent observations are deliberately not an equivalence relation.
pub(crate) fn fact(
    state: &DomainState,
    source: &SourceFacetBindingV1,
) -> Result<(RecordId, RecordId), EngineError> {
    let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
        state.records.get(&source.parse_product)
    else {
        return Err(incomplete());
    };
    product.validate()?;
    if product.id != source.parse_product || product.product.source_version_id != source.observation
    {
        return Err(EngineError::invalid());
    }
    let mut facets = product
        .product
        .facets
        .iter()
        .filter(|f| f.lineage_key == source.facet);
    let facet = facets.next().ok_or_else(incomplete)?;
    if facets.next().is_some()
        || !matches!(
            facet.kind,
            ObservationKind::Movement | ObservationKind::Balance
        )
        || source.facet.0.is_nil()
    {
        return Err(EngineError::invalid());
    }
    let SourceValue::Known(value) = facet.amount else {
        return Err(incomplete());
    };
    if !equal(value, source.amount)? {
        return Err(EngineError::invalid());
    }
    let (root, kind) = source_root(state, source.observation)?;
    if kind != facet.kind {
        return Err(EngineError::invalid());
    }
    // Delivered C07 admits one stable facet per observation. A reparse or
    // explicit source revision cannot invent a fresh ownership key for the
    // same fact; ambiguous decomposition belongs to a future owner contract.
    let mut current = source.observation;
    loop {
        let mut found = false;
        for record in state.records.values() {
            let parsed = match record {
                AuthoritativeRecordV1::AddressedParseProduct(p)
                    if p.product.source_version_id == current =>
                {
                    p.validate()?;
                    Some(&p.product)
                }
                AuthoritativeRecordV1::ParseProduct(p) if p.source_version_id == current => Some(p),
                _ => None,
            };
            if let Some(parsed) = parsed {
                found = true;
                if parsed.facets.len() != 1 {
                    return Err(EngineError::version());
                }
                if parsed
                    .facets
                    .first()
                    .is_none_or(|f| f.lineage_key != source.facet || f.kind != kind)
                {
                    return Err(EngineError::invalid());
                }
            }
        }
        if !found {
            return Err(incomplete());
        }
        let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
            state.records.get(&current)
        else {
            return Err(incomplete());
        };
        match observation.prior_version {
            Some(prior) => current = prior,
            None => break,
        }
    }
    Ok((root, source.facet))
}
fn source_root(
    state: &DomainState,
    id: RecordId,
) -> Result<(RecordId, ObservationKind), EngineError> {
    let mut current = id;
    let mut seen = BTreeSet::new();
    let mut identity = None;
    let mut kind = None;
    loop {
        if !seen.insert(current) {
            return Err(EngineError::invalid());
        }
        let Some(AuthoritativeRecordV1::ObservationVersion(observation)) =
            state.records.get(&current)
        else {
            return Err(incomplete());
        };
        if observation.id != current
            || observation.tombstone_of.is_some()
            || !matches!(
                observation.semantic_kind,
                ObservationKind::Movement | ObservationKind::Balance
            )
            || kind.is_some_and(|k| k != observation.semantic_kind)
            || identity.as_ref().is_some_and(|s| s != &observation.source)
        {
            return Err(EngineError::invalid());
        }
        identity = Some(observation.source.clone());
        kind = Some(observation.semantic_kind);
        match observation.prior_version {
            Some(prior) => current = prior,
            None => return Ok((current, observation.semantic_kind)),
        }
    }
}

fn validate(
    state: &DomainState,
    request: &ActivityRequestV1,
    admission: bool,
) -> Result<Vec<AdoptedFacetV1>, EngineError> {
    version(request.schema_version)?;
    match request.status {
        ActivityStatusV1::Pending
            if request.financial.is_none() && request.prior_pending.is_none() => {}
        ActivityStatusV1::Posted if request.financial.is_some() => {}
        _ => return Err(EngineError::invalid()),
    }
    let policy = state
        .dataset_policies
        .get(&(
            request.governing_policy.id,
            request.governing_policy.version,
        ))
        .ok_or_else(incomplete)?;
    if policy.id != request.governing_policy.id
        || policy.version != request.governing_policy.version
        || policy.version == 0
    {
        return Err(incomplete());
    }
    if request.legs.is_empty()
        || request.legs.len() > 1024
        || !request.event.relationships.is_empty()
        || request.interpretation.authority != AuthorityKind::Owner
        || request.interpretation.kind != DecisionKind::Interpretation
        || request.interpretation.reason.is_empty()
        || request.interpretation.reason.len() > 4096
        || request.event.interpretation != SourceValue::Known(request.interpretation.clone())
    {
        return Err(EngineError::invalid());
    }
    for id in &request.interpretation.evidence {
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
    let mut ids = BTreeSet::new();
    for id in [request.adoption_id, request.event.id] {
        if id.0.is_nil() || !ids.insert(id) {
            return Err(EngineError::invalid());
        }
    }
    if request.event.legs != request.legs.iter().map(|l| l.leg.id).collect::<Vec<_>>() {
        return Err(EngineError::invalid());
    }
    let mut facts = BTreeSet::new();
    let mut adopted = Vec::new();
    for item in &request.legs {
        let leg = &item.leg;
        if leg.id.0.is_nil()
            || !ids.insert(RecordId(leg.id.0))
            || leg.event_id != request.event.id
            || !leg.relationships.is_empty()
            || !matches!(leg.unresolved_residual, SourceValue::Absent)
            || leg.interpretation != SourceValue::Known(request.interpretation.clone())
            || item.sources.is_empty()
            || item.sources.len() > 256
        {
            return Err(EngineError::invalid());
        }
        let mut components = state.records.values().filter_map(|r| match r {
            AuthoritativeRecordV1::AccountComponent(c) if c.account_id == leg.account_id => Some(c),
            _ => None,
        });
        let component = components.next().ok_or_else(incomplete)?;
        if components.next().is_some() || component.unit_id != leg.movement.unit_id {
            return Err(EngineError::invalid());
        }
        let current = CatalogEngine::current(state, component.id)?;
        if (admission && current.catalog.lifecycle != Lifecycle::Open)
            || current.origin != AccountOriginV1::External
        {
            return Err(EngineError::invalid());
        }
        let definition = unit(state, leg.movement.unit_id)?;
        leg.movement.checked_rescale(definition.exponent)?;
        if leg.movement.coefficient == 0 {
            return Err(EngineError::invalid());
        }
        let expected = item
            .sources
            .iter()
            .map(|s| EvidenceAllocationV1 {
                observation: s.observation,
                facet: s.facet,
                amount: s.amount,
            })
            .collect::<Vec<_>>();
        if leg.allocations != expected {
            return Err(EngineError::invalid());
        }
        for source in &item.sources {
            let expected_kind = if request
                .financial
                .as_ref()
                .is_some_and(|f| f.opening.is_some())
            {
                ObservationKind::Balance
            } else {
                ObservationKind::Movement
            };
            let Some(AuthoritativeRecordV1::AddressedParseProduct(product)) =
                state.records.get(&source.parse_product)
            else {
                return Err(incomplete());
            };
            if !product
                .product
                .facets
                .iter()
                .any(|f| f.lineage_key == source.facet && f.kind == expected_kind)
            {
                return Err(EngineError::invalid());
            }
            if !equal(source.amount, leg.movement)? || !facts.insert(fact(state, source)?) {
                return Err(duplicate());
            }
            adopted.push(AdoptedFacetV1 {
                leg_id: leg.id,
                source: source.clone(),
            });
        }
    }
    Ok(adopted)
}
fn component(state: &DomainState, account: AccountId) -> Result<&AccountComponentV1, EngineError> {
    let mut matches = state.records.values().filter_map(|r| match r {
        AuthoritativeRecordV1::AccountComponent(c) if c.account_id == account => Some(c),
        _ => None,
    });
    let c = matches.next().ok_or_else(incomplete)?;
    if matches.next().is_some() {
        return Err(EngineError::invalid());
    }
    Ok(c)
}
fn monetary(
    state: &DomainState,
    request: &ActivityRequestV1,
    admission: bool,
) -> Result<Option<RecognitionV1>, EngineError> {
    let Some(terms) = &request.financial else {
        return Ok(None);
    };
    crate::segments::initial(state, request, admission)?;
    if admission {
        crate::segments::activity_boundary(state, request)?;
    }
    let index = state.accounting.as_ref().ok_or_else(incomplete)?;
    if index.prefix.through != state.through || !index.calendar_declared {
        return Err(incomplete());
    }
    let policy = index
        .date_policies
        .get(&(terms.date_policy.id, terms.date_policy.version))
        .ok_or_else(incomplete)?;
    if policy.id != terms.date_policy.id
        || policy.version != terms.date_policy.version
        || policy.version == 0
        || !known_time(&terms.bookkeeping_date)
    {
        return Err(incomplete());
    }
    if terms.recognition_id.0.is_nil()
        || terms.lineage.0.is_nil()
        || terms.postings.len() < 2
        || terms.postings.len() > 4096
    {
        return Err(EngineError::invalid());
    }
    let mut identities = BTreeSet::from([request.adoption_id, request.event.id]);
    identities.extend(request.event.legs.iter().map(|l| RecordId(l.0)));
    for id in [RecordId(terms.recognition_id.0), terms.lineage] {
        if !identities.insert(id) {
            return Err(EngineError::invalid());
        }
    }
    if let Some(opening) = &terms.opening
        && !identities.insert(opening.segment_id)
    {
        return Err(EngineError::invalid());
    }
    let mut sums = BTreeMap::<UnitId, Amount>::new();
    let mut external = BTreeMap::<AccountId, Amount>::new();
    let mut unresolved = false;
    for posting in &terms.postings {
        if posting.id.0.is_nil()
            || !identities.insert(posting.id)
            || posting.amount.coefficient == 0
            || !known_time(&posting.effective)
            || !known_time(&posting.posting)
        {
            return Err(EngineError::invalid());
        }
        let c = component(state, posting.account_id)?;
        let catalog = CatalogEngine::current(state, c.id)?;
        if admission && catalog.catalog.lifecycle != Lifecycle::Open {
            return Err(EngineError::invalid());
        }
        let mapping = index
            .accounts
            .get(&posting.account_id)
            .ok_or_else(incomplete)?;
        let u = unit(state, posting.amount.unit_id)?;
        if c.unit_id != posting.amount.unit_id
            || mapping.account_id != c.account_id
            || mapping.unit_id != c.unit_id
            || mapping.account_class != c.account_class
            || index.units.get(&u.id) != Some(&u)
        {
            return Err(EngineError::invalid());
        }
        posting.amount.checked_rescale(u.exponent)?;
        let sum = sums.entry(u.id).or_insert(Amount::new(0, 0, u.id)?);
        *sum = sum.checked_add(&posting.amount)?;
        match catalog.origin {
            AccountOriginV1::External => {
                let sum = external
                    .entry(c.account_id)
                    .or_insert(Amount::new(0, 0, c.unit_id)?);
                *sum = sum.checked_add(&posting.amount)?;
            }
            AccountOriginV1::Internal(control) => match control.role {
                ControlRoleV1::UnresolvedEquity => {
                    if terms.unresolved_account != Some(c.account_id) {
                        return Err(incomplete());
                    }
                    unresolved = true;
                }
                ControlRoleV1::IncomeFlow | ControlRoleV1::ExpenseFlow
                    if terms.opening.is_none() => {}
                ControlRoleV1::IncomeFlow | ControlRoleV1::ExpenseFlow => {
                    return Err(EngineError::invalid());
                }
                ControlRoleV1::OpeningEquity if terms.opening.is_some() => {}
                ControlRoleV1::OpeningEquity => return Err(EngineError::invalid()),
            },
        }
    }
    if sums.values().any(|a| a.coefficient != 0) {
        return Err(EngineError {
            class: ErrorClass::Unbalanced,
            ..EngineError::invalid()
        });
    }
    if terms.unresolved_account.is_some() != unresolved {
        return Err(EngineError::invalid());
    }
    let mut legs = BTreeMap::<AccountId, Amount>::new();
    for item in &request.legs {
        let sum = legs.entry(item.leg.account_id).or_insert(Amount::new(
            0,
            0,
            item.leg.movement.unit_id,
        )?);
        *sum = sum.checked_add(&item.leg.movement)?;
    }
    if legs.len() != external.len() {
        return Err(EngineError::invalid());
    }
    for (account, value) in legs {
        if !equal(
            value,
            *external.get(&account).ok_or_else(EngineError::invalid)?,
        )? {
            return Err(EngineError::invalid());
        }
    }
    let dates = terms
        .postings
        .iter()
        .filter_map(|p| match p.effective {
            TemporalValue::Date { civil, .. } => Some(civil),
            _ => None,
        })
        .collect::<Vec<_>>();
    let scope = DateScope {
        start: dates.iter().min().copied(),
        end: dates.iter().max().copied(),
        basis: DateBasis::Effective,
    };
    Ok(Some(RecognitionV1 {
        id: terms.recognition_id,
        lineage: terms.lineage,
        postings: terms.postings.clone(),
        kind: if terms.opening.is_some() {
            RecognitionKind::Opening
        } else {
            RecognitionKind::Activity
        },
        evidence_allocations: request
            .legs
            .iter()
            .flat_map(|l| l.leg.allocations.clone())
            .collect(),
        leg_ids: request.event.legs.clone(),
        scope,
        bookkeeping_date: terms.bookkeeping_date.clone(),
        date_policy: terms.date_policy.clone(),
    }))
}
pub(crate) fn restore(
    state: &DomainState,
    adoption: &ActivityAdoptionV1,
) -> Result<ActivityRequestV1, EngineError> {
    version(adoption.schema_version)?;
    let Some(AuthoritativeRecordV1::FinancialEvent(event)) = state.records.get(&adoption.event_id)
    else {
        return Err(incomplete());
    };
    if event.id != adoption.event_id || event.legs != adoption.legs {
        return Err(EngineError::invalid());
    }
    let mut legs = Vec::new();
    for id in &adoption.legs {
        let Some(AuthoritativeRecordV1::FinancialLeg(leg)) = state.records.get(&RecordId(id.0))
        else {
            return Err(incomplete());
        };
        if leg.id != *id {
            return Err(EngineError::invalid());
        }
        legs.push(AdoptedLegV1 {
            leg: leg.clone(),
            sources: adoption
                .facets
                .iter()
                .filter(|f| f.leg_id == *id)
                .map(|f| f.source.clone())
                .collect(),
        });
    }
    let financial = match adoption.recognition {
        None => {
            if adoption.opening.is_some() || adoption.unresolved_account.is_some() {
                return Err(EngineError::invalid());
            }
            None
        }
        Some(id) => {
            let Some(AuthoritativeRecordV1::Recognition(r)) = state.records.get(&RecordId(id.0))
            else {
                return Err(incomplete());
            };
            if r.id != id {
                return Err(EngineError::invalid());
            }
            Some(RecognitionTermsV1 {
                recognition_id: id,
                lineage: r.lineage,
                postings: r.postings.clone(),
                bookkeeping_date: r.bookkeeping_date.clone(),
                date_policy: r.date_policy.clone(),
                unresolved_account: adoption.unresolved_account,
                opening: adoption.opening.clone(),
            })
        }
    };
    Ok(ActivityRequestV1 {
        schema_version: adoption.schema_version,
        adoption_id: adoption.id,
        event: event.clone(),
        legs,
        status: adoption.status,
        prior_pending: adoption.prior_pending,
        interpretation: adoption.interpretation.clone(),
        governing_policy: adoption.governing_policy.clone(),
        financial,
    })
}
fn consumed(
    state: &DomainState,
    adoptions: &[ActivityAdoptionV1],
) -> Result<BTreeSet<RecordId>, EngineError> {
    let mut consumed = BTreeSet::new();
    let mut owners = BTreeMap::<(RecordId, RecordId), Vec<&ActivityAdoptionV1>>::new();
    let mut legs = BTreeSet::new();
    let mut recognitions = BTreeSet::new();
    for a in adoptions {
        for leg in &a.legs {
            if !legs.insert(*leg) {
                return Err(duplicate());
            }
        }
        if let Some(r) = a.recognition
            && !recognitions.insert(r)
        {
            return Err(duplicate());
        }
        if let Some(prior) = a.prior_pending {
            if a.status != ActivityStatusV1::Posted || !consumed.insert(prior) {
                return Err(duplicate());
            }
            let pending = adoptions
                .iter()
                .find(|p| p.id == prior)
                .ok_or_else(incomplete)?;
            if pending.status != ActivityStatusV1::Pending {
                return Err(EngineError::invalid());
            }
            let accounts =
                |adoption: &ActivityAdoptionV1| -> Result<BTreeSet<AccountId>, EngineError> {
                    adoption
                        .legs
                        .iter()
                        .map(|id| match state.records.get(&RecordId(id.0)) {
                            Some(AuthoritativeRecordV1::FinancialLeg(l)) => Ok(l.account_id),
                            _ => Err(incomplete()),
                        })
                        .collect()
                };
            if accounts(pending)? != accounts(a)? {
                return Err(EngineError::invalid());
            }
        }
        for source in a
            .facets
            .iter()
            .map(|f| &f.source)
            .chain(a.opening.iter().flat_map(|o| o.included_activity.iter()))
        {
            owners.entry(fact(state, source)?).or_default().push(a);
        }
    }
    for list in owners.values() {
        if list.len() == 1 {
            continue;
        }
        let pending = list
            .iter()
            .find(|p| p.status == ActivityStatusV1::Pending)
            .ok_or_else(duplicate)?;
        if list.len() != 2
            || !list.iter().any(|a| {
                a.status == ActivityStatusV1::Posted && a.prior_pending == Some(pending.id)
            })
        {
            return Err(duplicate());
        }
    }
    Ok(consumed)
}
impl RecognitionEngine {
    pub fn recognize(
        _state: &DomainState,
        _request: RecognitionRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        Err(EngineError::version())
    }
    pub fn adopt(
        state: &DomainState,
        request: &ActivityRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        let facets = validate(state, request, true)?;
        let recognition = monetary(state, request, true)?;
        let segment = crate::segments::initial(state, request, true)?;
        let ids = [request.adoption_id, request.event.id]
            .into_iter()
            .chain(request.event.legs.iter().map(|id| RecordId(id.0)))
            .chain(recognition.iter().map(|r| RecordId(r.id.0)))
            .chain(segment.iter().map(|s| s.id));
        let mut new_ids = BTreeSet::new();
        for id in ids {
            if id.0.is_nil() || !new_ids.insert(id) || state.records.contains_key(&id) {
                return Err(duplicate());
            }
        }
        let existing = Self::audit(state)?;
        // Older delivered recognitions may precede ActivityAdoption. Their legs
        // and explicit allocations remain owned; the new route cannot bypass them.
        for record in state.records.values() {
            if let AuthoritativeRecordV1::Recognition(old) = record {
                if old.leg_ids.iter().any(|l| request.event.legs.contains(l))
                    || recognition.as_ref().is_some_and(|r| {
                        r.id == old.id
                            || r.lineage == old.lineage
                            || r.postings
                                .iter()
                                .any(|p| old.postings.iter().any(|q| p.id == q.id))
                    })
                {
                    return Err(duplicate());
                }
                for allocation in &old.evidence_allocations {
                    for binding in &facets {
                        if source_root(state, allocation.observation)?.0
                            == fact(state, &binding.source)?.0
                            && allocation.facet == binding.source.facet
                        {
                            return Err(duplicate());
                        }
                    }
                }
            }
        }
        let adoption = ActivityAdoptionV1 {
            id: request.adoption_id,
            schema_version: 1,
            event_id: request.event.id,
            legs: request.event.legs.clone(),
            facets,
            status: request.status,
            prior_pending: request.prior_pending,
            recognition: recognition.as_ref().map(|r| r.id),
            interpretation: request.interpretation.clone(),
            governing_policy: request.governing_policy.clone(),
            opening: request.financial.as_ref().and_then(|f| f.opening.clone()),
            unresolved_account: request
                .financial
                .as_ref()
                .and_then(|f| f.unresolved_account),
        };
        let mut records = vec![AuthoritativeRecordV1::FinancialEvent(request.event.clone())];
        records.extend(
            request
                .legs
                .iter()
                .map(|l| AuthoritativeRecordV1::FinancialLeg(l.leg.clone())),
        );
        records.push(AuthoritativeRecordV1::ActivityAdoption(adoption.clone()));
        if let Some(segment) = segment {
            records.push(AuthoritativeRecordV1::AccountingSegment(segment));
        }
        let accounting = match recognition {
            Some(r) => {
                records.push(AuthoritativeRecordV1::Recognition(r.clone()));
                AccountingIntent::Recognition(r)
            }
            None => AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Explicit pending exposure; no native money".into(),
            }),
        };
        // Check prospective immutable ownership without mutating the caller state.
        let mut prospective = state.clone();
        for record in &records {
            if let AuthoritativeRecordV1::FinancialLeg(l) = record {
                prospective.records.insert(RecordId(l.id.0), record.clone());
            }
        }
        let mut all = existing;
        all.push(adoption);
        consumed(&prospective, &all)?;
        Ok(DecisionPlan {
            records,
            invalidations: vec![request.event.id],
            accounting,
        })
    }
    pub fn audit(state: &DomainState) -> Result<Vec<ActivityAdoptionV1>, EngineError> {
        let mut adoptions = Vec::new();
        let mut lineage_owners = BTreeMap::<RecordId, Vec<RecognitionId>>::new();
        for record in state.records.values() {
            if let AuthoritativeRecordV1::Recognition(r) = record {
                lineage_owners.entry(r.lineage).or_default().push(r.id);
            }
        }
        for (key, record) in &state.records {
            let AuthoritativeRecordV1::ActivityAdoption(a) = record else {
                continue;
            };
            if *key != a.id {
                return Err(EngineError::invalid());
            }
            let request = restore(state, a)?;
            if validate(state, &request, false)? != a.facets {
                return Err(EngineError::invalid());
            }
            if let Some(segment) = crate::segments::initial(state, &request, false)?
                && state.records.get(&segment.id)
                    != Some(&AuthoritativeRecordV1::AccountingSegment(segment))
            {
                return Err(EngineError::invalid());
            }
            if let Some(r) = monetary(state, &request, false)? {
                if lineage_owners.get(&r.lineage).map(Vec::as_slice) != Some([r.id].as_slice()) {
                    return Err(duplicate());
                }
                if state.records.get(&RecordId(r.id.0))
                    != Some(&AuthoritativeRecordV1::Recognition(r))
                {
                    return Err(EngineError::invalid());
                }
            }
            adoptions.push(a.clone());
        }
        consumed(state, &adoptions)?;
        Ok(adoptions)
    }
    /// Clock-free projection; only an exact accepted posted link consumes pending.
    pub fn pending(state: &DomainState) -> Result<Vec<ActivityAdoptionV1>, EngineError> {
        let all = Self::audit(state)?;
        let consumed = consumed(state, &all)?;
        Ok(all
            .into_iter()
            .filter(|a| a.status == ActivityStatusV1::Pending && !consumed.contains(&a.id))
            .collect())
    }
}
