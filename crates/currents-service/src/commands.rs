//! Trusted in-process planning/host seams. None is a client transport or SQL API.
use currents_protocol::*;
use currents_types::*;

#[derive(Clone, serde::Serialize)]
pub struct CommandPlan {
    pub decision_plan: DecisionPlan,
    pub frozen_batch: AccountingBatchV1,
    pub effects: EffectsV1,
    pub warnings: Vec<ErrorClass>,
    pub approval_scope: ApprovalScopeV1,
    pub decision_id: RecordId,
    pub manifest_id: RecordId,
    pub policy: PolicyReferenceV1,
}
impl CommandPlan {
    pub(crate) fn validate_scope(&self) -> Result<(), EngineError> {
        let covered = |account: &AccountId| {
            self.effects.accounts.contains(account)
                && self.approval_scope.accounts.contains(account)
        };
        if self
            .effects
            .accounts
            .iter()
            .any(|account| !covered(account))
            || self
                .frozen_batch
                .operations
                .iter()
                .any(|operation| match operation {
                    FrozenNativeOperationV1::Account(account) => !covered(&account.account_id),
                    FrozenNativeOperationV1::Entry(entry) => entry
                        .postings
                        .iter()
                        .any(|posting| !covered(&posting.original.account_id)),
                    FrozenNativeOperationV1::Policy(_)
                    | FrozenNativeOperationV1::Calendar(_)
                    | FrozenNativeOperationV1::Unit(_) => false,
                })
        {
            return Err(EngineError::invalid());
        }
        Ok(())
    }
}
/// Pure typed planning at the supplied exact prefix. Subsystem owners implement
/// financial semantics; unsupported operations must return UnsupportedVersion.
pub trait CommandPlanner: Send + Sync {
    fn prepare(
        &self,
        body: &CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<CommandPlan, EngineError>;
    /// Apply the authorized query's scope before constructing any result/count.
    fn query(
        &self,
        query: &QueryV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<QueryResponseV1, EngineError>;
}
#[derive(Clone)]
pub struct ApprovalChallenge {
    pub dataset_id: DatasetId,
    pub instance_id: InstanceId,
    pub client: RecordId,
    pub session: SessionId,
    pub grant_id: GrantId,
    pub command_id: CommandId,
    pub nonce: uuid::Uuid,
    pub issued_at: OffsetDateTime,
    pub preview: PreviewV1,
}
/// Host-only scope evidence. Never return its unfiltered contents to a client.
pub enum ResultAccess<'a> {
    Committed(&'a DomainCommitV1),
    Admitted {
        actor: RecordId,
        grant_id: GrantId,
        preview: &'a PreviewV1,
    },
    Missing,
}
/// The trusted host owns authenticated session/capability/approval proof. The
/// service owns lookup ordering, fresh-clock sampling, binding and one-use state.
/// No implementation supplied here may claim signed-host production authority.
pub trait HostGrants: Send + Sync {
    /// Invalidate every session/capability for this owner before draining close.
    /// This is an in-memory operation, not credential or security configuration.
    fn revoke_sessions(&self, dataset: DatasetId, instance: InstanceId);
    fn authenticate(
        &self,
        ctx: &RequestContext,
        dataset: DatasetId,
        capability: Option<&str>,
        now: OffsetDateTime,
    ) -> Result<(), EngineError>;
    fn authorize_command(
        &self,
        ctx: &RequestContext,
        body: &CommandBodyV1,
    ) -> Result<(), EngineError>;
    fn authorize_effects(
        &self,
        ctx: &RequestContext,
        preview: &PreviewV1,
    ) -> Result<(), EngineError>;
    /// Make this challenge available only to the authenticated approval helper.
    fn publish_challenge(&self, challenge: &ApprovalChallenge) -> Result<(), EngineError>;
    /// Read-only verification, safe to repeat at execution. The service, not
    /// this verifier, consumes the challenge exactly once during admission.
    fn verify_approval(
        &self,
        ctx: &RequestContext,
        auth: &AuthorizationEnvelope,
        challenge: &ApprovalChallenge,
        now: OffsetDateTime,
    ) -> Result<(), EngineError>;
    /// Require the original authorized scope or fresh owner authority, including
    /// all effect/record/account fields. Check before exposing digest conflicts.
    /// Missing/recovery status also requires an authorized lookup scope.
    fn authorize_result(
        &self,
        ctx: &RequestContext,
        access: ResultAccess<'_>,
    ) -> Result<(), EngineError>;
    fn authorize_query(&self, ctx: &RequestContext, query: &QueryV1) -> Result<(), EngineError>;
}
