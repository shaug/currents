//! What a reconciliation claim read from the state, and the digest of it.
//!
//! The digest covers the current version of each named observation, each
//! recognized leg the claim compared, each scope account's catalog terms
//! except its display name, the coverage report the claim used and the
//! comparison version. Legs are keyed by their stable identity, so an
//! identity-only correction, which replaces recognition and posting IDs and
//! keeps the legs, leaves the digest unchanged.
use crate::catalog::{CatalogEngine, incomplete};
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
        let day = match self.basis {
            DateBasis::Effective => &leg.effective,
            DateBasis::Posting => &leg.posting,
        };
        match day {
            TemporalValue::Date { civil, .. } => Some(*civil),
            _ => None,
        }
    }

    /// The leg affects the claim: it is on the scope account inside the
    /// interval, or on it with a date that is not a civil date, or it
    /// allocates a named observation.
    pub(crate) fn relevant(&self, leg: &FinancialLegV1) -> bool {
        let on_account = leg.account_id == self.account
            && self
                .day(leg)
                .is_none_or(|day| self.start <= day && day <= self.end);
        on_account
            || leg
                .allocations
                .iter()
                .any(|a| self.evidence.contains(&a.observation))
    }

    /// The leg is on the scope account and has no civil date by the basis.
    pub(crate) fn unmapped(&self, leg: &FinancialLegV1) -> bool {
        leg.account_id == self.account && self.day(leg).is_none()
    }
}

/// One live recognized leg and the live Activity recognitions that name it.
pub(crate) struct LiveLeg<'a> {
    pub(crate) leg: &'a FinancialLegV1,
    pub(crate) recognitions: BTreeSet<RecognitionId>,
}

/// Every leg of a live Activity recognition, in leg order. A recognition is
/// live until the accounting index records its reversal. The index must be
/// current with the state.
pub(crate) fn live_legs(state: &DomainState) -> Result<Vec<LiveLeg<'_>>, EngineError> {
    let index = state.accounting.as_ref().ok_or_else(incomplete)?;
    if index.prefix.through != state.through || index.prefix.domain_digest != state.domain_digest {
        return Err(stale());
    }
    let mut legs = BTreeMap::<LegId, LiveLeg<'_>>::new();
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
            legs.entry(*id)
                .or_insert_with(|| LiveLeg {
                    leg,
                    recognitions: BTreeSet::new(),
                })
                .recognitions
                .insert(recognition.id);
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

/// The dependency digest of an Activity claim over `scope` at `state`.
pub(crate) fn activity_digest(
    state: &DomainState,
    scope: &ActivityScope<'_>,
    comparison_version: u32,
) -> Result<Digest, EngineError> {
    let mut observations = Vec::new();
    for id in scope.evidence {
        if let Some(AuthoritativeRecordV1::ObservationVersion(_)) = state.records.get(id) {
            observations.push((*id, current_version(state, *id)?));
        }
    }
    observations.sort();
    observations.dedup();
    let mut rows = Vec::new();
    for live in live_legs(state)? {
        let leg = live.leg;
        if !scope.relevant(leg) {
            continue;
        }
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
        // One recognized leg, without posting or recognition IDs.
        rows.push(json!({
            "leg": leg.id,
            "account": leg.account_id,
            "unit": leg.movement.unit_id,
            "effective": leg.effective,
            "posting": leg.posting,
            "amount": leg.movement.normalized()?,
            "allocations": allocations,
        }));
    }
    let mut catalog = catalog(state, scope.account)?;
    catalog.display_name = String::new();
    let inputs = json!({
        "comparison_version": comparison_version,
        "account": scope.account,
        "cutoffs": DateScope {
            start: Some(scope.start),
            end: Some(scope.end),
            basis: scope.basis,
        },
        "observations": observations,
        "rows": rows,
        "catalog": catalog,
        "coverage": coverage(state, scope),
    });
    let bytes = serde_json::to_vec(&inputs).map_err(|_| EngineError::invalid())?;
    let mut hash = Sha256::new();
    hash.update(b"currents/reconciliation-dependencies/activity/v1\0");
    hash.update(&bytes);
    Ok(Digest(hash.finalize().into()))
}
