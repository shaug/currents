//! Explicit source-to-economic association admission and replay audit.
//! The standalone quantities helper checks arithmetic only; complete association
//! admission validates authority, exact prior ownership and frozen financial intent.
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};
pub struct IdentityEngine;
struct LegTotal {
    movement: Amount,
    additive: Amount,
    has_additive: bool,
    corroborated: bool,
}
struct FactTotal {
    capacity: Amount,
    allocated: Amount,
}
fn conflict() -> EngineError {
    EngineError {
        class: ErrorClass::AllocationConflict,
        ..EngineError::invalid()
    }
}
fn zero(unit_id: UnitId) -> Amount {
    Amount {
        coefficient: 0,
        scale: 0,
        unit_id,
    }
}
impl IdentityEngine {
    /// Check a proposed edge set against exact retained legacy source facts.
    /// Current global ownership, new relationship profiles and role-to-posting
    /// admission belong to the complete association command, not this helper.
    pub fn quantities(
        state: &DomainState,
        legs: &[FinancialLegV1],
        edges: &[QuantifiedFacetAllocationV1],
    ) -> Result<Vec<SourceAllocationResidualV1>, EngineError> {
        if legs.is_empty() || legs.len() > 1024 || edges.is_empty() || edges.len() > 16384 {
            return Err(conflict());
        }
        let mut totals = BTreeMap::new();
        for leg in legs {
            let movement = leg.movement.normalized()?;
            if leg.id.0.is_nil()
                || movement.coefficient == 0
                || totals
                    .insert(
                        leg.id,
                        LegTotal {
                            movement,
                            additive: zero(movement.unit_id),
                            has_additive: false,
                            corroborated: false,
                        },
                    )
                    .is_some()
            {
                return Err(conflict());
            }
        }
        let mut facts: BTreeMap<(RecordId, RecordId), FactTotal> = BTreeMap::new();
        let mut distinct = BTreeSet::new();
        for edge in edges {
            let fact = crate::recognition::fact(state, &edge.source)?;
            if !matches!(state.records.get(&edge.source.observation), Some(AuthoritativeRecordV1::ObservationVersion(v)) if v.semantic_kind == ObservationKind::Movement)
            {
                return Err(conflict());
            }
            if !distinct.insert((fact, edge.leg)) {
                return Err(conflict());
            }
            let leg = totals.get_mut(&edge.leg).ok_or_else(conflict)?;
            let allocated = edge.allocated.normalized()?;
            let capacity = edge.source.amount.normalized()?;
            if allocated.coefficient == 0
                || capacity.coefficient == 0
                || allocated.unit_id != capacity.unit_id
                || allocated.unit_id != leg.movement.unit_id
                || allocated.coefficient.signum() != capacity.coefficient.signum()
                || allocated.coefficient.signum() != leg.movement.coefficient.signum()
            {
                return Err(conflict());
            }
            let source = facts.entry(fact).or_insert(FactTotal {
                capacity,
                allocated: zero(capacity.unit_id),
            });
            if source.capacity != capacity {
                return Err(conflict());
            }
            source.allocated = source.allocated.checked_add(&allocated)?;
            match edge.mode {
                FacetAllocationModeV1::Additive => {
                    leg.has_additive = true;
                    leg.additive = leg.additive.checked_add(&allocated)?;
                }
                FacetAllocationModeV1::Corroborating => {
                    if allocated != leg.movement {
                        return Err(conflict());
                    }
                    leg.corroborated = true;
                }
            }
        }
        for leg in totals.values() {
            if (leg.has_additive && leg.additive.normalized()? != leg.movement)
                || (!leg.has_additive && !leg.corroborated)
            {
                return Err(conflict());
            }
        }
        facts
            .into_iter()
            .map(|((source_root, facet), fact)| {
                let remaining = fact
                    .capacity
                    .checked_add(&fact.allocated.checked_neg()?)?
                    .normalized()?;
                if remaining.coefficient != 0
                    && remaining.coefficient.signum() != fact.capacity.coefficient.signum()
                {
                    return Err(conflict());
                }
                Ok(SourceAllocationResidualV1 {
                    source_root,
                    facet,
                    remaining,
                })
            })
            .collect()
    }
}

impl IdentityEngine {
    /// Validate an explicitly supplied immutable revision family without
    /// choosing an alternative by arrival order. This structural view is not
    /// financial admission, proof of prior ownership or a durable append.
    pub fn revision_chain(
        revisions: &[AssociationRevisionV1],
        root: RecordId,
    ) -> Result<Vec<AssociationRevisionV1>, EngineError> {
        if root.0.is_nil() || revisions.is_empty() || revisions.len() > 16384 {
            return Err(conflict());
        }
        let mut by_id = BTreeMap::new();
        let mut successors = BTreeMap::new();
        for revision in revisions {
            if revision.schema_version != 1 {
                return Err(EngineError::version());
            }
            if revision.id.0.is_nil()
                || revision.root != root
                || revision.legs.is_empty()
                || revision.legs.len() > 1024
                || revision.allocations.len() > 16384
                || revision.retired_recognitions.len() > 16384
                || revision.governing_policy.id.0.is_nil()
                || revision.governing_policy.version == 0
                || revision.interpretation.authority != AuthorityKind::Owner
                || revision.interpretation.kind != DecisionKind::Interpretation
                || revision.interpretation.reason.trim().is_empty()
                || revision.interpretation.reason.len() > 4096
                || by_id.insert(revision.id, revision).is_some()
            {
                return Err(conflict());
            }
            let legs: BTreeSet<_> = revision.legs.iter().copied().collect();
            let retired: BTreeSet<_> = revision.retired_recognitions.iter().copied().collect();
            if legs.len() != revision.legs.len()
                || legs.iter().any(|id| id.0.is_nil())
                || retired.len() != revision.retired_recognitions.len()
                || retired.iter().any(|id| id.0.is_nil())
                || revision
                    .allocations
                    .iter()
                    .any(|edge| !legs.contains(&edge.leg))
                || revision.survivor.is_some_and(|id| !legs.contains(&id))
            {
                return Err(conflict());
            }
            match revision.prior_revision {
                None if revision.id == root => {}
                Some(prior) if revision.id != root && prior != revision.id => {
                    if successors.insert(prior, revision.id).is_some() {
                        return Err(conflict());
                    }
                }
                _ => return Err(conflict()),
            }
        }
        let first = by_id.get(&root).ok_or_else(conflict)?;
        let mut current = root;
        let mut seen = BTreeSet::new();
        let mut ordered = Vec::with_capacity(revisions.len());
        loop {
            if !seen.insert(current) {
                return Err(conflict());
            }
            let revision = by_id.get(&current).ok_or_else(conflict)?;
            if revision.kind != first.kind {
                return Err(conflict());
            }
            ordered.push((*revision).clone());
            match successors.get(&current) {
                Some(next) => current = *next,
                None => break,
            }
        }
        if ordered.len() != revisions.len() {
            return Err(conflict());
        }
        Ok(ordered)
    }
}

impl IdentityEngine {
    /// Exact currently ordered source version. Parser/capture identity is not
    /// economic identity, and receipt order cannot resolve competing revisions.
    pub fn captured_facet(
        state: &DomainState,
        binding: &CapturedFacetBindingV1,
        unit: UnitId,
    ) -> Result<ResolvedCapturedFacetV1, EngineError> {
        let Some(AuthoritativeRecordV1::RelationshipCapture(capture)) =
            state.records.get(&binding.capture)
        else {
            return Err(conflict());
        };
        if capture.id != binding.capture || capture.schema_version != 1 {
            return Err(conflict());
        }
        let mut versions = BTreeMap::<String, RelationshipSourceRowV1>::new();
        for record in state.records.values() {
            let AuthoritativeRecordV1::RelationshipCapture(candidate) = record else {
                continue;
            };
            if candidate.source != capture.source {
                continue;
            }
            RelationshipSourceRowV1::validate_all(&candidate.rows)?;
            let Some(AuthoritativeRecordV1::ImportReceipt(receipt)) =
                state.records.get(&candidate.receipt_id)
            else {
                return Err(conflict());
            };
            if candidate.schema_version != 1
                || receipt.source != candidate.source
                || receipt.adapter_version != "synthetic-relationships/1"
                || !matches!(receipt.parser_version.as_str(), "1" | "2")
                || receipt.contents != vec![candidate.raw.clone()]
                || !receipt.complete
                || receipt.truncated
                || receipt.result != ImportResult::Complete
            {
                return Err(conflict());
            }
            for row in &candidate.rows {
                if row.source_record != binding.source_record {
                    continue;
                }
                if versions
                    .get(&row.source_revision)
                    .is_some_and(|old| old != row)
                {
                    return Err(conflict());
                }
                versions.insert(row.source_revision.clone(), row.clone());
                if versions.len() > 16384 {
                    return Err(conflict());
                }
            }
        }
        let mut heads: BTreeSet<_> = versions.keys().cloned().collect();
        for row in versions.values() {
            if let Some(prior) = &row.prior_revision {
                if !versions.contains_key(prior) {
                    return Err(conflict());
                }
                heads.remove(prior);
            }
        }
        if heads.len() != 1 || !heads.contains(&binding.source_revision) {
            return Err(conflict());
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(binding.source_revision.as_str());
        while let Some(version) = current {
            if !seen.insert(version) {
                return Err(conflict());
            }
            current = versions
                .get(version)
                .ok_or_else(conflict)?
                .prior_revision
                .as_deref();
        }
        if seen.len() != versions.len() {
            return Err(conflict());
        }
        crate::relationship_parse::require_current(state, binding)?;
        Self::retained_facet(state, binding, unit)
    }
    /// Resolve immutable captured evidence, not eligibility for a new decision.
    /// New admission must additionally use `captured_facet`'s revision ordering.
    pub fn retained_facet(
        state: &DomainState,
        binding: &CapturedFacetBindingV1,
        unit: UnitId,
    ) -> Result<ResolvedCapturedFacetV1, EngineError> {
        let Some(AuthoritativeRecordV1::RelationshipCapture(capture)) =
            state.records.get(&binding.capture)
        else {
            return Err(conflict());
        };
        let Some(AuthoritativeRecordV1::ImportReceipt(receipt)) =
            state.records.get(&capture.receipt_id)
        else {
            return Err(conflict());
        };
        if capture.id != binding.capture {
            return Err(conflict());
        }
        crate::relationship_evidence::validate_metadata(capture, receipt)?;
        let row = crate::relationship_parse::retained_row(state, binding)?;
        let facet = row
            .facets
            .iter()
            .find(|facet| facet.lineage == binding.facet)
            .ok_or_else(conflict)?;
        let definition = crate::catalog::unit(state, unit)?;
        let date: TemporalValue = serde_json::from_value(
            serde_json::json!({"kind":"Date","civil":row.effective,"original_text":row.effective}),
        )
        .map_err(|_| conflict())?;
        Ok(ResolvedCapturedFacetV1 {
            fact_key: serde_json::to_string(&(&capture.source, &row.source_record, &facet.lineage))
                .map_err(|_| conflict())?,
            amount: facet.exact_amount(&definition)?,
            role: facet.role,
            group: facet.group.clone(),
            effective: date,
            status: row.status,
        })
    }
}
/// The current revision of each association family, by root.
pub(crate) fn current_heads(
    state: &DomainState,
) -> Result<BTreeMap<RecordId, AssociationRevisionV1>, EngineError> {
    let mut families = BTreeMap::<RecordId, Vec<AssociationRevisionV1>>::new();
    for record in state.records.values() {
        if let AuthoritativeRecordV1::AssociationRevision(r) = record {
            families.entry(r.root).or_default().push(r.clone());
        }
    }
    let mut heads = BTreeMap::new();
    for (root, revisions) in families {
        let head = IdentityEngine::revision_chain(&revisions, root)?
            .pop()
            .ok_or_else(EngineError::invalid)?;
        heads.insert(root, head);
    }
    Ok(heads)
}
/// The captured facts these current family revisions allocate. Only current
/// heads own capacity; prior snapshots never consume it a second time.
pub(crate) fn held_facts<'a>(
    state: &DomainState,
    heads: impl IntoIterator<Item = &'a AssociationRevisionV1>,
) -> Result<BTreeSet<String>, EngineError> {
    let mut held = BTreeSet::new();
    for edge in heads.into_iter().flat_map(|h| &h.captured_allocations) {
        let fact = IdentityEngine::retained_facet(state, &edge.source, edge.allocated.unit_id)?;
        held.insert(fact.fact_key);
    }
    Ok(held)
}
impl IdentityEngine {
    pub fn associate(
        state: &DomainState,
        request: &CompleteAssociationRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        if request.revision.kind == RelationshipKind::Duplicate {
            crate::association_duplicates::plan(state, request)
        } else {
            crate::association_admission::initial(state, request)
        }
    }
}

impl IdentityEngine {
    /// Check combined current ownership against a reconstructed native prefix.
    /// Each historical transition is separately admitted against its own prefix;
    /// this final audit must not reinterpret old evidence using the latest parser.
    pub fn audit(state: &DomainState) -> Result<(), EngineError> {
        let mut families = BTreeMap::<RecordId, Vec<AssociationRevisionV1>>::new();
        let mut reemissions = Vec::new();
        for (id, record) in &state.records {
            match record {
                AuthoritativeRecordV1::AssociationRevision(revision) => {
                    if *id != revision.id {
                        return Err(conflict());
                    }
                    families
                        .entry(revision.root)
                        .or_default()
                        .push(revision.clone());
                }
                AuthoritativeRecordV1::SegmentReemission(link) => reemissions.push(link),
                _ => {}
            }
        }
        if families.is_empty() && reemissions.is_empty() {
            return Ok(());
        }
        let index = state.accounting.as_ref().ok_or_else(conflict)?;
        if index.prefix.through != state.through
            || index.prefix.domain_digest != state.domain_digest
        {
            return Err(conflict());
        }
        // A segment replacement reversed each original and issued it again.
        // One family revision retires the original and names the re-emission,
        // which the family accepts as current only with identical economics.
        for link in reemissions {
            let (
                Some(AuthoritativeRecordV1::Recognition(original)),
                Some(AuthoritativeRecordV1::Recognition(again)),
            ) = (
                state.records.get(&RecordId(link.original.0)),
                state.records.get(&RecordId(link.replacement.0)),
            )
            else {
                return Err(conflict());
            };
            if !index.reversed.contains(&original.id)
                || !crate::correction::same_economics(original, again)
                || !families.values().flatten().any(|revision| {
                    revision.retired_recognitions.contains(&original.id)
                        && revision.recognitions.contains(&again.id)
                })
            {
                return Err(conflict());
            }
        }
        let mut facts = BTreeMap::new();
        let mut active = BTreeSet::new();
        let mut retired = BTreeSet::new();
        for (root, revisions) in families {
            let chain = Self::revision_chain(&revisions, root)?;
            for pair in chain.windows(2) {
                if pair[1]
                    .retired_recognitions
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    != pair[0].recognitions.iter().copied().collect()
                {
                    return Err(conflict());
                }
            }
            for revision in &chain {
                for id in &revision.retired_recognitions {
                    if !retired.insert(*id)
                        || !index.reversed.contains(id)
                        || !index.recognitions.contains_key(id)
                    {
                        return Err(conflict());
                    }
                }
            }
            let current = chain.last().ok_or_else(conflict)?;
            for id in &current.recognitions {
                if !active.insert(*id)
                    || index.reversed.contains(id)
                    || index.recognitions.get(id).is_none_or(Vec::is_empty)
                    || !matches!(state.records.get(&RecordId(id.0)),Some(AuthoritativeRecordV1::Recognition(r)) if r.id == *id)
                {
                    return Err(conflict());
                }
            }
            for edge in &current.captured_allocations {
                let fact = Self::retained_facet(state, &edge.source, edge.allocated.unit_id)?;
                if facts
                    .insert(fact.fact_key, root)
                    .is_some_and(|owner| owner != root)
                {
                    return Err(conflict());
                }
            }
        }
        if !active.is_disjoint(&retired) {
            return Err(conflict());
        }
        Ok(())
    }
}
