//! One serialized writer; dropping a submit future never drops admitted work.
use crate::{admission::*, commands::*};
use currents_protocol::*;
use currents_storage::{CommitControl, CommitFailure, EncryptedStore, PreparedCommit};
use currents_types::*;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, RwLock, oneshot};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ServiceProfile {
    Synthetic,
    Production,
}
struct Meta {
    prefix: Option<AccountingPrefixV1>,
    closing: bool,
    poisoned: bool,
}
struct Inner {
    dataset: DatasetId,
    instance: InstanceId,
    host: Arc<dyn HostGrants>,
    planner: Arc<dyn CommandPlanner>,
    clock: Arc<dyn TrustedClock + Send + Sync>,
    store: RwLock<Option<EncryptedStore>>,
    serial: AsyncMutex<()>,
    meta: Mutex<Meta>,
    previews: Mutex<BTreeMap<(SessionId, CommandId), PreviewEntry>>,
    admissions: AsyncMutex<BTreeMap<CommandId, Admission>>,
    tasks: AsyncMutex<Vec<tokio::task::JoinHandle<()>>>,
    #[cfg(feature = "synthetic-test-fixtures")]
    controls: Mutex<BTreeMap<CommandId, Arc<CommitControl>>>,
}
#[derive(Clone)]
pub struct Service {
    inner: Arc<Inner>,
}
struct Job {
    ctx: RequestContext,
    body: CommandBodyV1,
    auth: AuthorizationEnvelope,
    preview: PreviewEntry,
    control: Arc<CommitControl>,
}
impl Service {
    /// Trusted owner construction, never a transport entrypoint. The current
    /// unsigned implementation accepts synthetic profiles only. Rejected stores
    /// are explicitly closed; construction does not certify recovery complete.
    pub async fn new(
        store: EncryptedStore,
        dataset: DatasetId,
        instance: InstanceId,
        profile: ServiceProfile,
        host: Arc<dyn HostGrants>,
        planner: Arc<dyn CommandPlanner>,
        clock: Arc<dyn TrustedClock + Send + Sync>,
    ) -> Result<Self, EngineError> {
        if profile != ServiceProfile::Synthetic {
            store.close().await?;
            return Err(error(ErrorClass::Unauthorized));
        }
        Ok(Self {
            inner: Arc::new(Inner {
                dataset,
                instance,
                host,
                planner,
                clock,
                store: RwLock::new(Some(store)),
                serial: AsyncMutex::new(()),
                meta: Mutex::new(Meta {
                    prefix: None,
                    closing: false,
                    poisoned: false,
                }),
                previews: Mutex::new(BTreeMap::new()),
                admissions: AsyncMutex::new(BTreeMap::new()),
                tasks: AsyncMutex::new(vec![]),
                #[cfg(feature = "synthetic-test-fixtures")]
                controls: Mutex::new(BTreeMap::new()),
            }),
        })
    }
    fn prefix(&self) -> Result<AccountingPrefixV1, EngineError> {
        let meta = self
            .inner
            .meta
            .lock()
            .map_err(|_| error(ErrorClass::IntegrityFailure))?;
        if meta.closing {
            return Err(error(ErrorClass::Locked));
        }
        meta.prefix
            .clone()
            .ok_or_else(|| error(ErrorClass::ResultUnknown))
    }
    fn authenticate(
        &self,
        ctx: &RequestContext,
        capability: Option<&str>,
    ) -> Result<OffsetDateTime, EngineError> {
        if self
            .inner
            .meta
            .lock()
            .map_err(|_| error(ErrorClass::IntegrityFailure))?
            .closing
        {
            return Err(error(ErrorClass::Locked));
        }
        if ctx.instance_id != self.inner.instance {
            return Err(error(ErrorClass::Unauthorized));
        }
        let now = self.inner.clock.now();
        self.inner
            .host
            .authenticate(ctx, self.inner.dataset, capability, now)?;
        Ok(now)
    }
    /// Initial recovery only. An uncertain COMMIT requires close/reopen before
    /// this audit can authorize absence; a live uncertain connection is not reset.
    pub async fn recover(&self) -> Result<(), EngineError> {
        let _writer = self.inner.serial.lock().await;
        let admissions = self.inner.admissions.lock().await;
        if !admissions.is_empty() {
            return Err(error(ErrorClass::ResultUnknown));
        }
        {
            let meta = self
                .inner
                .meta
                .lock()
                .map_err(|_| error(ErrorClass::IntegrityFailure))?;
            if meta.closing || meta.poisoned {
                return Err(error(ErrorClass::ResultUnknown));
            }
        }
        let store = self.inner.store.read().await;
        let prefix = store
            .as_ref()
            .ok_or_else(|| error(ErrorClass::Locked))?
            .writer()
            .audited_prefix()
            .await?;
        self.inner
            .meta
            .lock()
            .map_err(|_| error(ErrorClass::IntegrityFailure))?
            .prefix = Some(prefix);
        Ok(())
    }
    pub fn preview(
        &self,
        ctx: RequestContext,
        body: CommandBodyV1,
    ) -> Result<PreviewV1, EngineError> {
        let now = self.authenticate(&ctx, None)?;
        self.inner.host.authorize_command(&ctx, &body)?;
        if body.dataset_id != self.inner.dataset {
            return Err(error(ErrorClass::Unauthorized));
        }
        let digest = command_digest(&body)?;
        let prefix = self.prefix()?;
        if body.expected_head != prefix.through {
            return Err(error(ErrorClass::StaleBasis));
        }
        let plan = self.inner.planner.prepare(&body, &prefix)?;
        plan.validate_scope()?;
        if plan.frozen_batch.preconditions != prefix
            || plan.decision_plan.invalidations != plan.effects.invalidations
            || plan.decision_plan.records.iter().any(|r| {
                matches!(
                    r,
                    AuthoritativeRecordV1::Decision(_)
                        | AuthoritativeRecordV1::FrozenAccountingManifest(_)
                )
            })
        {
            return Err(EngineError::invalid());
        }
        let bytes = canonical_bytes_with_limits(
            &plan,
            CanonicalLimits {
                max_bytes: 65 * 1_048_576,
                max_collection_items: 65_536,
            },
        )?;
        let retained_bytes = bytes.len();
        let preview = PreviewV1 {
            digest,
            expected_head: body.expected_head,
            effects: plan.effects.clone(),
            warnings: plan.warnings.clone(),
            approval_scope: plan.approval_scope.clone(),
            closure_digest: Digest(Sha256::digest(bytes).into()),
        };
        // The response itself still obeys C03's ordinary message limit.
        canonical_bytes(&preview)?;
        self.inner.host.authorize_effects(&ctx, &preview)?;
        let challenge = ApprovalChallenge {
            dataset_id: body.dataset_id,
            instance_id: ctx.instance_id,
            client: ctx.authenticated_client,
            session: ctx.session,
            grant_id: ctx.grant_id,
            command_id: body.command_id,
            nonce: uuid::Uuid::new_v4(),
            issued_at: now,
            preview: preview.clone(),
        };
        let mut previews = self
            .inner
            .previews
            .lock()
            .map_err(|_| error(ErrorClass::IntegrityFailure))?;
        previews.retain(|_, entry| fresh(&entry.challenge, now).is_ok());
        if previews.len() >= MAX_PREVIEWS
            || previews
                .values()
                .fold(retained_bytes, |n, entry| n.saturating_add(entry.bytes))
                > MAX_RETAINED_BYTES
        {
            return Err(EngineError::invalid());
        }
        self.inner.host.publish_challenge(&challenge)?;
        previews.insert(
            (ctx.session, body.command_id),
            PreviewEntry {
                challenge,
                plan,
                bytes: retained_bytes,
            },
        );
        Ok(preview)
    }
    pub async fn submit(
        &self,
        ctx: RequestContext,
        body: CommandBodyV1,
        auth: AuthorizationEnvelope,
    ) -> Result<CommandResultV1, EngineError> {
        self.authenticate(&ctx, Some(&auth.capability))?;
        if auth
            .capability
            .len()
            .saturating_add(auth.approval.as_ref().map_or(0, String::len))
            > MAX_MESSAGE_BYTES
        {
            return Err(EngineError::invalid());
        }
        if auth.session != ctx.session || body.dataset_id != self.inner.dataset {
            return Err(error(ErrorClass::Unauthorized));
        }
        self.inner.host.authorize_command(&ctx, &body)?;
        let digest = command_digest(&body)?;
        let request_bytes = canonical_bytes(&body)?
            .len()
            .saturating_add(auth.capability.len())
            .saturating_add(auth.approval.as_ref().map_or(0, String::len));
        self.reap_finished().await?;
        let mut admissions = self.inner.admissions.lock().await;
        let prefix = self.prefix()?;
        let store = self.inner.store.read().await;
        if let Some(commit) = store
            .as_ref()
            .ok_or_else(|| error(ErrorClass::Locked))?
            .writer()
            .command_record(body.command_id)
            .await?
        {
            self.authenticate(&ctx, Some(&auth.capability))?;
            self.inner
                .host
                .authorize_result(&ctx, ResultAccess::Committed(&commit))?;
            if commit.command_result.digest != digest {
                return Err(EngineError::invalid());
            }
            return Ok(commit.command_result);
        }
        drop(store);
        let now = self.authenticate(&ctx, Some(&auth.capability))?;
        self.inner.host.authorize_command(&ctx, &body)?;
        if let Some(entry) = admissions.get(&body.command_id) {
            self.inner.host.authorize_result(&ctx, entry.access())?;
            if entry.digest != digest {
                return Err(EngineError::invalid());
            }
            return Ok(entry.status(body.command_id));
        }
        if body.expected_head != prefix.through {
            return Err(error(ErrorClass::StaleBasis));
        }
        if admissions.len() >= MAX_ADMISSIONS {
            return Err(EngineError::invalid());
        }
        if auth.approval.is_none() {
            return Err(error(ErrorClass::Unauthorized));
        }
        let preview = {
            let mut previews = self
                .inner
                .previews
                .lock()
                .map_err(|_| error(ErrorClass::IntegrityFailure))?;
            let key = (ctx.session, body.command_id);
            let entry = previews
                .get(&key)
                .ok_or_else(|| error(ErrorClass::Unauthorized))?;
            let c = &entry.challenge;
            if admissions
                .values()
                .fold(entry.bytes.saturating_add(request_bytes), |n, entry| {
                    n.saturating_add(entry.retained_bytes)
                })
                > MAX_RETAINED_BYTES
            {
                return Err(EngineError::invalid());
            }
            if c.preview.digest != digest
                || c.preview.expected_head != body.expected_head
                || c.client != ctx.authenticated_client
                || c.grant_id != ctx.grant_id
                || c.instance_id != ctx.instance_id
                || c.dataset_id != body.dataset_id
            {
                return Err(error(ErrorClass::Unauthorized));
            }
            fresh(c, now)?;
            self.inner.host.authorize_effects(&ctx, &c.preview)?;
            self.inner.host.verify_approval(&ctx, &auth, c, now)?;
            previews
                .remove(&key)
                .ok_or_else(|| error(ErrorClass::Unauthorized))?
        };
        #[cfg(feature = "synthetic-test-fixtures")]
        let control = self
            .inner
            .controls
            .lock()
            .map_err(|_| error(ErrorClass::IntegrityFailure))?
            .remove(&body.command_id)
            .unwrap_or_else(|| Arc::new(CommitControl::default()));
        #[cfg(not(feature = "synthetic-test-fixtures"))]
        let control = Arc::new(CommitControl::default());
        admissions.insert(
            body.command_id,
            Admission {
                digest,
                actor: ctx.authenticated_client,
                grant_id: ctx.grant_id,
                preview: preview.challenge.preview.clone(),
                control: control.clone(),
                phase: Phase::Queued,
                result: None,
                retained_bytes: preview.bytes.saturating_add(request_bytes),
            },
        );
        let id = body.command_id;
        let (tx, rx) = oneshot::channel();
        let service = self.clone();
        let task = tokio::spawn(async move {
            let result = service
                .execute(Job {
                    ctx,
                    body,
                    auth,
                    preview,
                    control,
                })
                .await;
            let mut admissions = service.inner.admissions.lock().await;
            if result
                .as_ref()
                .is_ok_and(|r| r.state == CommandState::Committed)
            {
                // Durable history now owns identity, scope and retry results.
                admissions.remove(&id);
            }
            if let Some(entry) = admissions.get_mut(&id) {
                entry.phase = if result
                    .as_ref()
                    .is_ok_and(|r| r.state == CommandState::ResultUnknown)
                {
                    Phase::Unknown
                } else {
                    Phase::Settled
                };
                entry.result = result.as_ref().ok().cloned();
                entry.retained_bytes =
                    canonical_bytes(&entry.preview).map_or(MAX_MESSAGE_BYTES, |b| b.len());
            }
            let _ = tx.send(result);
        });
        self.inner.tasks.lock().await.push(task);
        drop(admissions);
        rx.await
            .unwrap_or_else(|_| Ok(status(id, digest, CommandState::ResultUnknown)))
    }
    async fn execute(&self, job: Job) -> Result<CommandResultV1, EngineError> {
        let _writer = self.inner.serial.lock().await;
        let digest = job.preview.challenge.preview.digest;
        {
            let mut admissions = self.inner.admissions.lock().await;
            let entry = admissions
                .get_mut(&job.body.command_id)
                .ok_or_else(|| error(ErrorClass::IntegrityFailure))?;
            if entry.phase == Phase::Settled {
                return Ok(entry.status(job.body.command_id));
            }
            entry.phase = Phase::Running;
        }
        let now = self.authenticate(&job.ctx, Some(&job.auth.capability))?;
        self.inner.host.authorize_command(&job.ctx, &job.body)?;
        fresh(&job.preview.challenge, now)?;
        self.inner
            .host
            .authorize_effects(&job.ctx, &job.preview.challenge.preview)?;
        self.inner
            .host
            .verify_approval(&job.ctx, &job.auth, &job.preview.challenge, now)?;
        let prefix = self.prefix()?;
        if job.body.expected_head != prefix.through {
            return Err(error(ErrorClass::StaleBasis));
        }
        let plan = job.preview.plan;
        let result = CommandResultV1 {
            command_id: job.body.command_id,
            digest,
            state: CommandState::Committed,
            commit: None,
            effects: plan.effects,
            health: vec![Health::Healthy],
        };
        let decision = DecisionEnvelopeV1 {
            id: plan.decision_id,
            commit: CommitSeq(
                prefix
                    .through
                    .0
                    .checked_add(1)
                    .ok_or_else(EngineError::invalid)?,
            ),
            item_position: u32::try_from(
                plan.decision_plan
                    .records
                    .len()
                    .checked_add(1)
                    .ok_or_else(EngineError::invalid)?,
            )
            .map_err(|_| EngineError::invalid())?,
            decision_type: plan.approval_scope.operation,
            schema_version: 1,
            actor: job.ctx.authenticated_client,
            authority: AuthorityKind::Owner,
            grant_id: Some(job.ctx.grant_id),
            command_id: job.body.command_id,
            command_digest: digest,
            references: vec![],
            recorded_time: RecordedInstant(now),
            adopted_result: result.effects.records.clone(),
            policy: plan.policy,
            corrections: vec![],
            supersedes: vec![],
        };
        let store = self.inner.store.read().await;
        let outcome = store
            .as_ref()
            .ok_or_else(|| error(ErrorClass::Locked))?
            .writer()
            .commit(PreparedCommit {
                decision_plan: plan.decision_plan,
                frozen_batch: plan.frozen_batch,
                prior_head: prefix.through,
                result,
                decisions: vec![decision],
                manifest_id: plan.manifest_id,
                control: job.control,
            })
            .await;
        match outcome {
            Ok(receipt) => {
                self.inner
                    .meta
                    .lock()
                    .map_err(|_| error(ErrorClass::IntegrityFailure))?
                    .prefix = Some(AccountingPrefixV1 {
                    through: receipt.commit,
                    domain_digest: receipt.domain_digest,
                    native_head: receipt.native_head,
                });
                Ok(receipt.result)
            }
            Err(CommitFailure::DefinitelyRolledBack(e)) => Err(e),
            Err(CommitFailure::OutcomeUnknown(_)) => {
                let mut meta = self
                    .inner
                    .meta
                    .lock()
                    .map_err(|_| error(ErrorClass::IntegrityFailure))?;
                meta.prefix = None;
                meta.poisoned = true;
                Ok(status(
                    job.body.command_id,
                    digest,
                    CommandState::ResultUnknown,
                ))
            }
        }
    }
    pub async fn command_result(
        &self,
        ctx: RequestContext,
        id: CommandId,
        digest: Digest,
    ) -> Result<CommandResultV1, EngineError> {
        self.authenticate(&ctx, None)?;
        let admissions = self.inner.admissions.lock().await;
        self.authenticate(&ctx, None)?;
        // A recovery boundary must not turn an unaudited missing row into absence.
        if self.prefix().is_err() {
            if let Some(entry) = admissions.get(&id) {
                self.inner.host.authorize_result(&ctx, entry.access())?;
                if entry.digest != digest {
                    return Err(EngineError::invalid());
                }
                return Ok(entry.status(id));
            }
            self.inner
                .host
                .authorize_result(&ctx, ResultAccess::Missing)?;
            return Ok(status(id, digest, CommandState::ResultUnknown));
        }
        let store = self.inner.store.read().await;
        let commit = store
            .as_ref()
            .ok_or_else(|| error(ErrorClass::Locked))?
            .writer()
            .command_record(id)
            .await?;
        self.authenticate(&ctx, None)?;
        if let Some(commit) = commit {
            self.inner
                .host
                .authorize_result(&ctx, ResultAccess::Committed(&commit))?;
            if commit.command_result.digest != digest {
                return Err(EngineError::invalid());
            }
            return Ok(commit.command_result);
        }
        if let Some(entry) = admissions.get(&id) {
            self.inner.host.authorize_result(&ctx, entry.access())?;
            if entry.digest != digest {
                return Err(EngineError::invalid());
            }
            return Ok(entry.status(id));
        }
        self.inner
            .host
            .authorize_result(&ctx, ResultAccess::Missing)?;
        Ok(status(id, digest, CommandState::DefinitelyNotCommitted))
    }
    pub async fn cancel(
        &self,
        ctx: RequestContext,
        id: CommandId,
    ) -> Result<CommandResultV1, EngineError> {
        self.authenticate(&ctx, None)?;
        let mut admissions = self.inner.admissions.lock().await;
        self.authenticate(&ctx, None)?;
        if self.prefix().is_ok() {
            let store = self.inner.store.read().await;
            let commit = store
                .as_ref()
                .ok_or_else(|| error(ErrorClass::Locked))?
                .writer()
                .command_record(id)
                .await?;
            self.authenticate(&ctx, None)?;
            if let Some(commit) = commit {
                self.inner
                    .host
                    .authorize_result(&ctx, ResultAccess::Committed(&commit))?;
                return Ok(commit.command_result);
            }
        }
        if let Some(entry) = admissions.get_mut(&id) {
            self.inner.host.authorize_result(&ctx, entry.access())?;
            if entry.phase == Phase::Queued {
                entry.phase = Phase::Settled;
            }
            entry.control.request_cancel();
            return Ok(entry.status(id));
        }
        self.inner
            .host
            .authorize_result(&ctx, ResultAccess::Missing)?;
        Ok(status(
            id,
            Digest::default(),
            if self.prefix().is_ok() {
                CommandState::DefinitelyNotCommitted
            } else {
                CommandState::ResultUnknown
            },
        ))
    }
    pub async fn query(
        &self,
        ctx: RequestContext,
        query: QueryV1,
    ) -> Result<QueryResponseV1, EngineError> {
        self.authenticate(&ctx, None)?;
        self.inner.host.authorize_query(&ctx, &query)?;
        canonical_bytes(&query)?;
        if query.basis.dataset_id != self.inner.dataset {
            return Err(error(ErrorClass::Unauthorized));
        }
        let _writer = self.inner.serial.lock().await;
        self.authenticate(&ctx, None)?;
        self.inner.host.authorize_query(&ctx, &query)?;
        let prefix = self.prefix()?;
        if query.basis.through_commit != prefix.through {
            return Err(error(ErrorClass::StaleBasis));
        }
        let response = self.inner.planner.query(&query, &prefix)?;
        if response.processed_head != prefix.through || response.basis != query.basis {
            return Err(error(ErrorClass::IntegrityFailure));
        }
        canonical_bytes(&response)?;
        Ok(response)
    }
    /// Await admitted operations, not just the lifetime of their callers.
    async fn reap_finished(&self) -> Result<(), EngineError> {
        let mut tasks = self.inner.tasks.lock().await;
        while let Some(position) = tasks.iter().position(tokio::task::JoinHandle::is_finished) {
            if tasks.swap_remove(position).await.is_err() {
                let mut meta = self
                    .inner
                    .meta
                    .lock()
                    .map_err(|_| error(ErrorClass::IntegrityFailure))?;
                meta.prefix = None;
                meta.poisoned = true;
                return Err(error(ErrorClass::ResultUnknown));
            }
        }
        Ok(())
    }
    pub async fn drain(&self) -> Result<(), EngineError> {
        let tasks = {
            let _admissions = self.inner.admissions.lock().await;
            std::mem::take(&mut *self.inner.tasks.lock().await)
        };
        for task in tasks {
            task.await.map_err(|_| error(ErrorClass::ResultUnknown))?;
        }
        Ok(())
    }
    /// Stop admission and request cancellation, then resolve the writer boundary
    /// and await encrypted connection shutdown. This never undoes a commit.
    pub async fn close(&self) -> Result<(), EngineError> {
        {
            let mut admissions = self.inner.admissions.lock().await;
            self.inner
                .meta
                .lock()
                .map_err(|_| error(ErrorClass::IntegrityFailure))?
                .closing = true;
            self.inner
                .host
                .revoke_sessions(self.inner.dataset, self.inner.instance);
            for entry in admissions.values_mut() {
                if entry.phase == Phase::Queued {
                    entry.phase = Phase::Settled;
                }
                entry.control.request_cancel();
            }
        }
        self.inner
            .previews
            .lock()
            .map_err(|_| error(ErrorClass::IntegrityFailure))?
            .clear();
        self.drain().await?;
        let _writer = self.inner.serial.lock().await;
        if let Some(store) = self.inner.store.write().await.take() {
            store.close().await?;
        }
        Ok(())
    }
    #[cfg(feature = "synthetic-test-fixtures")]
    #[doc(hidden)]
    pub fn set_test_control(
        &self,
        id: CommandId,
        control: Arc<CommitControl>,
    ) -> Result<(), EngineError> {
        self.inner
            .controls
            .lock()
            .map_err(|_| error(ErrorClass::IntegrityFailure))?
            .insert(id, control);
        Ok(())
    }
}

#[cfg(test)]
mod lookup_tests;
