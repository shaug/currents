//! Safe typed errors: no arbitrary diagnostic string crosses this boundary.
use crate::RecordId;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    Unauthorized,
    Locked,
    UnsupportedVersion,
    StaleBasis,
    InvalidReference,
    DuplicateRecognition,
    AllocationConflict,
    Unbalanced,
    PrecisionOrOverflow,
    IncompleteBasis,
    IntegrityFailure,
    StorageFailure,
    Canceled,
    ResultUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeNextAction {
    Authenticate,
    Unlock,
    Upgrade,
    RefreshBasis,
    CorrectInput,
    InspectEvidence,
    RetrySameCommand,
    Recover,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{class:?}; {next_action:?}")]
#[serde(deny_unknown_fields)]
pub struct EngineError {
    pub class: ErrorClass,
    pub affected_scope: Vec<RecordId>,
    pub next_action: SafeNextAction,
}
impl EngineError {
    pub fn invalid() -> Self {
        Self {
            class: ErrorClass::InvalidReference,
            affected_scope: vec![],
            next_action: SafeNextAction::CorrectInput,
        }
    }
    pub fn precision() -> Self {
        Self {
            class: ErrorClass::PrecisionOrOverflow,
            ..Self::invalid()
        }
    }
    pub fn version() -> Self {
        Self {
            class: ErrorClass::UnsupportedVersion,
            next_action: SafeNextAction::Upgrade,
            affected_scope: vec![],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Healthy,
    SourcePartial,
    SourceStale,
    Locked,
    ProjectionLagging,
    IntegrityFailed,
    MigrationRequired,
    RecoveryInProgress,
}

impl crate::schema::Schema for ErrorClass {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"string","enum":["unauthorized","locked","unsupported_version","stale_basis","invalid_reference","duplicate_recognition","allocation_conflict","unbalanced","precision_or_overflow","incomplete_basis","integrity_failure","storage_failure","canceled","result_unknown"]})
    }
}
