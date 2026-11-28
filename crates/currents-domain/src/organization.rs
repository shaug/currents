//! C10 owner organization: versioned definitions, notes and leg values.
//!
//! Definitions (party, alias, category, tag) and notes are
//! `OrganizationRecordV1` versions. A party, category or tag value on a leg is
//! an `OrganizationAssignmentV1` version. An edit appends the next version and
//! never changes an earlier one. `validate` holds every rule; the engine and
//! replay both apply it, so replay accepts exactly what the engine can plan.
use crate::reconciliation::derived_id;
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

pub struct OrganizationEngine;

/// One owner change: definitions and notes to create or revise, then values
/// to set, replace or clear on legs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationChangeV1 {
    pub records: Vec<OrganizationRecordV1>,
    pub values: Vec<LegValueRequestV1>,
}

/// The owner's value for one field on listed legs. `value` names a
/// definition's first version, and is `None` exactly when `action` is `Clear`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegValueRequestV1 {
    pub field: OrganizationFieldV1,
    pub legs: Vec<LegId>,
    pub value: Option<RecordId>,
    pub action: OverrideActionV1,
}

/// The plan for one change and the legs `Preserve` left alone because they
/// already hold a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationPreviewV1 {
    pub plan: DecisionPlan,
    pub skipped: Vec<LegId>,
}

fn invalid() -> EngineError {
    EngineError::invalid()
}

fn kind_of(field: OrganizationFieldV1) -> OrganizationKind {
    match field {
        OrganizationFieldV1::Party => OrganizationKind::Party,
        OrganizationFieldV1::Category => OrganizationKind::Category,
        OrganizationFieldV1::Tag => OrganizationKind::Tag,
    }
}

fn assignments(
    records: &BTreeMap<RecordId, AuthoritativeRecordV1>,
) -> impl Iterator<Item = &OrganizationAssignmentV1> {
    records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::OrganizationAssignment(a) => Some(a),
        _ => None,
    })
}

/// The latest version of the owner's `field` assignment that holds `leg`,
/// including one that cleared the value.
fn head(
    records: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    leg: LegId,
    field: OrganizationFieldV1,
) -> Option<&OrganizationAssignmentV1> {
    let superseded: BTreeSet<RecordId> =
        assignments(records).filter_map(|a| a.supersedes).collect();
    assignments(records).find(|a| {
        a.field == field
            && a.source == OrganizationSourceV1::Owner
            && a.legs.contains(&leg)
            && !superseded.contains(&a.id)
    })
}

/// An assignment's ID derives from its field, leg and the version it
/// supersedes, so a planned version names one record.
fn assignment_id(
    field: OrganizationFieldV1,
    leg: LegId,
    supersedes: Option<RecordId>,
) -> Result<RecordId, EngineError> {
    let bytes = serde_json::to_vec(&(field, leg, supersedes)).map_err(|_| invalid())?;
    Ok(derived_id(b"currents/organization-assignment/v1\0", &bytes))
}

impl OrganizationEngine {
    /// Preview one owner change. The plan holds the change's records, in
    /// request order, and changes no accounting. Each value on a leg is its
    /// own assignment: the next version of the leg's current one for that
    /// field, or a first version.
    pub fn set(
        state: &DomainState,
        change: &OrganizationChangeV1,
    ) -> Result<OrganizationPreviewV1, EngineError> {
        if change.records.is_empty() && change.values.is_empty() {
            return Err(invalid());
        }
        let mut records: Vec<AuthoritativeRecordV1> = change
            .records
            .iter()
            .cloned()
            .map(AuthoritativeRecordV1::Organization)
            .collect();
        let mut skipped = Vec::new();
        for request in &change.values {
            let mut seen = BTreeSet::new();
            if request.legs.is_empty()
                || !request.legs.iter().all(|leg| seen.insert(*leg))
                || (request.action == OverrideActionV1::Clear) != request.value.is_none()
            {
                return Err(invalid());
            }
            for &leg in &request.legs {
                let prior = head(&state.records, leg, request.field);
                let held = prior.is_some_and(|a| a.value.is_some());
                match request.action {
                    OverrideActionV1::Preserve if held => {
                        skipped.push(leg);
                        continue;
                    }
                    // There is no value to release.
                    OverrideActionV1::Clear if !held => return Err(invalid()),
                    _ => {}
                }
                let supersedes = prior.map(|a| a.id);
                let version = match prior {
                    None => 1,
                    Some(a) => a.version.checked_add(1).ok_or_else(invalid)?,
                };
                records.push(AuthoritativeRecordV1::OrganizationAssignment(
                    OrganizationAssignmentV1 {
                        id: assignment_id(request.field, leg, supersedes)?,
                        schema_version: 1,
                        version,
                        supersedes,
                        field: request.field,
                        legs: vec![leg],
                        value: request.value,
                        override_action: request.action,
                        source: OrganizationSourceV1::Owner,
                    },
                ));
            }
        }
        let mut next = state.records.clone();
        for record in &records {
            if let Some(id) = id_of(record) {
                next.insert(id, record.clone());
            }
        }
        validate(&state.records, &next, &records)?;
        Ok(OrganizationPreviewV1 {
            plan: DecisionPlan {
                records,
                invalidations: vec![],
                accounting: AccountingIntent::NoEffect(NoEffectV1 {
                    reason: "Owner organization".into(),
                }),
            },
            skipped,
        })
    }
}

fn id_of(record: &AuthoritativeRecordV1) -> Option<RecordId> {
    match record {
        AuthoritativeRecordV1::Organization(o) => Some(o.id),
        AuthoritativeRecordV1::OrganizationAssignment(a) => Some(a.id),
        _ => None,
    }
}

fn organization(
    records: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    id: RecordId,
) -> Option<&OrganizationRecordV1> {
    match records.get(&id) {
        Some(AuthoritativeRecordV1::Organization(o)) => Some(o),
        _ => None,
    }
}

fn is_leg(records: &BTreeMap<RecordId, AuthoritativeRecordV1>, id: RecordId) -> bool {
    matches!(
        records.get(&id),
        Some(AuthoritativeRecordV1::FinancialLeg(_))
    )
}

/// A definition's first version, which values and parents name.
fn first_version(
    records: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    id: RecordId,
    kind: OrganizationKind,
) -> bool {
    organization(records, id).is_some_and(|o| o.supersedes.is_none() && o.kind == kind)
}

/// Each version's one successor. A second successor of one version is a fork.
fn successors(
    versions: impl Iterator<Item = (RecordId, Option<RecordId>)>,
) -> Result<BTreeMap<RecordId, RecordId>, EngineError> {
    let mut found = BTreeMap::new();
    for (id, supersedes) in versions {
        if let Some(prior) = supersedes
            && found.insert(prior, id).is_some()
        {
            return Err(invalid());
        }
    }
    Ok(found)
}

/// Check every organization record and assignment in `records`, the new
/// records of one commit, against `next`, the records after that commit.
/// `prior` holds the records before it.
///
/// - Every new record has a fresh, distinct ID.
/// - A first version is version 1 and supersedes nothing. A revision is the
///   next version of the version it supersedes, which no other record
///   supersedes, and keeps its kind or field.
/// - A party, category or tag definition has no targets. A party has no
///   parent; a category or tag may name the first version of another of its
///   kind; an alias names a party's first version. No parent chain returns
///   to where it started. A note targets existing legs and has no parent.
/// - An assignment holds the owner's value on existing legs. `Clear` names
///   no value and supersedes a version that holds one; any other action names
///   the first version of a definition of the field's kind, and `Preserve`
///   never supersedes a value. A revision keeps its predecessor's legs.
/// - Each leg has at most one latest owner assignment per field.
pub(crate) fn validate(
    prior: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    next: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    records: &[AuthoritativeRecordV1],
) -> Result<(), EngineError> {
    let mut fresh = BTreeSet::new();
    let mut touched = BTreeSet::new();
    let mut organized = false;
    for record in records {
        let Some(id) = id_of(record) else {
            continue;
        };
        if id.0.is_nil()
            || prior.contains_key(&id)
            || !fresh.insert(id)
            || next.get(&id) != Some(record)
        {
            return Err(invalid());
        }
        match record {
            AuthoritativeRecordV1::Organization(o) => {
                definition(next, o)?;
                organized = true;
            }
            AuthoritativeRecordV1::OrganizationAssignment(a) => {
                assignment(next, a)?;
                touched.extend(a.legs.iter().map(|leg| (*leg, a.field as u8)));
            }
            _ => {}
        }
    }
    if fresh.is_empty() {
        return Ok(());
    }
    let definitions: Vec<&OrganizationRecordV1> = next
        .values()
        .filter_map(|record| match record {
            AuthoritativeRecordV1::Organization(o) => Some(o),
            _ => None,
        })
        .collect();
    let next_definition = successors(definitions.iter().map(|o| (o.id, o.supersedes)))?;
    let next_assignment = successors(assignments(next).map(|a| (a.id, a.supersedes)))?;
    if organized {
        acyclic(next, &definitions, &next_definition)?;
    }
    // At most one latest owner assignment per leg and field.
    let mut current = BTreeSet::new();
    for a in assignments(next) {
        if a.source != OrganizationSourceV1::Owner || next_assignment.contains_key(&a.id) {
            continue;
        }
        for leg in &a.legs {
            let key = (*leg, a.field as u8);
            if touched.contains(&key) && !current.insert(key) {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

/// The rules one definition or note version must meet on its own.
fn definition(
    next: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    o: &OrganizationRecordV1,
) -> Result<(), EngineError> {
    // Allocations belong to the reporting split leaf, not this one.
    if !o.allocations.is_empty() || o.kind == OrganizationKind::ReportingAllocation {
        return Err(EngineError::version());
    }
    match o.supersedes {
        None if o.version == 1 => {}
        Some(prior) if prior != o.id => {
            let prior = organization(next, prior).ok_or_else(invalid)?;
            if prior.kind != o.kind || prior.version.checked_add(1) != Some(o.version) {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    let parent = |kind| o.parent.is_some_and(|p| first_version(next, p, kind));
    let valid = match o.kind {
        OrganizationKind::Party => o.targets.is_empty() && o.parent.is_none(),
        OrganizationKind::Category | OrganizationKind::Tag => {
            o.targets.is_empty() && (o.parent.is_none() || parent(o.kind))
        }
        OrganizationKind::Alias => o.targets.is_empty() && parent(OrganizationKind::Party),
        OrganizationKind::Note => {
            let mut seen = BTreeSet::new();
            o.parent.is_none()
                && !o.targets.is_empty()
                && o.targets
                    .iter()
                    .all(|t| seen.insert(*t) && is_leg(next, *t))
        }
        OrganizationKind::ReportingAllocation => false,
    };
    if !valid {
        return Err(invalid());
    }
    Ok(())
}

/// The rules one assignment version must meet on its own.
fn assignment(
    next: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    a: &OrganizationAssignmentV1,
) -> Result<(), EngineError> {
    // Rule and proposal sources belong to later CUR-019 leaves.
    if a.schema_version != 1 || a.source != OrganizationSourceV1::Owner {
        return Err(EngineError::version());
    }
    let mut seen = BTreeSet::new();
    if a.legs.is_empty()
        || !a
            .legs
            .iter()
            .all(|leg| seen.insert(*leg) && is_leg(next, RecordId(leg.0)))
    {
        return Err(invalid());
    }
    let named = match (a.override_action, a.value) {
        (OverrideActionV1::Clear, None) => true,
        (OverrideActionV1::Clear, Some(_)) | (_, None) => false,
        (_, Some(value)) => first_version(next, value, kind_of(a.field)),
    };
    if !named {
        return Err(invalid());
    }
    let prior = match a.supersedes {
        None if a.version == 1 => None,
        Some(prior) if prior != a.id => match next.get(&prior) {
            Some(AuthoritativeRecordV1::OrganizationAssignment(p))
                if p.field == a.field
                    && p.source == a.source
                    && p.legs == a.legs
                    && p.version.checked_add(1) == Some(a.version) =>
            {
                Some(p)
            }
            _ => return Err(invalid()),
        },
        _ => return Err(invalid()),
    };
    let held = prior.is_some_and(|p| p.value.is_some());
    match a.override_action {
        OverrideActionV1::Preserve if held => Err(invalid()),
        OverrideActionV1::Clear if !held => Err(invalid()),
        _ => Ok(()),
    }
}

/// No category, tag or alias parent chain, read through each definition's
/// latest version, returns to where it started.
fn acyclic(
    next: &BTreeMap<RecordId, AuthoritativeRecordV1>,
    definitions: &[&OrganizationRecordV1],
    successor: &BTreeMap<RecordId, RecordId>,
) -> Result<(), EngineError> {
    let latest = |first: RecordId| -> Result<&OrganizationRecordV1, EngineError> {
        let mut id = first;
        for _ in 0..=definitions.len() {
            match successor.get(&id) {
                Some(next_id) => id = *next_id,
                None => return organization(next, id).ok_or_else(invalid),
            }
        }
        Err(invalid())
    };
    for start in definitions.iter().filter(|o| o.supersedes.is_none()) {
        let mut at = start.id;
        for _ in 0..=definitions.len() {
            match latest(at)?.parent {
                None => break,
                Some(parent) if parent == start.id => return Err(invalid()),
                Some(parent) => at = parent,
            }
        }
    }
    Ok(())
}
