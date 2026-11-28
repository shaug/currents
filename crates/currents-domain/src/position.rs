//! Per-component position selection (C09). For each component in scope, the
//! query lists the source balances its aliases bind, selects the latest
//! compatible one at or before the requested date, and reports the journal's
//! anchored balance or, without an anchor, the known movements. Beside them
//! it labels each selected balance's freshness and lists pending activity,
//! the selected sources' reported coverage and any journal discrepancy. It
//! reads the state and writes nothing. It never rolls a source balance
//! forward. An explanation is the graph of one component's selection inputs
//! at the same basis.
use crate::activity_predicate::civil_date;
use crate::aggregation::{self, Found};
use crate::catalog::{CatalogEngine, incomplete, journal_sign};
use crate::dependencies::{self, Anchor};
use crate::policy::PolicyEngine;
use crate::recognition::RecognitionEngine;
use crate::reconciliation::{ReconciliationEngine, facets};
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

pub struct PositionEngine;

/// The most components one query reports.
pub const MAX_POSITION_COMPONENTS: usize = 256;

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const NANOS_PER_HOUR: i128 = 3_600_000_000_000;
const UNIX_EPOCH_JULIAN_DAY: i32 = 2_440_588;

/// A source as-of boundary. Every date-only value, the requested date
/// included, reads in one unknown timezone between UTC−12 and UTC+14. Two
/// dates therefore compare as civil dates. Against an instant, a date spans
/// from its start at UTC+14 to its end at UTC−12.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Date(Date),
    Instant(i128),
}

impl Boundary {
    fn of(temporal: &TemporalValue) -> Option<Self> {
        match temporal {
            TemporalValue::Date { civil, .. } => Some(Self::Date(*civil)),
            TemporalValue::Instant { utc, .. } => Some(Self::Instant(utc.unix_timestamp_nanos())),
            TemporalValue::Unknown { .. } => None,
        }
    }

    /// The earliest and latest instants it can mean, in nanoseconds since
    /// the Unix epoch.
    fn span(self) -> (i128, i128) {
        match self {
            Self::Date(day) => {
                // A civil date's day number and these offsets are far inside
                // i128, so the saturating forms never saturate.
                let hours = |n: i128| n.saturating_mul(NANOS_PER_HOUR);
                let days = i128::from(day.to_julian_day().saturating_sub(UNIX_EPOCH_JULIAN_DAY));
                let midnight = hours(days.saturating_mul(24));
                (
                    midnight.saturating_sub(hours(14)),
                    midnight.saturating_add(hours(36)),
                )
            }
            Self::Instant(nanos) => (nanos, nanos),
        }
    }

    /// It starts after `other` ends, whatever the timezone.
    fn after(self, other: Self) -> bool {
        match (self, other) {
            (Self::Date(a), Self::Date(b)) => a > b,
            _ => self.span().0 > other.span().1,
        }
    }
}

/// A balance that can be selected for a component.
struct Candidate {
    observation: RecordId,
    source: SourceIdentityV1,
    as_of: TemporalValue,
    boundary: Boundary,
    amount: Amount,
}

/// The candidates no other candidate is strictly later than. When they share
/// one boundary, the highest-ranked source in `priority` among them decides.
/// Equal amounts corroborate and are all selected. Different amounts are a
/// conflict: nothing is selected, and the error holds the candidates that
/// conflict.
type Selection<'a> = Result<Vec<&'a Candidate>, Vec<&'a Candidate>>;

fn select<'a>(candidates: &'a [Candidate], priority: &[RecordId]) -> Selection<'a> {
    let latest: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| !candidates.iter().any(|o| o.boundary.after(c.boundary)))
        .collect();
    let Some(first) = latest.first() else {
        return Ok(Vec::new());
    };
    let mut pool = latest.clone();
    if latest.iter().all(|c| c.boundary == first.boundary)
        && let Some(top) = priority
            .iter()
            .find(|source| latest.iter().any(|c| c.source.connection_id == **source))
    {
        pool.retain(|c| c.source.connection_id == *top);
    }
    if pool.iter().all(|c| c.amount == pool[0].amount) {
        Ok(pool)
    } else {
        Err(pool)
    }
}

/// The requested date of `basis`, once the state can answer at it: the
/// basis names the state's commit, the accounting index is current with the
/// state, and the effective scope ends on a date by effective date. It also
/// returns every live leg.
fn requested<'s>(
    state: &'s DomainState,
    basis: &ReadBasis,
) -> Result<(Date, Vec<&'s FinancialLegV1>), EngineError> {
    if basis.through_commit != state.through {
        return Err(dependencies::stale());
    }
    let legs = dependencies::live_legs(state)?;
    let (Some(day), DateBasis::Effective) =
        (basis.effective_scope.end, basis.effective_scope.basis)
    else {
        return Err(EngineError::invalid());
    };
    Ok((day, legs))
}

/// The governing source priority, or none.
fn priority(policy: &GoverningPolicyV1) -> Vec<RecordId> {
    match &policy.source_priority {
        PolicySettingV1::Set {
            value: SourceValue::Known(order),
            ..
        } => order.clone(),
        _ => Vec::new(),
    }
}

/// The account's active anchor when its cutoff is on or before `day`, with
/// its segment.
fn anchor(
    state: &DomainState,
    account: AccountId,
    day: Date,
) -> Result<Option<(Anchor, &AccountingSegmentV1)>, EngineError> {
    let Some(anchor) = Anchor::active(state, account)?.filter(|a| a.cutoff <= day) else {
        return Ok(None);
    };
    let Some(AuthoritativeRecordV1::AccountingSegment(segment)) =
        state.records.get(&anchor.segment)
    else {
        return Err(EngineError::invalid());
    };
    Ok(Some((anchor, segment)))
}

/// The components a scope names directly or through their accounts, each
/// with its current catalog terms.
fn components(
    state: &DomainState,
    scope: &AccountScope,
) -> Result<BTreeMap<RecordId, SetAccountCatalogV1>, EngineError> {
    let named = scope.components.len().saturating_add(scope.accounts.len());
    if named == 0 || named > MAX_POSITION_COMPONENTS {
        return Err(EngineError::invalid());
    }
    let mut ids: BTreeSet<RecordId> = scope.components.iter().copied().collect();
    for account in &scope.accounts {
        ids.insert(dependencies::catalog(state, *account)?.component_id);
    }
    ids.into_iter()
        .map(|id| Ok((id, CatalogEngine::current(state, id)?.catalog)))
        .collect()
}

/// Every declared alias, with the component that declares it. Catalog
/// revisions only add aliases, so the union is each component's current set.
fn aliases(state: &DomainState) -> Vec<(&SourceAliasV1, RecordId)> {
    let mut found = Vec::new();
    for record in state.records.values() {
        match record {
            AuthoritativeRecordV1::AccountComponent(c) => {
                found.extend(c.aliases.iter().map(|a| (a, c.id)));
            }
            AuthoritativeRecordV1::AccountCatalogRevision(r) => {
                found.extend(r.catalog.aliases.iter().map(|a| (a, r.component_id)));
            }
            _ => {}
        }
    }
    found
}

/// The source balances that bind one component.
#[derive(Default)]
struct Reading {
    candidates: Vec<Candidate>,
    assertions: Vec<PositionAssertionV1>,
}

/// Each component's source balances whose boundary does not start after
/// `requested` ends: every current balance facet of an observation version an
/// alias binds to the component. Each balance that cannot be a candidate adds
/// its code to `codes`.
fn balances(
    state: &DomainState,
    components: &BTreeMap<RecordId, SetAccountCatalogV1>,
    requested: Boundary,
    codes: &mut BTreeSet<&'static str>,
) -> Result<BTreeMap<RecordId, Reading>, EngineError> {
    let aliases = aliases(state);
    let mut readings: BTreeMap<RecordId, Reading> = BTreeMap::new();
    for record in state.records.values() {
        let AuthoritativeRecordV1::ObservationVersion(observation) = record else {
            continue;
        };
        let bound: BTreeSet<RecordId> = aliases
            .iter()
            .filter(|(alias, _)| {
                alias.source == observation.source
                    && observation.original_ids.contains(&alias.original_id)
            })
            .map(|(_, component)| *component)
            .collect();
        if !bound.iter().any(|c| components.contains_key(c))
            || observation.tombstone_of.is_some()
            || dependencies::current_version(state, observation.id)? != observation.id
        {
            continue;
        }
        for (facet, kind) in facets(state, observation.id)? {
            if facet.kind != ObservationKind::Balance {
                continue;
            }
            let boundary = Boundary::of(&facet.temporal);
            if boundary.is_some_and(|b| b.after(requested)) {
                continue;
            }
            let SourceValue::Known(amount) = facet.amount else {
                codes.insert("unknown_amount");
                continue;
            };
            for (component, catalog) in components.iter().filter(|(c, _)| bound.contains(c)) {
                let mut fits = true;
                let mut miss = |code| {
                    codes.insert(code);
                    fits = false;
                };
                if bound.len() > 1 {
                    miss("ambiguous_alias");
                }
                match kind {
                    BalanceKindV1::Posted => {}
                    BalanceKindV1::Unknown => miss("unknown_balance_kind"),
                    BalanceKindV1::Available | BalanceKindV1::Statement => {
                        miss("incompatible_balance_kind")
                    }
                }
                if amount.unit_id != catalog.unit_id {
                    miss("incompatible_unit");
                }
                if boundary.is_none() {
                    miss("unknown_as_of");
                }
                let reading = readings.entry(*component).or_default();
                reading.assertions.push(PositionAssertionV1 {
                    id: observation.id,
                    component: *component,
                    amount,
                    as_of: facet.temporal.clone(),
                    source: observation.source.clone(),
                    coverage: CoverageV1 {
                        scope: DateScope {
                            start: None,
                            end: None,
                            basis: DateBasis::Effective,
                        },
                        status: CoverageStatus::Unknown,
                    },
                });
                if let (true, Some(boundary)) = (fits, boundary) {
                    reading.candidates.push(Candidate {
                        observation: observation.id,
                        source: observation.source.clone(),
                        as_of: facet.temporal.clone(),
                        boundary,
                        amount: amount.normalized()?,
                    });
                }
            }
        }
    }
    Ok(readings)
}

/// The journal side of `component`'s position on `day`. With an active
/// anchor whose cutoff is on or before `day`, the anchored balance in
/// journal sign: the opening, converted by the account's normal sign, plus
/// the live legs on the account after the cutoff through `day`. Without one,
/// it pushes each live leg on the account through `day` to `movements` and
/// adds `unknown_opening`.
fn journal(
    state: &DomainState,
    legs: &[&FinancialLegV1],
    component: RecordId,
    catalog: &SetAccountCatalogV1,
    day: Date,
    codes: &mut BTreeSet<&'static str>,
    movements: &mut Vec<AccountMovementV1>,
) -> Result<Option<AnchoredBalanceV1>, EngineError> {
    let account = catalog.account_id;
    let anchored = anchor(state, account, day)?;
    let mut amount = anchored
        .as_ref()
        .map(|(a, _)| journal_sign(catalog.normal_sign, a.opening.amount))
        .transpose()?;
    let mut dependencies: Vec<RecordId> = anchored.iter().map(|(a, _)| a.segment).collect();
    let mut unmapped = false;
    for leg in legs.iter().filter(|l| l.account_id == account) {
        let Some(leg_day) = civil_date(DateBasis::Effective, &leg.effective, &leg.posting) else {
            unmapped = true;
            continue;
        };
        if leg_day > day {
            continue;
        }
        match &anchored {
            Some((anchor, _)) if leg_day > anchor.cutoff => {
                amount = amount.map(|a| a.checked_add(&leg.movement)).transpose()?;
                dependencies.push(RecordId(leg.id.0));
            }
            Some(_) => {}
            None => movements.push(AccountMovementV1 {
                account,
                component,
                amount: leg.movement,
                effective: leg.effective.clone(),
                posting: leg.posting.clone(),
                evidence: std::iter::once(RecordId(leg.id.0))
                    .chain(leg.allocations.iter().map(|a| a.observation))
                    .collect(),
            }),
        }
    }
    if unmapped {
        codes.insert("missing_date_mapping");
    }
    let Some((_, segment)) = anchored else {
        codes.insert("unknown_opening");
        return Ok(None);
    };
    Ok(Some(AnchoredBalanceV1 {
        component,
        opening: RecordId(segment.opening.0),
        amount: match (amount, unmapped) {
            (Some(amount), false) => SourceValue::Known(amount.normalized()?),
            _ => SourceValue::Unknown,
        },
        through: TemporalValue::Date {
            civil: day,
            original_text: day.to_string(),
        },
        dependencies,
    }))
}

/// The freshness label of a selected balance at `evaluated`, in nanoseconds
/// since the Unix epoch, under the governing `thresholds`. A known threshold
/// for its connection makes it `Fresh` when even the oldest instant its as-of
/// can mean is within the threshold, and `Stale` when even the newest is
/// beyond it; otherwise the timezone decides, and it is `Unknown`.
fn freshness(
    thresholds: &PolicySettingV1<Vec<FreshnessThresholdV1>>,
    candidate: &Candidate,
    evaluated: i128,
) -> FreshnessV1 {
    let (status, seconds) = match thresholds {
        PolicySettingV1::NotSet => (FreshnessStatus::NotAssessed, 0),
        PolicySettingV1::Set {
            value: SourceValue::Known(list),
            ..
        } => match list
            .iter()
            .find(|t| t.source == candidate.source.connection_id)
        {
            None => (FreshnessStatus::NotAssessed, 0),
            Some(threshold) => {
                let limit = i128::from(threshold.seconds.0).saturating_mul(NANOS_PER_SECOND);
                let (oldest, newest) = candidate.boundary.span();
                let status = if evaluated.saturating_sub(oldest) <= limit {
                    FreshnessStatus::Fresh
                } else if evaluated.saturating_sub(newest) > limit {
                    FreshnessStatus::Stale
                } else {
                    FreshnessStatus::Unknown
                };
                (status, threshold.seconds.0)
            }
        },
        PolicySettingV1::Set { .. } => (FreshnessStatus::Unknown, 0),
    };
    FreshnessV1 {
        source: candidate.observation,
        status,
        as_of: candidate.as_of.clone(),
        threshold_seconds: CommitSeq(seconds),
    }
}

/// The unrecorded Balance claim on `account` for a selected balance as of a
/// civil date, when it reports a difference. A balance whose observation
/// holds another facet is not one a Balance claim compares. Without an
/// anchor at or before the date the claim compares nothing, so it reports
/// none.
fn discrepancy(
    state: &DomainState,
    basis: &ReadBasis,
    account: AccountId,
    candidate: &Candidate,
) -> Result<Option<ReconciliationClaimV1>, EngineError> {
    let Boundary::Date(day) = candidate.boundary else {
        return Ok(None);
    };
    if facets(state, candidate.observation)?.len() != 1 {
        return Ok(None);
    }
    let claim = ReconciliationEngine::evaluate(
        state,
        &ReconciliationRequestV1 {
            kind: ReconciliationKindV1::Balance,
            scope: AccountScope {
                accounts: vec![account],
                components: vec![],
            },
            cutoffs: DateScope {
                start: None,
                end: Some(day),
                basis: DateBasis::Effective,
            },
            evidence: vec![candidate.observation],
            exclusions: vec![],
            basis: basis.clone(),
        },
    )?;
    Ok((!claim.differences.is_empty()).then_some(claim))
}

/// Each leg on `component`'s account of the `pending` adoptions, dated on or
/// before `day` by its effective date, with the leg ID and its allocated
/// observations as evidence.
fn pending(
    state: &DomainState,
    adoptions: &[ActivityAdoptionV1],
    component: RecordId,
    account: AccountId,
    day: Date,
) -> Result<Vec<AccountMovementV1>, EngineError> {
    let mut found = Vec::new();
    for leg_id in adoptions.iter().flat_map(|a| &a.legs) {
        let Some(AuthoritativeRecordV1::FinancialLeg(leg)) = state.records.get(&RecordId(leg_id.0))
        else {
            return Err(incomplete());
        };
        if leg.account_id != account
            || civil_date(DateBasis::Effective, &leg.effective, &leg.posting)
                .is_none_or(|leg_day| leg_day > day)
        {
            continue;
        }
        found.push(AccountMovementV1 {
            account,
            component,
            amount: leg.movement,
            effective: leg.effective.clone(),
            posting: leg.posting.clone(),
            evidence: std::iter::once(RecordId(leg.id.0))
                .chain(leg.allocations.iter().map(|a| a.observation))
                .collect(),
        });
    }
    Ok(found)
}

/// The reported coverage of every receipt from one of `sources`, in the
/// order the receipts were committed.
fn coverage(state: &DomainState, sources: &[SourceIdentityV1]) -> Vec<CoverageV1> {
    let mut receipts: Vec<(CommitSeq, RecordId)> = state
        .receipt_commits
        .iter()
        .map(|(id, commit)| (*commit, *id))
        .collect();
    receipts.sort();
    let mut found = Vec::new();
    for (_, id) in receipts {
        if let Some(AuthoritativeRecordV1::ImportReceipt(receipt)) = state.records.get(&id)
            && sources.contains(&receipt.source)
        {
            found.extend(receipt.reported_coverage.iter().cloned());
        }
    }
    found
}

impl PositionEngine {
    /// The position of each component in `scope` on the requested date,
    /// `basis.effective_scope.end`, at the state's commit.
    pub fn query(
        state: &DomainState,
        basis: &ReadBasis,
        scope: &AccountScope,
    ) -> Result<PositionResponseV1, EngineError> {
        let (day, legs) = requested(state, basis)?;
        let components = components(state, scope)?;
        let policy = PolicyEngine::governing(state, day)?;
        let priority = priority(&policy);
        let evaluated = basis.evaluation_instant.unix_timestamp_nanos();
        let held = RecognitionEngine::pending(state)?;
        let mut codes = BTreeSet::new();
        let mut readings = balances(state, &components, Boundary::Date(day), &mut codes)?;

        let mut assertions = Vec::new();
        let mut movements = Vec::new();
        let mut anchored_balance = Vec::new();
        let mut selected = BTreeSet::new();
        let mut labels = Vec::new();
        let mut sources = Vec::new();
        let mut discrepancies = Vec::new();
        let mut pending_movements = Vec::new();
        let mut found = Vec::new();
        for (component, catalog) in &components {
            let reading = readings.remove(component).unwrap_or_default();
            let chosen = select(&reading.candidates, &priority).ok();
            if chosen.is_none() {
                codes.insert("conflict");
            }
            // Labels, coverage and discrepancies describe the selection and
            // never change it.
            for candidate in chosen.iter().flatten() {
                labels.push(freshness(
                    &policy.freshness_thresholds,
                    candidate,
                    evaluated,
                ));
                if !sources.contains(&candidate.source) {
                    sources.push(candidate.source.clone());
                }
                discrepancies.extend(discrepancy(state, basis, catalog.account_id, candidate)?);
            }
            pending_movements.extend(pending(state, &held, *component, catalog.account_id, day)?);
            let anchored = journal(
                state,
                &legs,
                *component,
                catalog,
                day,
                &mut codes,
                &mut movements,
            )?;
            // The selected amount, else the anchored balance, each in the
            // source's convention: a liability's is the amount owed.
            let amount = match (chosen, &anchored) {
                (None, _) => Found::Missing("conflict"),
                (Some(chosen), _) if !chosen.is_empty() => {
                    let ids: BTreeSet<RecordId> = chosen.iter().map(|c| c.observation).collect();
                    selected.extend(ids.iter().copied());
                    Found::Known(chosen[0].amount, ids.into_iter().collect())
                }
                (Some(_), None) => Found::Missing("unknown_opening"),
                (Some(_), Some(anchored)) => match anchored.amount {
                    SourceValue::Known(amount) => Found::Known(
                        journal_sign(catalog.normal_sign, amount)?,
                        anchored.dependencies.clone(),
                    ),
                    _ => Found::Missing("missing_date_mapping"),
                },
            };
            found.push((*component, amount));
            assertions.extend(reading.assertions);
            anchored_balance.extend(anchored);
        }
        Ok(PositionResponseV1 {
            basis: basis.clone(),
            scope: scope.clone(),
            assertions,
            movements,
            anchored_balance,
            selected_basis: selected.into_iter().collect(),
            pending: pending_movements,
            freshness: labels,
            coverage: coverage(state, &sources),
            discrepancies,
            missing_inputs: codes.into_iter().map(String::from).collect(),
            aggregate: aggregation::aggregate(state, day, found)?,
        })
    }

    /// The graph behind `component`'s position at `basis`, read as `query`
    /// reads it: the balances that could be selected, the anchor and its
    /// opening, and the policy revisions the query uses for the component.
    /// It refuses what `query` refuses.
    pub fn explain(
        state: &DomainState,
        basis: &ReadBasis,
        component: RecordId,
    ) -> Result<ExplanationV1, EngineError> {
        let (day, _) = requested(state, basis)?;
        let scope = AccountScope {
            accounts: vec![],
            components: vec![component],
        };
        let components = components(state, &scope)?;
        let account = components[&component].account_id;
        let policy = PolicyEngine::governing(state, day)?;
        let mut readings = balances(
            state,
            &components,
            Boundary::Date(day),
            &mut BTreeSet::new(),
        )?;
        let candidates = readings.remove(&component).unwrap_or_default().candidates;
        let (selected, conflicting) = match select(&candidates, &priority(&policy)) {
            Ok(selected) => (selected, Vec::new()),
            Err(conflicting) => (Vec::new(), conflicting),
        };
        let has = |pool: &[&Candidate], c: &Candidate| {
            pool.iter().any(|o| o.observation == c.observation)
        };

        let mut graph = Graph::default();
        for candidate in &candidates {
            let role = if has(&selected, candidate) {
                "selected balance"
            } else if has(&conflicting, candidate) {
                "conflicting balance"
            } else {
                "candidate balance"
            };
            let summary = format!("{role} as of {}", date_of(&candidate.as_of));
            graph.node(
                candidate.observation,
                ExplanationNodeKindV1::Evidence,
                summary,
            );
        }
        for candidate in &selected {
            graph.edge(component, candidate.observation, "selected");
        }
        for a in &conflicting {
            for b in &conflicting {
                if a.observation < b.observation && a.amount != b.amount {
                    graph.edge(a.observation, b.observation, "conflicts_with");
                }
            }
        }
        if let Some((anchor, segment)) = anchor(state, account, day)? {
            let opening = RecordId(segment.opening.0);
            let cutoff = anchor.cutoff;
            graph.node(
                anchor.segment,
                ExplanationNodeKindV1::Decision,
                format!("accounting segment from cutoff {cutoff}"),
            );
            graph.node(
                opening,
                ExplanationNodeKindV1::Decision,
                format!("opening recognition at cutoff {cutoff}"),
            );
            graph.edge(anchor.segment, component, "anchors");
            graph.edge(opening, anchor.segment, "anchors");
        }
        // Source priority decides the selection, and the ownership perimeter
        // and reporting currency decide what the aggregate counts. Freshness
        // thresholds label each selected balance.
        for version in [
            version(&policy.source_priority),
            version(&policy.ownership_perimeter),
            version(&policy.reporting_currency),
        ]
        .into_iter()
        .flatten()
        {
            let revision = graph.revision(state, version)?;
            graph.edge(revision, component, "governs");
        }
        if let Some(version) = version(&policy.freshness_thresholds)
            && !selected.is_empty()
        {
            let revision = graph.revision(state, version)?;
            for candidate in &selected {
                graph.edge(revision, candidate.observation, "labels");
            }
        }
        Ok(ExplanationV1 {
            basis: basis.clone(),
            roots: vec![component],
            nodes: graph.nodes.into_values().collect(),
            edges: graph
                .edges
                .into_iter()
                .map(|(from, to, relationship)| ExplanationEdgeV1 {
                    from,
                    to,
                    relationship: relationship.into(),
                })
                .collect(),
        })
    }
}

/// An explanation's nodes in ID order and its edges in (from, to,
/// relationship) order. A node's first summary stays.
#[derive(Default)]
struct Graph {
    nodes: BTreeMap<RecordId, ExplanationNodeV1>,
    edges: BTreeSet<(RecordId, RecordId, &'static str)>,
}

impl Graph {
    fn node(&mut self, id: RecordId, kind: ExplanationNodeKindV1, summary: String) {
        self.nodes
            .entry(id)
            .or_insert(ExplanationNodeV1 { id, kind, summary });
    }

    fn edge(&mut self, from: RecordId, to: RecordId, relationship: &'static str) {
        self.edges.insert((from, to, relationship));
    }

    /// Add the dataset policy revision with `version` as a node, and return
    /// its ID. A dataset has one revision family, so a version names one
    /// revision.
    fn revision(&mut self, state: &DomainState, version: u32) -> Result<RecordId, EngineError> {
        let revision = state
            .records
            .values()
            .find_map(|record| match record {
                AuthoritativeRecordV1::DatasetPolicyRevision(r) if r.version == version => Some(r),
                _ => None,
            })
            .ok_or_else(incomplete)?;
        let from = revision.effective_scope.start.map_or_else(
            || "with no start date".into(),
            |start| format!("from {start}"),
        );
        let summary = format!("policy revision {version} effective {from}");
        self.node(revision.id, ExplanationNodeKindV1::Policy, summary);
        Ok(revision.id)
    }
}

/// The version of the revision that sets a governing policy field.
fn version<T>(setting: &PolicySettingV1<T>) -> Option<u32> {
    match setting {
        PolicySettingV1::Set { version, .. } => Some(*version),
        PolicySettingV1::NotSet => None,
    }
}

/// A temporal value's date for a node summary: a civil date, or an instant's
/// UTC date (its offset is always zero). It never repeats the source's
/// original text.
fn date_of(temporal: &TemporalValue) -> String {
    match temporal {
        TemporalValue::Date { civil, .. } => civil.to_string(),
        TemporalValue::Instant { utc, .. } => format!("{} UTC", utc.date()),
        TemporalValue::Unknown { .. } => "an unknown date".into(),
    }
}
