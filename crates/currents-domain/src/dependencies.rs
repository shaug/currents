//! What a reconciliation claim read from the state, and the digest of it.
//!
//! The digest covers the current version of each named observation, each
//! recognized leg the claim compared, the scope account's catalog terms
//! except its display name, the coverage report an Activity claim used, the
//! account's opening anchor where it decides the claim, and the comparison
//! version. Legs are keyed by their stable identity, so an identity-only
//! correction, which replaces recognition and posting IDs and keeps the legs,
//! leaves the digest unchanged. A claim whose recorded digest differs from
//! the digest at a later state no longer applies there.
use crate::activity_predicate::civil_date;
use crate::catalog::{CatalogEngine, incomplete};
use crate::reconciliation::InvalidationReasonV1;
use currents_types::*;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The scope of a claim: one account, its dates by one basis, and the named
/// evidence. An Activity claim's interval has a start. A Balance claim's
/// interval has none, because the balance at its boundary, the interval's
/// end, depends on every date through it.
#[derive(Clone, Copy)]
pub(crate) struct ClaimScope<'a> {
    pub(crate) kind: ReconciliationKindV1,
    pub(crate) account: AccountId,
    pub(crate) start: Option<Date>,
    pub(crate) end: Date,
    pub(crate) basis: DateBasis,
    pub(crate) evidence: &'a [RecordId],
}

impl<'a> ClaimScope<'a> {
    /// The scope of a `kind` claim on `accounts` over `cutoffs`. It must name
    /// one account and an end. An Activity interval also needs a start no
    /// later than its end; a Balance interval must have no start.
    pub(crate) fn new(
        kind: ReconciliationKindV1,
        accounts: &[AccountId],
        cutoffs: &DateScope,
        evidence: &'a [RecordId],
    ) -> Result<Self, EngineError> {
        let [account] = accounts else {
            return Err(EngineError::version());
        };
        let start = match (kind, cutoffs.start, cutoffs.end) {
            (ReconciliationKindV1::Activity, Some(start), Some(end)) if start <= end => Some(start),
            (ReconciliationKindV1::Balance, None, Some(_)) => None,
            (ReconciliationKindV1::Activity | ReconciliationKindV1::Balance, ..) => {
                return Err(EngineError::invalid());
            }
            _ => return Err(EngineError::version()),
        };
        Ok(Self {
            kind,
            account: *account,
            start,
            end: cutoffs.end.ok_or_else(EngineError::invalid)?,
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
        self.start.is_none_or(|start| start <= day) && day <= self.end
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

    /// The anchor decides the claim. A Balance claim always compares with
    /// it. An Activity interval that starts on or before its cutoff reaches
    /// into history the journal does not hold.
    fn decided_by(&self, anchor: &Anchor) -> bool {
        self.start.is_none_or(|start| start <= anchor.cutoff)
    }
}

/// An account's opening anchor (C08): its segment, the opening's cutoff and
/// the opening terms, including the balance evidence the opening adopted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub(crate) segment: RecordId,
    pub(crate) cutoff: Date,
    pub(crate) opening: OpeningBasisV1,
}

impl Anchor {
    /// The anchor of the account's active segment, if it has one.
    pub(crate) fn active(
        state: &DomainState,
        account: AccountId,
    ) -> Result<Option<Self>, EngineError> {
        let component = catalog(state, account)?.component_id;
        crate::segments::active(state, component)?
            .map(|segment| Self::of(state, segment))
            .transpose()
    }

    /// The anchor of the segment `claim` names, if it names one.
    fn named(
        state: &DomainState,
        claim: &ReconciliationClaimV1,
    ) -> Result<Option<Self>, EngineError> {
        claim
            .accounting_dependencies
            .iter()
            .find_map(|id| match state.records.get(id) {
                Some(AuthoritativeRecordV1::AccountingSegment(segment)) => Some(segment),
                _ => None,
            })
            .map(|segment| Self::of(state, segment))
            .transpose()
    }

    fn of(state: &DomainState, segment: &AccountingSegmentV1) -> Result<Self, EngineError> {
        let opening = state
            .records
            .values()
            .find_map(|record| match record {
                AuthoritativeRecordV1::ActivityAdoption(a)
                    if a.recognition == Some(segment.opening) =>
                {
                    a.opening.as_ref()
                }
                _ => None,
            })
            .ok_or_else(incomplete)?;
        let TemporalValue::Date { civil, .. } = opening.cutoff else {
            return Err(incomplete());
        };
        Ok(Self {
            segment: segment.id,
            cutoff: civil,
            opening: opening.clone(),
        })
    }

    /// `line` is the balance evidence the opening adopted, or a later
    /// revision of it.
    pub(crate) fn adopted(
        &self,
        state: &DomainState,
        line: EvidenceLineV1,
    ) -> Result<bool, EngineError> {
        let evidence = &self.opening.evidence;
        Ok(line.facet == evidence.facet
            && crate::recognition::revises(state, line.observation, evidence.observation)?)
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
    scope: &ClaimScope<'_>,
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
                    && c.scope
                        .start
                        .is_none_or(|start| scope.start.is_some_and(|s| start <= s))
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

/// What a claim's dependency digest covers, read at one state.
#[derive(Clone)]
pub(crate) struct ClaimInputs {
    /// Each named observation and its latest explicit revision.
    observations: Vec<(RecordId, RecordId)>,
    /// Each relevant live leg, in leg order.
    rows: Vec<serde_json::Value>,
    /// The account's catalog terms with a blank display name.
    catalog: SetAccountCatalogV1,
    /// The coverage an Activity claim used.
    coverage: Option<(RecordId, CoverageV1)>,
    /// The account's active anchor, when it decides the claim.
    pub(crate) anchor: Option<Anchor>,
}

impl ClaimInputs {
    pub(crate) fn read(state: &DomainState, scope: &ClaimScope<'_>) -> Result<Self, EngineError> {
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
        let coverage = match scope.kind {
            ReconciliationKindV1::Activity => coverage(state, scope).map(|(id, c)| (id, c.clone())),
            _ => None,
        };
        Ok(Self {
            observations,
            rows,
            catalog,
            coverage,
            anchor: Anchor::active(state, scope.account)?.filter(|a| scope.decided_by(a)),
        })
    }

    pub(crate) fn digest(
        &self,
        scope: &ClaimScope<'_>,
        comparison_version: u32,
    ) -> Result<Digest, EngineError> {
        let mut inputs = json!({
            "comparison_version": comparison_version,
            "account": scope.account,
            "cutoffs": DateScope {
                start: scope.start,
                end: Some(scope.end),
                basis: scope.basis,
            },
            "observations": self.observations,
            "rows": self.rows,
            "catalog": self.catalog,
            "coverage": self.coverage,
        });
        // An Activity claim that no anchor decides keeps the digest it had
        // before anchors entered it.
        if let Some(anchor) = &self.anchor {
            inputs["anchor"] = json!({"segment": anchor.segment, "opening": anchor.opening});
        }
        let bytes = serde_json::to_vec(&inputs).map_err(|_| EngineError::invalid())?;
        let mut hash = Sha256::new();
        hash.update(match scope.kind {
            ReconciliationKindV1::Activity => {
                &b"currents/reconciliation-dependencies/activity/v1\0"[..]
            }
            _ => &b"currents/reconciliation-dependencies/balance/v1\0"[..],
        });
        hash.update(&bytes);
        Ok(Digest(hash.finalize().into()))
    }

    /// The inputs that differ from the ones `claim` compared, when its
    /// recorded digest is not the digest of these inputs. The claim records
    /// the legs it compared, the anchor's segment where an anchor decided it,
    /// and whether its interval was complete, and evaluation accepts only
    /// named observations without a later revision. A named receipt never
    /// changes, so when the interval's completeness is unchanged, a remaining
    /// difference is in the account's catalog terms.
    pub(crate) fn changes(
        &self,
        state: &DomainState,
        scope: &ClaimScope<'_>,
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
        compared.anchor = Anchor::named(state, claim)?;
        if compared.rows != self.rows {
            reasons.push(InvalidationReasonV1::ScopeActivity);
        }
        if compared.observations != self.observations {
            reasons.push(InvalidationReasonV1::EvidenceRevision);
        }
        if compared.anchor != self.anchor {
            reasons.push(InvalidationReasonV1::Anchor);
        }
        let complete = !claim.reasons.contains(&ClaimReasonV1::IncompleteInterval);
        if scope.kind == ReconciliationKindV1::Activity && complete != self.coverage.is_some() {
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
