//! Explicit synthetic interpretation revisions. Original source evidence and
//! economic identity remain immutable; publication has no financial effect.
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub struct ValidatedRelationshipParseV1(RelationshipParseBatchV1);
impl ValidatedRelationshipParseV1 {
    pub fn new(batch: RelationshipParseBatchV1) -> Result<Self, EngineError> {
        metadata(&batch.revision, &batch.receipt)?;
        if batch.contents.len() != 1 {
            return Err(EngineError::invalid());
        }
        let c = &batch.contents[0];
        let p = &batch.revision.profile;
        if c.bytes.len() > 1_048_576
            || c.digest != p.digest
            || c.bytes.len() as u64 != p.byte_count.0
            || Digest(Sha256::digest(&c.bytes).into()) != c.digest
            || !crate::evidence::safe(&c.bytes)
        {
            return Err(EngineError::invalid());
        }
        let decoded: RelationshipCorrectionProfileV1 =
            serde_json::from_slice(&c.bytes).map_err(|_| EngineError::invalid())?;
        if decoded != batch.revision.correction {
            return Err(EngineError::invalid());
        }
        Ok(Self(batch))
    }
}
fn metadata(p: &RelationshipParseRevisionV1, r: &ImportReceiptV1) -> Result<(), EngineError> {
    RelationshipSourceRowV1::validate_all(std::slice::from_ref(&p.row))?;
    if p.schema_version != 1
        || p.correction.schema_version != 1
        || p.id.0.is_nil()
        || p.receipt_id.0.is_nil()
        || p.capture.0.is_nil()
        || p.id == p.receipt_id
        || p.id == p.capture
        || p.receipt_id == p.capture
        || p.receipt_id != r.id
        || p.prior_parse == Some(p.id)
        || p.prior_parse.is_some_and(|id| id.0.is_nil())
        || r.source.source != "synthetic"
        || r.source.device_id.is_some()
        || r.source.connection_id.0.is_nil()
        || r.adapter_version != "synthetic-relationships/1"
        || r.parser_version != RELATIONSHIP_CORRECTION_PARSER
        || r.requested_coverage != r.reported_coverage
        || r.requested_coverage.len() > 16
        || r.result != ImportResult::Complete
        || !r.complete
        || r.truncated
        || r.cursor != SourceValue::Absent
        || r.contents != vec![p.profile.clone()]
        || p.profile.media_type != RELATIONSHIP_CORRECTION_MEDIA
        || p.profile.byte_count.0 > 1_048_576
    {
        return Err(EngineError::invalid());
    }
    Ok(())
}
fn capture(state: &DomainState, id: RecordId) -> Result<&RelationshipCaptureV1, EngineError> {
    let Some(AuthoritativeRecordV1::RelationshipCapture(c)) = state.records.get(&id) else {
        return Err(EngineError::invalid());
    };
    let Some(AuthoritativeRecordV1::ImportReceipt(r)) = state.records.get(&c.receipt_id) else {
        return Err(EngineError::invalid());
    };
    crate::relationship_evidence::validate_metadata(c, r)?;
    if c.id != id {
        return Err(EngineError::invalid());
    }
    Ok(c)
}
/// Pure verification also used by domain replay. Authenticated replay separately
/// verifies the retained profile bytes before invoking this semantic check.
pub(crate) fn validate(
    state: &DomainState,
    p: &RelationshipParseRevisionV1,
    receipt: &ImportReceiptV1,
) -> Result<(), EngineError> {
    let root = capture(state, p.capture)?;
    let base = root
        .rows
        .iter()
        .find(|r| {
            r.source_record == p.row.source_record && r.source_revision == p.row.source_revision
        })
        .ok_or_else(EngineError::invalid)?;
    let mut chain = vec![];
    let mut seen = BTreeSet::new();
    let mut current = p;
    let mut current_receipt = receipt;
    loop {
        if !seen.insert(current.id) || seen.len() > 1024 {
            return Err(EngineError::invalid());
        }
        metadata(current, current_receipt)?;
        // One exact original root per parse chain. Equivalent recaptures cannot
        // create separate financial capacity: current selection is source-wide.
        if current.capture != p.capture
            || current_receipt.source != root.source
            || current.row.source_record != base.source_record
            || current.row.source_revision != base.source_revision
        {
            return Err(EngineError::invalid());
        }
        chain.push(current);
        let Some(id) = current.prior_parse else {
            break;
        };
        let Some(AuthoritativeRecordV1::RelationshipParseRevision(prior)) = state.records.get(&id)
        else {
            return Err(EngineError::invalid());
        };
        let Some(AuthoritativeRecordV1::ImportReceipt(r)) = state.records.get(&prior.receipt_id)
        else {
            return Err(EngineError::invalid());
        };
        if prior.id != id {
            return Err(EngineError::invalid());
        }
        current = prior;
        current_receipt = r;
    }
    let mut expected = base.clone();
    for revision in chain.into_iter().rev() {
        expected = revision.correction.apply(&expected)?;
        if expected != revision.row {
            return Err(EngineError::invalid());
        }
    }
    Ok(())
}
pub fn retained_row(
    state: &DomainState,
    binding: &CapturedFacetBindingV1,
) -> Result<RelationshipSourceRowV1, EngineError> {
    let c = capture(state, binding.capture)?;
    let base = c
        .rows
        .iter()
        .find(|r| {
            r.source_record == binding.source_record && r.source_revision == binding.source_revision
        })
        .ok_or_else(EngineError::invalid)?;
    let Some(id) = binding.parse_revision else {
        return Ok(base.clone());
    };
    let Some(AuthoritativeRecordV1::RelationshipParseRevision(p)) = state.records.get(&id) else {
        return Err(EngineError::invalid());
    };
    let Some(AuthoritativeRecordV1::ImportReceipt(r)) = state.records.get(&p.receipt_id) else {
        return Err(EngineError::invalid());
    };
    if p.id != id
        || p.capture != c.id
        || p.row.source_record != binding.source_record
        || p.row.source_revision != binding.source_revision
    {
        return Err(EngineError::invalid());
    }
    validate(state, p, r)?;
    Ok(p.row.clone())
}
pub(crate) fn require_current(
    state: &DomainState,
    binding: &CapturedFacetBindingV1,
) -> Result<(), EngineError> {
    let root = capture(state, binding.capture)?;
    let mut revisions = std::collections::BTreeMap::new();
    for record in state.records.values() {
        let AuthoritativeRecordV1::RelationshipParseRevision(p) = record else {
            continue;
        };
        let c = capture(state, p.capture)?;
        if c.source != root.source
            || p.row.source_record != binding.source_record
            || p.row.source_revision != binding.source_revision
        {
            continue;
        }
        let Some(AuthoritativeRecordV1::ImportReceipt(r)) = state.records.get(&p.receipt_id) else {
            return Err(EngineError::invalid());
        };
        validate(state, p, r)?;
        revisions.insert(p.id, p);
        if revisions.len() > 1024 {
            return Err(EngineError::invalid());
        }
    }
    if revisions.is_empty() {
        return if binding.parse_revision.is_none() {
            Ok(())
        } else {
            Err(EngineError::invalid())
        };
    }
    let mut heads: BTreeSet<_> = revisions.keys().copied().collect();
    for p in revisions.values() {
        if let Some(prior) = p.prior_parse {
            if !revisions.contains_key(&prior) {
                return Err(EngineError::invalid());
            }
            heads.remove(&prior);
        }
    }
    if heads.len() != 1 || !binding.parse_revision.is_some_and(|id| heads.contains(&id)) {
        return Err(EngineError::invalid());
    }
    // Unique head alone is insufficient if an unrelated root/cycle is retained.
    let mut seen = BTreeSet::new();
    let mut cursor = binding.parse_revision;
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return Err(EngineError::invalid());
        }
        cursor = revisions
            .get(&id)
            .ok_or_else(EngineError::invalid)?
            .prior_parse;
    }
    if seen.len() != revisions.len() {
        return Err(EngineError::invalid());
    }
    Ok(())
}
pub struct RelationshipParseEngine;
impl RelationshipParseEngine {
    pub fn publish(
        state: &DomainState,
        batch: ValidatedRelationshipParseV1,
    ) -> Result<DecisionPlan, EngineError> {
        plan(state, &batch.0.revision, &batch.0.receipt)
    }
}
pub(crate) fn plan(
    state: &DomainState,
    p: &RelationshipParseRevisionV1,
    r: &ImportReceiptV1,
) -> Result<DecisionPlan, EngineError> {
    if state.records.contains_key(&p.id) || state.records.contains_key(&r.id) {
        return Err(EngineError::invalid());
    }
    validate(state, p, r)?;
    Ok(DecisionPlan {
        records: vec![
            AuthoritativeRecordV1::ImportReceipt(r.clone()),
            AuthoritativeRecordV1::RelationshipParseRevision(p.clone()),
        ],
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: "Retained explicit synthetic parse correction only".into(),
        }),
    })
}
