//! Reconciliation claim evaluation, and the bounded comparison of
//! caller-selected Activity lines it uses.
use crate::activity_predicate::ActivityPredicateV1;
use crate::catalog::incomplete;
use crate::dependencies::{self, ActivityScope};
use currents_types::{
    AccountId, AccountScope, AccountingIntent, Amount, AuthoritativeRecordV1, ClaimDispositionV1,
    ClaimReasonV1, Date, DateBasis, DecisionKind, DecisionPlan, Digest, DomainCommitV1,
    DomainState, EngineError, EvidenceAllocationV1, MovementFacetV1, NoEffectV1, ObservationKind,
    ReconciliationClaimV1, ReconciliationDispositionV1, ReconciliationKindV1,
    ReconciliationRequestV1, ReconciliationStatus, RecordId, SourceValue, TemporalValue,
};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// One source line: a facet of one observation version.
pub type EvidenceLineIdV1 = currents_types::EvidenceLineV1;

#[derive(Clone, Debug)]
pub struct ActivityLineV1 {
    pub identity: EvidenceLineIdV1,
    pub account: AccountId,
    pub amount: Amount,
    pub effective: TemporalValue,
    pub posting: TemporalValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityDifferenceV1 {
    MissingAllocation(EvidenceLineIdV1),
    UnexpectedAllocation(EvidenceLineIdV1),
    UnsupportedMultiplicity(EvidenceLineIdV1),
    DifferentTerms(EvidenceLineIdV1),
}

type Terms = (AccountId, Date, Amount);

#[derive(Default)]
struct LineGroup {
    in_scope: Option<Terms>,
    seen: bool,
    repeated: bool,
}

fn group_lines(
    predicate: &ActivityPredicateV1<'_>,
    rows: &[ActivityLineV1],
) -> Result<BTreeMap<EvidenceLineIdV1, LineGroup>, EngineError> {
    let mut groups = BTreeMap::<_, LineGroup>::new();
    for row in rows {
        let group = groups.entry(row.identity).or_default();
        group.repeated |= group.seen;
        group.seen = true;
        if let Some(date) =
            predicate.posting_date_in_scope(row.account, &row.effective, &row.posting)?
        {
            group.in_scope = Some((row.account, date, row.amount.normalized()?));
        }
    }
    Ok(groups)
}

/// Compare exact source identities and signed terms for a bounded one-to-one
/// candidate. Repeat identities touching the comparison (including split
/// allocations straddling its bounds) are unsupported, never netted. Callers
/// must include all sibling rows for relevant identities; do not filter these
/// rows by date or account before comparison.
///
/// An empty difference list is not a reconciliation claim: independent source
/// completeness, active recognized-leg selection, reference qualification,
/// exact retained basis, and independent boundary agreement are all outside
/// this pure function. Neither input is authenticated by the comparison.
pub fn compare_activity_allocations(
    accounts: &[AccountId],
    basis: DateBasis,
    start: Date,
    end: Date,
    sources: &[ActivityLineV1],
    allocations: &[ActivityLineV1],
) -> Result<Vec<ActivityDifferenceV1>, EngineError> {
    use ActivityDifferenceV1::*;
    let predicate = ActivityPredicateV1::new(accounts, basis, start, end)?;
    let sources = group_lines(&predicate, sources)?;
    let allocations = group_lines(&predicate, allocations)?;
    let mut differences = Vec::new();
    for id in sources
        .keys()
        .chain(allocations.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let source = sources.get(&id);
        let allocation = allocations.get(&id);
        let expected = source.and_then(|g| g.in_scope);
        let actual = allocation.and_then(|g| g.in_scope);
        if expected.is_none() && actual.is_none() {
            continue;
        }
        if source.is_some_and(|g| g.repeated) || allocation.is_some_and(|g| g.repeated) {
            differences.push(UnsupportedMultiplicity(id));
            continue;
        }
        match (expected, actual) {
            (Some(_), None) => differences.push(MissingAllocation(id)),
            (None, Some(_)) => differences.push(UnexpectedAllocation(id)),
            (Some(expected), Some(actual)) if expected != actual => {
                differences.push(DifferentTerms(id))
            }
            _ => {}
        }
    }
    Ok(differences)
}

/// The version of the Activity comparison a claim records.
pub const ACTIVITY_COMPARISON_VERSION: u32 = 1;

pub struct ReconciliationEngine;

impl ReconciliationEngine {
    /// Evaluate `request` at `state` into a claim that is `Proposed` or
    /// `PartiallySupported`. The request's basis must name the state's
    /// commit. Only Activity claims are evaluated so far.
    pub fn evaluate(
        state: &DomainState,
        request: &ReconciliationRequestV1,
    ) -> Result<ReconciliationClaimV1, EngineError> {
        if request.basis.through_commit != state.through {
            return Err(dependencies::stale());
        }
        match request.kind {
            ReconciliationKindV1::Activity => activity(state, request),
            ReconciliationKindV1::Balance
            | ReconciliationKindV1::Relationship
            | ReconciliationKindV1::Holdings => Err(EngineError::version()),
        }
    }

    /// Evaluate `request` at `state` and record the claim, with no
    /// accounting effect.
    pub fn record(
        state: &DomainState,
        request: &ReconciliationRequestV1,
    ) -> Result<DecisionPlan, EngineError> {
        let claim = Self::evaluate(state, request)?;
        Ok(no_effect(
            AuthoritativeRecordV1::ReconciliationClaim(claim),
            "Record a reconciliation claim",
        ))
    }

    /// Confirm a `Proposed` claim for its scope. `digest` must be the digest
    /// the claim recorded, and the digest computed at `state` must still
    /// equal it.
    pub fn confirm(
        state: &DomainState,
        claim: RecordId,
        digest: Digest,
    ) -> Result<DecisionPlan, EngineError> {
        let recorded = Self::claim(state, claim)?;
        if Self::lifecycle(state, claim)? != ReconciliationStatus::Proposed {
            return Err(EngineError::invalid());
        }
        if digest != recorded.dependency_digest || current_digest(state, recorded)? != digest {
            return Err(dependencies::stale());
        }
        disposition(
            claim,
            digest,
            ClaimDispositionV1::ConfirmedForScope,
            "Confirmed for the claim's scope at its dependency digest",
        )
    }

    /// Dispute a `Proposed` or `PartiallySupported` claim. `digest` must be
    /// the digest the claim recorded.
    pub fn dispute(
        state: &DomainState,
        claim: RecordId,
        digest: Digest,
        reason: &str,
    ) -> Result<DecisionPlan, EngineError> {
        let recorded = Self::claim(state, claim)?;
        if !matches!(
            Self::lifecycle(state, claim)?,
            ReconciliationStatus::Proposed | ReconciliationStatus::PartiallySupported
        ) || reason.trim().is_empty()
        {
            return Err(EngineError::invalid());
        }
        if digest != recorded.dependency_digest {
            return Err(dependencies::stale());
        }
        disposition(claim, digest, ClaimDispositionV1::Disputed, reason)
    }

    /// The claim's lifecycle status at `state`. A newer claim with the same
    /// kind, accounts, unit and interval makes it `Superseded`. Otherwise an
    /// owner's confirmation or dispute decides it, and before either the
    /// claim's own recorded status does. A claim's commit follows its basis
    /// commit, so the claim with the later basis is the newer one.
    pub fn lifecycle(
        state: &DomainState,
        claim: RecordId,
    ) -> Result<ReconciliationStatus, EngineError> {
        let recorded = Self::claim(state, claim)?;
        let mut decided = None;
        for record in state.records.values() {
            match record {
                AuthoritativeRecordV1::ReconciliationClaim(other)
                    if other.id != recorded.id
                        && other.kind == recorded.kind
                        && other.scope == recorded.scope
                        && other.unit == recorded.unit
                        && other.cutoffs == recorded.cutoffs
                        && other.basis.through_commit > recorded.basis.through_commit =>
                {
                    return Ok(ReconciliationStatus::Superseded);
                }
                AuthoritativeRecordV1::ReconciliationDisposition(d) if d.claim == claim => {
                    if decided.is_some() {
                        return Err(EngineError::invalid());
                    }
                    decided = Some(match d.disposition {
                        ClaimDispositionV1::ConfirmedForScope => {
                            ReconciliationStatus::ConfirmedForScope
                        }
                        ClaimDispositionV1::Disputed => ReconciliationStatus::Disputed,
                    });
                }
                _ => {}
            }
        }
        Ok(decided.unwrap_or(recorded.status))
    }

    fn claim(state: &DomainState, id: RecordId) -> Result<&ReconciliationClaimV1, EngineError> {
        match state.records.get(&id) {
            Some(AuthoritativeRecordV1::ReconciliationClaim(claim)) if claim.id == id => Ok(claim),
            _ => Err(EngineError::invalid()),
        }
    }
}

/// C06 replay of a claim, confirmation or dispute commit: the plan derived
/// again from the prior history must be the commit's whole record set, with
/// one `Reconciliation` decision and no invalidations.
pub(crate) fn readmit(prior: &DomainState, commit: &DomainCommitV1) -> Result<(), EngineError> {
    let Some((AuthoritativeRecordV1::FrozenAccountingManifest(_), held)) =
        commit.records.split_last()
    else {
        return Err(EngineError::invalid());
    };
    let plan = match held {
        [AuthoritativeRecordV1::ReconciliationClaim(claim)] => {
            ReconciliationEngine::record(prior, &request_of(claim))?
        }
        [AuthoritativeRecordV1::ReconciliationDisposition(d)] => match d.disposition {
            ClaimDispositionV1::ConfirmedForScope => {
                ReconciliationEngine::confirm(prior, d.claim, d.dependency_digest)?
            }
            ClaimDispositionV1::Disputed => {
                ReconciliationEngine::dispute(prior, d.claim, d.dependency_digest, &d.reason)?
            }
        },
        _ => return Err(EngineError::invalid()),
    };
    let [decision] = commit.decisions.as_slice() else {
        return Err(EngineError::invalid());
    };
    if plan.records != held
        || decision.decision_type != DecisionKind::Reconciliation
        || !commit.command_result.effects.invalidations.is_empty()
    {
        return Err(EngineError::invalid());
    }
    Ok(())
}

fn no_effect(record: AuthoritativeRecordV1, reason: &str) -> DecisionPlan {
    DecisionPlan {
        records: vec![record],
        invalidations: vec![],
        accounting: AccountingIntent::NoEffect(NoEffectV1 {
            reason: reason.into(),
        }),
    }
}

fn disposition(
    claim: RecordId,
    digest: Digest,
    disposition: ClaimDispositionV1,
    reason: &str,
) -> Result<DecisionPlan, EngineError> {
    // A claim takes at most one confirmation or dispute.
    let bytes = serde_json::to_vec(&(claim, disposition)).map_err(|_| EngineError::invalid())?;
    let record = ReconciliationDispositionV1 {
        id: derived_id(b"currents/reconciliation-disposition/v1\0", &bytes),
        claim,
        dependency_digest: digest,
        disposition,
        reason: reason.into(),
    };
    Ok(no_effect(
        AuthoritativeRecordV1::ReconciliationDisposition(record),
        "Decide a reconciliation claim",
    ))
}

/// The dependency digest of `claim`'s scope at `state`.
fn current_digest(
    state: &DomainState,
    claim: &ReconciliationClaimV1,
) -> Result<Digest, EngineError> {
    match claim.kind {
        ReconciliationKindV1::Activity
            if claim.comparison_version == ACTIVITY_COMPARISON_VERSION =>
        {
            let scope =
                ActivityScope::new(&claim.scope, &claim.cutoffs, &claim.evidence_dependencies)?;
            dependencies::activity_digest(state, &scope, claim.comparison_version)
        }
        _ => Err(EngineError::version()),
    }
}

/// A named statement: its movement lines and its reported balances.
struct Statement<'a> {
    lines: Vec<(EvidenceLineIdV1, Amount, &'a TemporalValue)>,
    balances: Vec<(Amount, &'a TemporalValue)>,
}

impl<'a> Statement<'a> {
    fn read(state: &'a DomainState, evidence: &[RecordId]) -> Result<Self, EngineError> {
        let mut statement = Self {
            lines: Vec::new(),
            balances: Vec::new(),
        };
        for id in evidence {
            match state.records.get(id) {
                Some(AuthoritativeRecordV1::ImportReceipt(_)) => {}
                Some(AuthoritativeRecordV1::ObservationVersion(_)) => {
                    for facet in facets(state, *id)? {
                        let SourceValue::Known(amount) = facet.amount else {
                            return Err(incomplete());
                        };
                        let line = EvidenceLineIdV1 {
                            observation: *id,
                            facet: facet.lineage_key,
                        };
                        match facet.kind {
                            ObservationKind::Movement => {
                                statement.lines.push((line, amount, &facet.temporal))
                            }
                            ObservationKind::Balance => {
                                statement.balances.push((amount, &facet.temporal))
                            }
                            _ => return Err(EngineError::invalid()),
                        }
                    }
                }
                _ => return Err(EngineError::invalid()),
            }
        }
        Ok(statement)
    }

    /// The one balance reported at the end of `day`, if the statement
    /// reports exactly one amount there.
    fn balance(&self, day: Option<Date>) -> Result<Option<Amount>, EngineError> {
        let mut found: Option<Amount> = None;
        for (amount, temporal) in &self.balances {
            let TemporalValue::Date { civil, .. } = temporal else {
                continue;
            };
            if Some(*civil) != day {
                continue;
            }
            let amount = amount.normalized()?;
            if found.is_some_and(|old| old != amount) {
                return Ok(None);
            }
            found = Some(amount);
        }
        Ok(found)
    }

    fn unmapped(&self) -> bool {
        let civil = |t: &TemporalValue| matches!(t, TemporalValue::Date { .. });
        self.lines.iter().any(|(_, _, t)| !civil(t)) || self.balances.iter().any(|(_, t)| !civil(t))
    }
}

/// Activity coverage: every source line in a complete interval is allocated
/// exactly once to a recognized leg, every recognized leg in scope is
/// supported or excluded, and the source opening plus its lines in the
/// interval equals its closing.
fn activity(
    state: &DomainState,
    request: &ReconciliationRequestV1,
) -> Result<ReconciliationClaimV1, EngineError> {
    let evidence = sorted(&request.evidence);
    let exclusions = sorted(&request.exclusions);
    let scope = ActivityScope::new(&request.scope.accounts, &request.cutoffs, &evidence)?;
    let unit = dependencies::catalog(state, scope.account)?.unit_id;
    let legs = dependencies::live_legs(state)?;
    let statement = Statement::read(state, &evidence)?;
    for id in &exclusions {
        let names_line = statement.lines.iter().any(|(l, ..)| l.observation == *id);
        let names_leg = legs.iter().any(|live| RecordId(live.leg.id.0) == *id);
        if !names_line && !names_leg {
            return Err(EngineError::invalid());
        }
    }
    let excluded = |id: RecordId| exclusions.binary_search(&id).is_ok();

    // Every source line with a civil date, and the source's movement in the
    // interval, exclusions included.
    let mut sources = Vec::new();
    let mut movement = Amount::new(0, 0, unit)?;
    for (line, amount, temporal) in &statement.lines {
        let TemporalValue::Date { civil, .. } = temporal else {
            continue;
        };
        if scope.contains(*civil) {
            movement = movement.checked_add(amount)?;
        }
        if !excluded(line.observation) {
            sources.push(ActivityLineV1 {
                identity: *line,
                account: scope.account,
                amount: *amount,
                effective: (*temporal).clone(),
                posting: (*temporal).clone(),
            });
        }
    }
    // Every allocation of a live recognized leg. The comparison selects the
    // ones in scope; it needs the others to find repeated identities.
    let mut allocations = Vec::new();
    let mut accounting = BTreeSet::new();
    let mut unmapped = statement.unmapped();
    let mut unsupported = Vec::new();
    for live in &legs {
        let leg = live.leg;
        if scope.relevant(leg) {
            accounting.insert(RecordId(leg.id.0));
            accounting.extend(live.recognitions.iter().map(|r| RecordId(r.0)));
        }
        if excluded(RecordId(leg.id.0)) {
            continue;
        }
        if scope.unmapped(leg) {
            unmapped = true;
            continue;
        }
        // A leg without allocations, such as one an association admitted,
        // gives the comparison no row, so it is reported here.
        if leg.allocations.is_empty() && scope.inside(leg) {
            unsupported.push(leg.id);
        }
        for allocation in leg.allocations.iter() {
            if excluded(allocation.observation) {
                continue;
            }
            allocations.push(ActivityLineV1 {
                identity: EvidenceLineIdV1 {
                    observation: allocation.observation,
                    facet: allocation.facet,
                },
                account: leg.account_id,
                amount: allocation.amount,
                effective: leg.effective.clone(),
                posting: leg.posting.clone(),
            });
        }
    }

    let mut reasons = Vec::new();
    let mut differences = Vec::new();
    if dependencies::coverage(state, &scope).is_none() {
        reasons.push(ClaimReasonV1::IncompleteInterval);
    }
    // A reported balance holds at the end of its day: the opening is the
    // day before the interval and the closing its last day.
    let opening = statement.balance(scope.start.previous_day())?;
    let closing = statement.balance(Some(scope.end))?;
    if opening.is_none() {
        reasons.push(ClaimReasonV1::UnknownOpening);
    }
    if closing.is_none() {
        reasons.push(ClaimReasonV1::UnknownClosing);
    }
    if let (Some(opening), Some(closing)) = (opening, closing) {
        let residual = closing.checked_add(&opening.checked_add(&movement)?.checked_neg()?)?;
        if residual.coefficient != 0 {
            reasons.push(ClaimReasonV1::UnexplainedResidual);
            differences.push(residual.normalized()?);
        }
    }
    if unmapped {
        reasons.push(ClaimReasonV1::MissingDateMapping);
    }
    let mut unmatched = BTreeSet::new();
    for difference in compare_activity_allocations(
        &[scope.account],
        scope.basis,
        scope.start,
        scope.end,
        &sources,
        &allocations,
    )? {
        use ActivityDifferenceV1 as D;
        let (line, reason) = match difference {
            D::MissingAllocation(l) => (l, ClaimReasonV1::MissingAllocation(l)),
            D::UnexpectedAllocation(l) => (l, ClaimReasonV1::UnexpectedAllocation(l)),
            D::UnsupportedMultiplicity(l) => (l, ClaimReasonV1::RepeatedIdentity(l)),
            D::DifferentTerms(l) => (l, ClaimReasonV1::DifferentTerms(l)),
        };
        unmatched.insert(line);
        reasons.push(reason);
    }
    reasons.extend(unsupported.into_iter().map(ClaimReasonV1::UnsupportedLeg));
    let matched_allocations = sources
        .iter()
        .filter(|s| {
            !unmatched.contains(&s.identity)
                && matches!(&s.posting, TemporalValue::Date { civil, .. } if scope.contains(*civil))
        })
        .map(|s| EvidenceAllocationV1 {
            observation: s.identity.observation,
            facet: s.identity.facet,
            amount: s.amount,
        })
        .collect();

    let (status, reason) = if reasons.is_empty() {
        (
            ReconciliationStatus::Proposed,
            "Every source line in the interval is allocated once, and the source balances.",
        )
    } else {
        (
            ReconciliationStatus::PartiallySupported,
            "Activity coverage is partial for the listed reasons.",
        )
    };
    let mut claim = ReconciliationClaimV1 {
        id: RecordId(uuid::Uuid::nil()),
        kind: ReconciliationKindV1::Activity,
        comparison_version: ACTIVITY_COMPARISON_VERSION,
        basis: request.basis.clone(),
        scope: vec![scope.account],
        unit,
        cutoffs: request.cutoffs.clone(),
        dependency_digest: dependencies::activity_digest(
            state,
            &scope,
            ACTIVITY_COMPARISON_VERSION,
        )?,
        evidence_dependencies: evidence,
        decision_dependencies: vec![],
        accounting_dependencies: accounting.into_iter().collect(),
        matched_allocations,
        differences,
        exclusions,
        status,
        reasons,
        reason: reason.into(),
    };
    claim.id = claim_id(&claim)?;
    Ok(claim)
}

fn sorted(ids: &[RecordId]) -> Vec<RecordId> {
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    ids
}

/// The facets that parse products report for `observation`, one per lineage
/// key. Products that disagree about one facet are refused.
fn facets(
    state: &DomainState,
    observation: RecordId,
) -> Result<Vec<&MovementFacetV1>, EngineError> {
    let mut facets = BTreeMap::<RecordId, &MovementFacetV1>::new();
    for record in state.records.values() {
        let product = match record {
            AuthoritativeRecordV1::AddressedParseProduct(p)
                if p.product.source_version_id == observation =>
            {
                p.validate()?;
                &p.product
            }
            AuthoritativeRecordV1::ParseProduct(p) if p.source_version_id == observation => p,
            _ => continue,
        };
        for facet in &product.facets {
            if facets
                .insert(facet.lineage_key, facet)
                .is_some_and(|old| old != facet)
            {
                return Err(EngineError::invalid());
            }
        }
    }
    if facets.is_empty() {
        return Err(incomplete());
    }
    Ok(facets.into_values().collect())
}

/// The request a claim records: its kind, accounts, `cutoffs`, sorted
/// evidence and exclusions, and basis. Replay evaluates it again.
fn request_of(claim: &ReconciliationClaimV1) -> ReconciliationRequestV1 {
    ReconciliationRequestV1 {
        kind: claim.kind,
        scope: AccountScope {
            accounts: claim.scope.clone(),
            components: vec![],
        },
        cutoffs: claim.cutoffs.clone(),
        evidence: claim.evidence_dependencies.clone(),
        exclusions: claim.exclusions.clone(),
        basis: claim.basis.clone(),
    }
}

/// A claim's ID derives from the request it records, so the same request at
/// the same basis yields the same claim, and replay derives the ID again.
fn claim_id(claim: &ReconciliationClaimV1) -> Result<RecordId, EngineError> {
    let bytes = serde_json::to_vec(&request_of(claim)).map_err(|_| EngineError::invalid())?;
    Ok(derived_id(b"currents/reconciliation-claim/v1\0", &bytes))
}

/// An RFC 9562 version 8 (custom) UUID from the SHA-256 of `tag` and `bytes`.
fn derived_id(tag: &[u8], bytes: &[u8]) -> RecordId {
    let mut hash = Sha256::new();
    hash.update(tag);
    hash.update(bytes);
    let digest: [u8; 32] = hash.finalize().into();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    RecordId(uuid::Builder::from_custom_bytes(bytes).into_uuid())
}
