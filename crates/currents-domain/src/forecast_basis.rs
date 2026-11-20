//! Replay-only qualification of a bounded retained starting-basis artifact.
use currents_types::*;

pub(crate) fn exact_anchor(
    prior: &DomainState,
    next: &DomainState,
    run: &ForecastRunV1,
) -> Option<VerifiedForecastAnchor> {
    let basis = &run.request.basis;
    let input = &run.expanded_inputs;
    let [position] = input.starting_positions.as_slice() else {
        return None;
    };
    let [selected] = position.selected_basis.as_slice() else {
        return None;
    };
    let SourceValue::Known(amount) = &position.amount else {
        return None;
    };
    let date = position.as_of.0;
    let index = prior.accounting.as_ref()?;
    let AuthoritativeRecordV1::Dataset(dataset) =
        prior.records.get(&RecordId(basis.dataset_id.0))?
    else {
        return None;
    };
    let AuthoritativeRecordV1::Scenario(scenario) = next.records.get(&run.request.scenario)? else {
        return None;
    };
    let AuthoritativeRecordV1::UnitCatalog(catalog) = prior.records.get(&dataset.unit_catalog)?
    else {
        return None;
    };
    if basis.through_commit != prior.through
        || prior.through == CommitSeq(0)
        || index.prefix.through != prior.through
        || index.prefix.domain_digest != prior.domain_digest
        || basis.query_semantics_version != 1
        || basis.selection_policy_version != 1
        || basis.unit_catalog_version != catalog.version
        || basis.effective_scope.basis != DateBasis::Effective
        || basis.effective_scope.start != Some(date)
        || basis.effective_scope.end != Some(date)
        || run.request.start != position.as_of
        || basis.evaluation_instant.date() < date
        || scenario.basis != *basis
        || scenario.version != 1
        || !scenario.commitments.is_empty()
        || !scenario.assumptions.is_empty()
        || !run.request.commitment_versions.is_empty()
        || !run.request.assumption_versions.is_empty()
        || !input.commitments.is_empty()
        || !input.occurrences.is_empty()
        || !input.assumptions.is_empty()
        || !input.matches.is_empty()
        || !position.inclusion_map.is_empty()
        || !run.outputs.is_empty()
        || run.algorithm_version != 1
    {
        return None;
    }
    let AuthoritativeRecordV1::AccountingSegment(segment) = prior.records.get(selected)? else {
        return None;
    };
    let (root, successor) = crate::segments::segment_family(prior, segment.component).ok()??;
    if root != segment || successor.is_some() || !segment.included_activity.is_empty() {
        return None;
    }
    let account = crate::catalog::CatalogEngine::current(prior, segment.component).ok()?;
    if position.account != account.catalog.account_id
        || !matches!(
            account.catalog.account_class,
            AccountClass::Asset | AccountClass::Liability
        )
        || index.reversed.contains(&segment.opening)
    {
        return None;
    }
    let [entry] = index.recognitions.get(&segment.opening)?.as_slice() else {
        return None;
    };
    let mut postings = entry
        .postings
        .iter()
        .filter(|posting| posting.original.account_id == position.account);
    let opening = &postings.next()?.original;
    if postings.next().is_some()
        || opening.amount.normalized().ok()? != amount.normalized().ok()?
        || !matches!(&opening.effective, TemporalValue::Date { civil, .. } if *civil == date)
    {
        return None;
    }
    // The retained opening is not sufficient if another live native posting
    // touches the account through its effective cutoff. Include counterpostings,
    // even from legacy entries with no domain economic leg to inspect.
    for (recognition, entries) in &index.recognitions {
        if *recognition == segment.opening || index.reversed.contains(recognition) {
            continue;
        }
        for posting in entries
            .iter()
            .flat_map(|entry| &entry.postings)
            .filter(|posting| posting.original.account_id == position.account)
        {
            match &posting.original.effective {
                TemporalValue::Date { civil, .. } if *civil > date => {}
                _ => return None,
            }
        }
    }
    Some(VerifiedForecastAnchor {
        account: position.account,
        as_of: position.as_of,
        segment: *selected,
    })
}
