//! Per-component position selection (C09). For each component in scope, the
//! query lists the source balances its aliases bind, selects the latest
//! compatible one at or before the requested date, and reports the journal's
//! anchored balance or, without an anchor, the known movements. It reads the
//! state and writes nothing. It never rolls a source balance forward.
use crate::activity_predicate::civil_date;
use crate::catalog::CatalogEngine;
use crate::dependencies::{self, Anchor};
use crate::policy::PolicyEngine;
use crate::reconciliation::facets;
use currents_types::*;
use std::collections::{BTreeMap, BTreeSet};

pub struct PositionEngine;

/// The most components one query reports.
pub const MAX_POSITION_COMPONENTS: usize = 256;

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
    connection: RecordId,
    boundary: Boundary,
    amount: Amount,
}

/// The candidates no other candidate is strictly later than. When they share
/// one boundary, the highest-ranked source in `priority` among them decides.
/// Equal amounts corroborate and are all selected. Different amounts are a
/// conflict, `None`, with nothing selected.
fn select(candidates: &[Candidate], priority: &[RecordId]) -> Option<Vec<RecordId>> {
    let latest: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| !candidates.iter().any(|o| o.boundary.after(c.boundary)))
        .collect();
    let Some(first) = latest.first() else {
        return Some(Vec::new());
    };
    let mut pool = latest.clone();
    if latest.iter().all(|c| c.boundary == first.boundary)
        && let Some(top) = priority
            .iter()
            .find(|source| latest.iter().any(|c| c.connection == **source))
    {
        pool.retain(|c| c.connection == *top);
    }
    pool.iter()
        .all(|c| c.amount == pool[0].amount)
        .then(|| pool.iter().map(|c| c.observation).collect())
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
                        connection: observation.source.connection_id,
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
/// anchor whose cutoff is on or before `day`, the anchored balance: the
/// opening plus the live legs on the account after the cutoff through `day`.
/// Without one, it pushes each live leg on the account through `day` to
/// `movements` and adds `unknown_opening`.
fn journal(
    state: &DomainState,
    legs: &[&FinancialLegV1],
    component: RecordId,
    account: AccountId,
    day: Date,
    codes: &mut BTreeSet<&'static str>,
    movements: &mut Vec<AccountMovementV1>,
) -> Result<Option<AnchoredBalanceV1>, EngineError> {
    let anchor = Anchor::active(state, account)?.filter(|a| a.cutoff <= day);
    let mut amount = anchor.as_ref().map(|a| a.opening.amount);
    let mut dependencies: Vec<RecordId> = anchor.iter().map(|a| a.segment).collect();
    let mut unmapped = false;
    for leg in legs.iter().filter(|l| l.account_id == account) {
        let Some(leg_day) = civil_date(DateBasis::Effective, &leg.effective, &leg.posting) else {
            unmapped = true;
            continue;
        };
        if leg_day > day {
            continue;
        }
        match &anchor {
            Some(anchor) if leg_day > anchor.cutoff => {
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
    let Some(anchor) = anchor else {
        codes.insert("unknown_opening");
        return Ok(None);
    };
    let Some(AuthoritativeRecordV1::AccountingSegment(segment)) =
        state.records.get(&anchor.segment)
    else {
        return Err(EngineError::invalid());
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

impl PositionEngine {
    /// The position of each component in `scope` on the requested date,
    /// `basis.effective_scope.end`, at the state's commit.
    pub fn query(
        state: &DomainState,
        basis: &ReadBasis,
        scope: &AccountScope,
    ) -> Result<PositionResponseV1, EngineError> {
        if basis.through_commit != state.through {
            return Err(dependencies::stale());
        }
        let legs = dependencies::live_legs(state)?;
        let (Some(day), DateBasis::Effective) =
            (basis.effective_scope.end, basis.effective_scope.basis)
        else {
            return Err(EngineError::invalid());
        };
        let components = components(state, scope)?;
        let priority = match PolicyEngine::governing(state, day)?.source_priority {
            PolicySettingV1::Set {
                value: SourceValue::Known(order),
                ..
            } => order,
            _ => Vec::new(),
        };
        let mut codes = BTreeSet::new();
        let mut readings = balances(state, &components, Boundary::Date(day), &mut codes)?;

        let mut assertions = Vec::new();
        let mut movements = Vec::new();
        let mut anchored_balance = Vec::new();
        let mut selected = BTreeSet::new();
        let mut aggregate = Vec::new();
        for (component, catalog) in &components {
            let reading = readings.remove(component).unwrap_or_default();
            match select(&reading.candidates, &priority) {
                Some(ids) => selected.extend(ids),
                None => {
                    codes.insert("conflict");
                }
            }
            assertions.extend(reading.assertions);
            anchored_balance.extend(journal(
                state,
                &legs,
                *component,
                catalog.account_id,
                day,
                &mut codes,
                &mut movements,
            )?);
            aggregate.push(AggregateComponentV1 {
                component: *component,
                amount: SourceValue::Unknown,
                evidence: Vec::new(),
            });
        }
        Ok(PositionResponseV1 {
            basis: basis.clone(),
            scope: scope.clone(),
            assertions,
            movements,
            anchored_balance,
            selected_basis: selected.into_iter().collect(),
            pending: Vec::new(),
            freshness: Vec::new(),
            coverage: Vec::new(),
            discrepancies: Vec::new(),
            missing_inputs: codes.into_iter().map(String::from).collect(),
            aggregate: AggregateV1::Indeterminate {
                components: aggregate,
                obstructions: vec!["aggregation_unavailable".into()],
            },
        })
    }
}
