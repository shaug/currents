//! What a reconciliation claim read from the state, and the digest of it.
//!
//! The digest covers the current version of each named observation, each
//! recognized leg the claim compared, each scope account's catalog terms
//! except its display name, the coverage report the claim used and the
//! comparison version. Legs are keyed by their stable identity, so an
//! identity-only correction, which replaces recognition and posting IDs and
//! keeps the legs, leaves the digest unchanged. A claim whose recorded digest
//! differs from the digest at a later state no longer applies there.
use crate::activity_predicate::{civil_date, within};
use crate::catalog::{CatalogEngine, incomplete};
use crate::reconciliation::InvalidationReasonV1;
use currents_types::*;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The scope of an Activity claim: one account, a complete interval by one
/// date basis, and the named evidence.
#[derive(Clone, Copy)]
pub(crate) struct ActivityScope<'a> {
    pub(crate) account: AccountId,
    pub(crate) start: Date,
    pub(crate) end: Date,
    pub(crate) basis: DateBasis,
    pub(crate) evidence: &'a [RecordId],
}

impl<'a> ActivityScope<'a> {
    /// The scope of `accounts` over `cutoffs`, which must name one account
    /// and a bounded interval.
    pub(crate) fn new(
        accounts: &[AccountId],
        cutoffs: &DateScope,
        evidence: &'a [RecordId],
    ) -> Result<Self, EngineError> {
        let [account] = accounts else {
            return Err(EngineError::version());
        };
        let (Some(start), Some(end)) = (cutoffs.start, cutoffs.end) else {
            return Err(EngineError::invalid());
        };
        if start > end {
            return Err(EngineError::invalid());
        }
        Ok(Self {
            account: *account,
            start,
            end,
            basis: cutoffs.basis,
            evidence,
        })
    }

    /// The leg's date by the scope's basis, when it is a civil date.
    fn day(&self, leg: &FinancialLegV1) -> Option<Date> {
        civil_date(self.basis, &leg.effective, &leg.posting)
    }

    /// `day` falls inside the claim's interval.
    pub(crate) fn contains(&self, day: Date) -> bool {
        within(self.start, self.end, day)
    }

    /// The leg affects the claim: it is on the scope account inside the
    /// interval, or on it with a date that is not a civil date, or it
    /// allocates a named observation.
    pub(crate) fn relevant(&self, leg: &FinancialLegV1) -> bool {
        let on_account =
            leg.account_id == self.account && self.day(leg).is_none_or(|day| self.contains(day));
        on_account
            || leg
                .allocations
                .iter()
                .any(|a| self.evidence.contains(&a.observation))
    }

    /// The leg is on the scope account with a civil date inside the interval.
    pub(crate) fn inside(&self, leg: &FinancialLegV1) -> bool {
        leg.account_id == self.account && self.day(leg).is_some_and(|day| self.contains(day))
    }

    /// The leg is on the scope account and has no civil date by the basis.
    pub(crate) fn unmapped(&self, leg: &FinancialLegV1) -> bool {
        leg.account_id == self.account && self.day(leg).is_none()
    }
}

/// Every leg of a live Activity recognition, in leg order. A recognition is
/// live until the accounting index records its reversal. The index must be
/// current with the state.
pub(crate) fn live_legs(state: &DomainState) -> Result<Vec<&FinancialLegV1>, EngineError> {
    let index = state.accounting.as_ref().ok_or_else(incomplete)?;
    if index.prefix.through != state.through || index.prefix.domain_digest != state.domain_digest {
        return Err(stale());
    }
    let mut legs = BTreeMap::<LegId, &FinancialLegV1>::new();
    for record in state.records.values() {
        let AuthoritativeRecordV1::Recognition(recognition) = record else {
            continue;
        };
        if recognition.kind != RecognitionKind::Activity || index.reversed.contains(&recognition.id)
        {
            continue;
        }
        for id in &recognition.leg_ids {
            let Some(AuthoritativeRecordV1::FinancialLeg(leg)) = state.records.get(&RecordId(id.0))
            else {
                return Err(incomplete());
            };
            if leg.id != *id {
                return Err(EngineError::invalid());
            }
            legs.insert(*id, leg);
        }
    }
    Ok(legs.into_values().collect())
}

pub(crate) fn stale() -> EngineError {
    EngineError {
        class: ErrorClass::StaleBasis,
        affected_scope: vec![],
        next_action: SafeNextAction::RefreshBasis,
    }
}

/// The named receipt and coverage that report the whole interval complete by
/// the scope's basis, if any.
pub(crate) fn coverage<'a>(
    state: &'a DomainState,
    scope: &ActivityScope<'_>,
) -> Option<(RecordId, &'a CoverageV1)> {
    scope.evidence.iter().find_map(|id| {
        let Some(AuthoritativeRecordV1::ImportReceipt(receipt)) = state.records.get(id) else {
            return None;
        };
        receipt
            .reported_coverage
            .iter()
            .find(|c| {
                c.status == CoverageStatus::Complete
                    && c.scope.basis == scope.basis
                    && c.scope.start.is_none_or(|start| start <= scope.start)
                    && c.scope.end.is_none_or(|end| scope.end <= end)
            })
            .map(|c| (*id, c))
    })
}

/// The latest explicit revision of `observation`.
pub(crate) fn current_version(
    state: &DomainState,
    observation: RecordId,
) -> Result<RecordId, EngineError> {
    let mut current = observation;
    let mut seen = BTreeSet::from([current]);
    loop {
        let mut next = state.records.values().filter_map(|record| match record {
            AuthoritativeRecordV1::ObservationVersion(o) if o.prior_version == Some(current) => {
                Some(o.id)
            }
            _ => None,
        });
        let Some(successor) = next.next() else {
            return Ok(current);
        };
        if next.next().is_some() || !seen.insert(successor) {
            return Err(EngineError::invalid());
        }
        current = successor;
    }
}

/// The account's current catalog terms.
pub(crate) fn catalog(
    state: &DomainState,
    account: AccountId,
) -> Result<SetAccountCatalogV1, EngineError> {
    let component = state
        .records
        .values()
        .find_map(|record| match record {
            AuthoritativeRecordV1::AccountComponent(c) if c.account_id == account => Some(c.id),
            _ => None,
        })
        .ok_or_else(EngineError::invalid)?;
    Ok(CatalogEngine::current(state, component)?.catalog)
}

/// What an Activity claim's dependency digest covers, read at one state.
#[derive(Clone)]
pub(crate) struct ActivityInputs {
    /// Each named observation and its latest explicit revision.
    observations: Vec<(RecordId, RecordId)>,
    /// Each relevant live leg, in leg order.
    rows: Vec<serde_json::Value>,
    /// The account's catalog terms with a blank display name.
    catalog: SetAccountCatalogV1,
    coverage: Option<(RecordId, CoverageV1)>,
}

impl ActivityInputs {
    pub(crate) fn read(
        state: &DomainState,
        scope: &ActivityScope<'_>,
    ) -> Result<Self, EngineError> {
        let mut observations = Vec::new();
        for id in scope.evidence {
            if let Some(AuthoritativeRecordV1::ObservationVersion(_)) = state.records.get(id) {
                observations.push((*id, current_version(state, *id)?));
            }
        }
        observations.sort();
        observations.dedup();
        let rows = live_legs(state)?
            .into_iter()
            .filter(|leg| scope.relevant(leg))
            .map(row)
            .collect::<Result<_, _>>()?;
        let mut catalog = catalog(state, scope.account)?;
        catalog.display_name = String::new();
        Ok(Self {
            observations,
            rows,
            catalog,
            coverage: coverage(state, scope).map(|(id, c)| (id, c.clone())),
        })
    }

    pub(crate) fn digest(
        &self,
        scope: &ActivityScope<'_>,
        comparison_version: u32,
    ) -> Result<Digest, EngineError> {
        let inputs = json!({
            "comparison_version": comparison_version,
            "account": scope.account,
            "cutoffs": DateScope {
                start: Some(scope.start),
                end: Some(scope.end),
                basis: scope.basis,
            },
            "observations": self.observations,
            "rows": self.rows,
            "catalog": self.catalog,
            "coverage": self.coverage,
        });
        let bytes = serde_json::to_vec(&inputs).map_err(|_| EngineError::invalid())?;
        let mut hash = Sha256::new();
        hash.update(b"currents/reconciliation-dependencies/activity/v1\0");
        hash.update(&bytes);
        Ok(Digest(hash.finalize().into()))
    }

    /// The inputs that differ from the ones `claim` compared, when its
    /// recorded digest is not the digest of these inputs. The claim records
    /// the legs it compared and whether its interval was complete, and it
    /// read each named observation at the named version. A named receipt
    /// never changes, so when the interval's completeness is unchanged, a
    /// remaining difference is in the account's catalog terms.
    pub(crate) fn changes(
        &self,
        state: &DomainState,
        scope: &ActivityScope<'_>,
        claim: &ReconciliationClaimV1,
    ) -> Result<Vec<InvalidationReasonV1>, EngineError> {
        let mut reasons = Vec::new();
        if self.digest(scope, claim.comparison_version)? == claim.dependency_digest {
            return Ok(reasons);
        }
        let mut compared = self.clone();
        compared.rows = Vec::new();
        for id in &claim.accounting_dependencies {
            if let Some(AuthoritativeRecordV1::FinancialLeg(leg)) = state.records.get(id) {
                compared.rows.push(row(leg)?);
            }
        }
        for (named, current) in &mut compared.observations {
            *current = *named;
        }
        if compared.rows != self.rows {
            reasons.push(InvalidationReasonV1::ScopeActivity);
        }
        if compared.observations != self.observations {
            reasons.push(InvalidationReasonV1::EvidenceRevision);
        }
        let complete = !claim.reasons.contains(&ClaimReasonV1::IncompleteInterval);
        if complete != self.coverage.is_some() {
            reasons.push(InvalidationReasonV1::Coverage);
        } else if compared.digest(scope, claim.comparison_version)? != claim.dependency_digest {
            reasons.push(InvalidationReasonV1::AccountMapping);
        }
        Ok(reasons)
    }
}

/// One recognized leg, without posting or recognition IDs.
fn row(leg: &FinancialLegV1) -> Result<serde_json::Value, EngineError> {
    let mut allocations = leg
        .allocations
        .iter()
        .map(|a| {
            Ok(EvidenceAllocationV1 {
                observation: a.observation,
                facet: a.facet,
                amount: a.amount.normalized()?,
            })
        })
        .collect::<Result<Vec<_>, EngineError>>()?;
    allocations.sort_by_key(|a| (a.observation, a.facet, a.amount.coefficient, a.amount.scale));
    Ok(json!({
        "leg": leg.id,
        "account": leg.account_id,
        "unit": leg.movement.unit_id,
        "effective": leg.effective,
        "posting": leg.posting,
        "amount": leg.movement.normalized()?,
        "allocations": allocations,
    }))
}
