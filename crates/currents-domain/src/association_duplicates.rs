//! Explicit N-way deduplication of already adopted economic movements.
use crate::{associations::IdentityEngine, recognition::RecognitionEngine};
use currents_types::*;
use std::collections::BTreeSet;
fn invalid() -> EngineError {
    EngineError::invalid()
}
pub(crate) fn plan(
    state: &DomainState,
    request: &CompleteAssociationRequestV1,
) -> Result<DecisionPlan, EngineError> {
    let r = &request.revision;
    IdentityEngine::revision_chain(std::slice::from_ref(r), r.root)?;
    if r.kind != RelationshipKind::Duplicate
        || r.prior_revision.is_some()
        || !request.events.is_empty()
        || !request.legs.is_empty()
        || !request.recognitions.is_empty()
        || !r.captured_allocations.is_empty()
        || !r.residuals.is_empty()
        || r.legs.len() < 2
        || r.allocations.len() != r.legs.len()
        || state.records.contains_key(&r.id)
    {
        return Err(invalid());
    }
    if !state
        .dataset_policies
        .contains_key(&(r.governing_policy.id, r.governing_policy.version))
    {
        return Err(invalid());
    }
    let survivor = r.survivor.ok_or_else(invalid)?;
    let Some(AuthoritativeRecordV1::FinancialLeg(chosen)) =
        state.records.get(&RecordId(survivor.0))
    else {
        return Err(invalid());
    };
    let index = state.accounting.as_ref().ok_or_else(invalid)?;
    if index.prefix.through != state.through {
        return Err(invalid());
    }
    let adoptions = RecognitionEngine::audit(state)?;
    let mut retired = BTreeSet::new();
    let mut superseded = BTreeSet::new();
    let mut owner = None;
    let mut covered = BTreeSet::new();
    for leg_id in &r.legs {
        let Some(AuthoritativeRecordV1::FinancialLeg(leg)) = state.records.get(&RecordId(leg_id.0))
        else {
            return Err(invalid());
        };
        if leg.account_id != chosen.account_id
            || leg.movement.normalized()? != chosen.movement.normalized()?
        {
            return Err(invalid());
        }
        if state.records.values().any(|record|matches!(record,AuthoritativeRecordV1::AssociationRevision(v) if v.legs.contains(leg_id))) {return Err(invalid());}
        let owners: Vec<_> = adoptions
            .iter()
            .filter(|a| a.legs.contains(leg_id))
            .collect();
        if owners.len() != 1 {
            return Err(invalid());
        }
        let adoption = owners[0];
        if adoption.status != ActivityStatusV1::Posted
            || adoption.legs != vec![*leg_id]
            || adoption.facets.len() != 1
            || adoption.opening.is_some()
        {
            return Err(invalid());
        }
        let recognition = adoption.recognition.ok_or_else(invalid)?;
        if index.reversed.contains(&recognition)
            || index
                .recognitions
                .get(&recognition)
                .is_none_or(Vec::is_empty)
        {
            return Err(invalid());
        }
        if *leg_id == survivor {
            owner = Some(recognition);
        } else {
            if !retired.insert(recognition) || !superseded.insert(adoption.id) {
                return Err(invalid());
            }
        }
        let source = &adoption.facets[0].source;
        let fact = crate::recognition::fact(state, source)?;
        if !covered.insert(fact) {
            return Err(invalid());
        }
        let edges: Vec<_> = r
            .allocations
            .iter()
            .filter(|edge| edge.source == *source)
            .collect();
        if edges.len() != 1
            || edges[0].leg != survivor
            || edges[0].mode != FacetAllocationModeV1::Corroborating
            || edges[0].allocated.normalized()? != chosen.movement.normalized()?
        {
            return Err(invalid());
        }
    }
    if r.recognitions != vec![owner.ok_or_else(invalid)?]
        || retired != r.retired_recognitions.iter().copied().collect()
        || retired.len() != r.retired_recognitions.len()
        || superseded != r.superseded_adoptions.iter().copied().collect()
        || superseded.len() != r.superseded_adoptions.len()
    {
        return Err(invalid());
    }
    Ok(DecisionPlan {
        records: vec![AuthoritativeRecordV1::AssociationRevision(r.clone())],
        invalidations: r.legs.iter().map(|id| RecordId(id.0)).collect(),
        accounting: AccountingIntent::Corrections(
            r.retired_recognitions
                .iter()
                .map(|id| CorrectionV1 {
                    active_recognition: *id,
                    replacement: vec![],
                    remapping: vec![],
                    reason: r.interpretation.reason.clone(),
                })
                .collect(),
        ),
    })
}
