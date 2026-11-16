//! Deterministic typed domain projection. No clocks, entropy, network or storage.
use currents_types::*;
pub struct ReplayEngine;
impl ReplayEngine {
    /// Applies an already authenticated commit without reinterpreting its decisions.
    /// Anonymous rows remain in ordered authority; this is only the latest-ID index.
    pub fn apply(state: &mut DomainState, commit: &DomainCommitV1) -> Result<(), EngineError> {
        if commit.native_head_after.semantics_version != 1 {
            return Err(EngineError::version());
        }
        if commit.seq.0
            != state
                .through
                .0
                .checked_add(1)
                .ok_or_else(EngineError::invalid)?
            || commit.previous_digest != state.domain_digest
            || commit.command_result.commit != Some(commit.seq)
            || commit.command_result.state != CommandState::Committed
        {
            return Err(EngineError::invalid());
        }
        let mut records = state.records.clone();
        for record in commit.records.iter().cloned().chain(
            commit
                .decisions
                .iter()
                .cloned()
                .map(AuthoritativeRecordV1::Decision),
        ) {
            let id = match &record {
                AuthoritativeRecordV1::Dataset(v) => {
                    if v.format_versions.records != 1
                        || v.format_versions.canonical != 1
                        || v.format_versions.protocol != 1
                    {
                        return Err(EngineError::version());
                    }
                    Some(RecordId(v.id.0))
                }
                AuthoritativeRecordV1::FrozenAccountingManifest(v) => {
                    if v.schema_version != 1 || v.native_semantics != 1 {
                        return Err(EngineError::version());
                    }
                    Some(v.id)
                }
                AuthoritativeRecordV1::Decision(v) => {
                    if v.schema_version != 1 {
                        return Err(EngineError::version());
                    }
                    if v.commit != commit.seq
                        || v.command_id != commit.command_result.command_id
                        || v.command_digest != commit.command_result.digest
                    {
                        return Err(EngineError::invalid());
                    }
                    Some(v.id)
                }
                AuthoritativeRecordV1::DateConversionPolicy(v) => Some(v.id),
                AuthoritativeRecordV1::DatasetPolicy(v) => Some(v.id),
                AuthoritativeRecordV1::UnitCatalog(v) => Some(v.id),
                AuthoritativeRecordV1::AccountComponent(v) => Some(v.id),
                AuthoritativeRecordV1::AccountAlias(v) => Some(v.id),
                AuthoritativeRecordV1::OwnershipPerimeter(v) => Some(v.id),
                AuthoritativeRecordV1::ImportReceipt(v) => Some(v.id),
                AuthoritativeRecordV1::ObservationVersion(v) => Some(v.id),
                AuthoritativeRecordV1::FinancialEvent(v) => Some(v.id),
                AuthoritativeRecordV1::FinancialLeg(v) => Some(RecordId(v.id.0)),
                AuthoritativeRecordV1::Recognition(v) => Some(RecordId(v.id.0)),
                AuthoritativeRecordV1::AccountCatalogRevision(v) => Some(v.id),
                AuthoritativeRecordV1::ActivityAdoption(v) => Some(v.id),
                AuthoritativeRecordV1::PendingExpiry(v) => Some(v.id),
                AuthoritativeRecordV1::AssociationRevision(v) => Some(v.id),
                AuthoritativeRecordV1::RelationshipCapture(v) => Some(v.id),
                AuthoritativeRecordV1::AccountingSegment(v) => Some(v.id),
                // These owners have not delivered semantic validation yet. Full
                // structural closure alone must never certify financial trust.
                AuthoritativeRecordV1::Association(_)
                | AuthoritativeRecordV1::Allocation(_)
                | AuthoritativeRecordV1::OccurrenceMatch(_) => return Err(EngineError::version()),
                AuthoritativeRecordV1::ReconciliationClaim(v) => Some(v.id),
                AuthoritativeRecordV1::Organization(v) => {
                    if !v.allocations.is_empty() {
                        return Err(EngineError::version());
                    }
                    Some(v.id)
                }
                AuthoritativeRecordV1::Rule(v) => Some(v.id),
                AuthoritativeRecordV1::Review(v) => Some(v.id),
                AuthoritativeRecordV1::Commitment(v) => Some(RecordId(v.id.0)),
                AuthoritativeRecordV1::Scenario(v) => Some(v.id),
                AuthoritativeRecordV1::ForecastRun(v) => Some(v.id),
                AuthoritativeRecordV1::ParseProduct(_) => None,
                AuthoritativeRecordV1::AddressedParseProduct(v) => {
                    v.validate()?;
                    Some(v.id)
                }
            };
            if let Some(id) = id {
                records.insert(id, record);
            }
        }
        let mut projection = state.clone();
        projection.records = records;
        projection.dataset_policies =
            crate::policy_history::PolicyHistory::project(state, &commit.records)?;
        crate::recognition_replay::validate_transition(state, &projection, commit)?;
        state.records = projection.records;
        state.dataset_policies = projection.dataset_policies;
        state.through = commit.seq;
        state.domain_digest = commit.digest;
        Ok(())
    }
}
