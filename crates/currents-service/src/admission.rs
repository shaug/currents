use crate::commands::{ApprovalChallenge, CommandPlan, ResultAccess};
use currents_storage::CommitControl;
use currents_types::*;
use std::sync::Arc;

pub(crate) const MAX_ADMISSIONS: usize = 1024;
pub(crate) const MAX_PREVIEWS: usize = 1024;
pub(crate) const MAX_RETAINED_BYTES: usize = 128 * 1_048_576;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Queued,
    Running,
    CommitSubmitted,
    Settled,
    Unknown,
}
pub(crate) struct Admission {
    pub digest: Digest,
    pub actor: RecordId,
    pub grant_id: GrantId,
    pub preview: currents_protocol::PreviewV1,
    pub control: Arc<CommitControl>,
    pub phase: Phase,
    pub result: Option<CommandResultV1>,
    pub retained_bytes: usize,
}
impl Admission {
    pub fn phase(&self) -> Phase {
        if self.phase == Phase::Running && self.control.commit_submitted() {
            Phase::CommitSubmitted
        } else {
            self.phase
        }
    }
    pub fn access(&self) -> ResultAccess<'_> {
        ResultAccess::Admitted {
            actor: self.actor,
            grant_id: self.grant_id,
            preview: &self.preview,
        }
    }
    pub fn status(&self, id: CommandId) -> CommandResultV1 {
        if let Some(result) = &self.result {
            return result.clone();
        }
        status(
            id,
            self.digest,
            match self.phase() {
                Phase::Queued | Phase::Running | Phase::CommitSubmitted => CommandState::Pending,
                Phase::Settled => CommandState::DefinitelyNotCommitted,
                Phase::Unknown => CommandState::ResultUnknown,
            },
        )
    }
}
pub(crate) struct PreviewEntry {
    pub challenge: ApprovalChallenge,
    pub plan: CommandPlan,
    pub bytes: usize,
}
pub(crate) fn status(id: CommandId, digest: Digest, state: CommandState) -> CommandResultV1 {
    let uncertain = state == CommandState::ResultUnknown;
    CommandResultV1 {
        command_id: id,
        digest,
        state,
        commit: None,
        effects: EffectsV1 {
            records: vec![],
            accounts: vec![],
            amounts: vec![],
            invalidations: vec![],
        },
        health: if uncertain {
            vec![Health::RecoveryInProgress]
        } else {
            vec![]
        },
    }
}
pub(crate) fn error(class: ErrorClass) -> EngineError {
    EngineError {
        class,
        affected_scope: vec![],
        next_action: match class {
            ErrorClass::Unauthorized => SafeNextAction::Authenticate,
            ErrorClass::Locked => SafeNextAction::Unlock,
            ErrorClass::StaleBasis => SafeNextAction::RefreshBasis,
            ErrorClass::ResultUnknown => SafeNextAction::RetrySameCommand,
            _ => SafeNextAction::Recover,
        },
    }
}
pub(crate) fn fresh(challenge: &ApprovalChallenge, now: OffsetDateTime) -> Result<(), EngineError> {
    let elapsed = now
        .unix_timestamp_nanos()
        .checked_sub(challenge.issued_at.unix_timestamp_nanos());
    if elapsed.is_none_or(|n| !(0..300_000_000_000).contains(&n)) {
        return Err(error(ErrorClass::Unauthorized));
    }
    Ok(())
}
