//! Effective-dated dataset policy revisions (C09). A revision never changes
//! a bootstrap `DatasetPolicy` record or an earlier revision. The governing
//! lookup reads only revisions: never a bootstrap field, and never the latest
//! revision outside its effective dates.
use crate::catalog::{incomplete, unit};
use crate::reconciliation::derived_id;
use currents_types::*;
use std::collections::BTreeSet;

pub struct PolicyEngine;

fn invalid() -> EngineError {
    EngineError::invalid()
}

fn revisions(state: &DomainState) -> impl Iterator<Item = &DatasetPolicyRevisionV1> {
    state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::DatasetPolicyRevision(revision) => Some(revision),
        _ => None,
    })
}

/// The one dataset the state holds.
fn dataset(state: &DomainState) -> Result<DatasetId, EngineError> {
    let mut found = state.records.values().filter_map(|record| match record {
        AuthoritativeRecordV1::Dataset(dataset) => Some(dataset.id),
        _ => None,
    });
    let dataset = found.next().ok_or_else(incomplete)?;
    if found.next().is_some() {
        return Err(invalid());
    }
    Ok(dataset)
}

/// Every connection an import receipt or an account alias names.
fn connections(state: &DomainState) -> BTreeSet<RecordId> {
    let mut found = BTreeSet::new();
    for record in state.records.values() {
        match record {
            AuthoritativeRecordV1::ImportReceipt(receipt) => {
                found.insert(receipt.source.connection_id);
            }
            AuthoritativeRecordV1::AccountAlias(alias) => {
                found.insert(alias.alias.source.connection_id);
            }
            AuthoritativeRecordV1::AccountComponent(component) => {
                found.extend(component.aliases.iter().map(|a| a.source.connection_id));
            }
            AuthoritativeRecordV1::AccountCatalogRevision(revision) => {
                found.extend(
                    revision
                        .catalog
                        .aliases
                        .iter()
                        .map(|a| a.source.connection_id),
                );
            }
            _ => {}
        }
    }
    found
}

fn declared_accounts(state: &DomainState) -> BTreeSet<AccountId> {
    state
        .records
        .values()
        .filter_map(|record| match record {
            AuthoritativeRecordV1::AccountComponent(component) => Some(component.account_id),
            _ => None,
        })
        .collect()
}

fn distinct<T: Ord>(items: impl Iterator<Item = T>) -> bool {
    let mut seen = BTreeSet::new();
    items.into_iter().all(|item| seen.insert(item))
}

/// The value a change adopts when it is known, after checking that its basis
/// names this dataset at or before the state's commit.
fn adopted<'a, T>(
    state: &DomainState,
    dataset: DatasetId,
    change: &'a Option<PolicyChangeV1<T>>,
) -> Result<Option<&'a T>, EngineError> {
    let Some(change) = change else {
        return Ok(None);
    };
    if change.basis.dataset_id != dataset || change.basis.through_commit > state.through {
        return Err(invalid());
    }
    Ok(match &change.value {
        SourceValue::Known(value) => Some(value),
        _ => None,
    })
}

fn contains(scope: &DateScope, day: Date) -> bool {
    scope.start.is_none_or(|start| start <= day) && scope.end.is_none_or(|end| day <= end)
}

/// A revision's ID derives from its family and version, so one family
/// version names one record and replay derives the ID again.
fn revision_id(policy_id: RecordId, version: u32) -> Result<RecordId, EngineError> {
    let bytes = serde_json::to_vec(&(policy_id, version)).map_err(|_| invalid())?;
    Ok(derived_id(b"currents/dataset-policy-revision/v1\0", &bytes))
}

/// The request a committed revision records.
pub(crate) fn request_of(revision: &DatasetPolicyRevisionV1) -> SetDatasetPolicyV1 {
    SetDatasetPolicyV1 {
        policy_id: revision.policy_id,
        prior_version: revision.prior_version,
        effective_scope: revision.effective_scope.clone(),
        ownership_perimeter: revision.ownership_perimeter.clone(),
        source_priority: revision.source_priority.clone(),
        freshness_thresholds: revision.freshness_thresholds.clone(),
        timezone: revision.timezone.clone(),
        reporting_currency: revision.reporting_currency.clone(),
    }
}

/// The value the highest-version revision in `in_effect` that sets the field
/// gives it.
fn setting<T: Clone>(
    in_effect: &[&DatasetPolicyRevisionV1],
    field: impl Fn(&DatasetPolicyRevisionV1) -> &Option<PolicyChangeV1<T>>,
) -> PolicySettingV1<T> {
    in_effect
        .iter()
        .find_map(|revision| {
            field(revision).as_ref().map(|change| PolicySettingV1::Set {
                version: revision.version,
                value: change.value.clone(),
            })
        })
        .unwrap_or(PolicySettingV1::NotSet)
}

impl PolicyEngine {
    /// Record an owner's policy change as the next revision of its family.
    /// The plan holds that one revision and changes no accounting.
    pub fn set(
        state: &DomainState,
        request: &SetDatasetPolicyV1,
    ) -> Result<DecisionPlan, EngineError> {
        if !matches!(
            state.records.get(&request.policy_id),
            Some(AuthoritativeRecordV1::DatasetPolicy(_))
        ) {
            return Err(invalid());
        }
        // A dataset has one revision family, and each revision follows the
        // family's latest.
        let mut latest = None;
        for revision in revisions(state) {
            if revision.policy_id != request.policy_id {
                return Err(invalid());
            }
            latest = latest.max(Some(revision.version));
        }
        if request.prior_version != latest {
            return Err(invalid());
        }
        let version = match latest {
            None => 1,
            Some(latest) => latest.checked_add(1).ok_or_else(invalid)?,
        };
        let scope = &request.effective_scope;
        if scope.basis != DateBasis::Effective
            || matches!((scope.start, scope.end), (Some(start), Some(end)) if start > end)
        {
            return Err(invalid());
        }
        if request.ownership_perimeter.is_none()
            && request.source_priority.is_none()
            && request.freshness_thresholds.is_none()
            && request.timezone.is_none()
            && request.reporting_currency.is_none()
        {
            return Err(invalid());
        }
        let dataset = dataset(state)?;
        if let Some(entries) = adopted(state, dataset, &request.ownership_perimeter)? {
            let accounts = declared_accounts(state);
            if !distinct(entries.iter().map(|e| e.account_id))
                || entries.iter().any(|e| {
                    !accounts.contains(&e.account_id)
                        || e.denominator.0 == 0
                        || e.numerator > e.denominator
                })
            {
                return Err(invalid());
            }
        }
        let connections = connections(state);
        if let Some(order) = adopted(state, dataset, &request.source_priority)?
            && (!distinct(order.iter()) || order.iter().any(|s| !connections.contains(s)))
        {
            return Err(invalid());
        }
        if let Some(thresholds) = adopted(state, dataset, &request.freshness_thresholds)?
            && (!distinct(thresholds.iter().map(|t| t.source))
                || thresholds
                    .iter()
                    .any(|t| t.seconds.0 == 0 || !connections.contains(&t.source)))
        {
            return Err(invalid());
        }
        adopted(state, dataset, &request.timezone)?;
        if let Some(currency) = adopted(state, dataset, &request.reporting_currency)?
            && unit(state, *currency)?.kind != UnitKind::Currency
        {
            return Err(invalid());
        }
        let revision = DatasetPolicyRevisionV1 {
            id: revision_id(request.policy_id, version)?,
            policy_id: request.policy_id,
            version,
            prior_version: request.prior_version,
            effective_scope: scope.clone(),
            ownership_perimeter: request.ownership_perimeter.clone(),
            source_priority: request.source_priority.clone(),
            freshness_thresholds: request.freshness_thresholds.clone(),
            timezone: request.timezone.clone(),
            reporting_currency: request.reporting_currency.clone(),
        };
        Ok(DecisionPlan {
            records: vec![AuthoritativeRecordV1::DatasetPolicyRevision(revision)],
            invalidations: vec![],
            accounting: AccountingIntent::NoEffect(NoEffectV1 {
                reason: "Dataset policy revision".into(),
            }),
        })
    }

    /// For each field, the value of the highest-version revision whose
    /// effective dates contain `day` and which sets that field, or `NotSet`.
    pub fn governing(state: &DomainState, day: Date) -> Result<GoverningPolicyV1, EngineError> {
        let mut in_effect: Vec<&DatasetPolicyRevisionV1> = revisions(state)
            .filter(|revision| contains(&revision.effective_scope, day))
            .collect();
        in_effect.sort_by_key(|revision| std::cmp::Reverse(revision.version));
        if !distinct(in_effect.iter().map(|r| r.version)) {
            return Err(invalid());
        }
        Ok(GoverningPolicyV1 {
            ownership_perimeter: setting(&in_effect, |r| &r.ownership_perimeter),
            source_priority: setting(&in_effect, |r| &r.source_priority),
            freshness_thresholds: setting(&in_effect, |r| &r.freshness_thresholds),
            timezone: setting(&in_effect, |r| &r.timezone),
            reporting_currency: setting(&in_effect, |r| &r.reporting_currency),
        })
    }
}
