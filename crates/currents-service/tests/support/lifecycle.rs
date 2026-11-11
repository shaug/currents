use currents_accounting::AccountingAdapter;
use currents_cipher::{CipherProfileV1, DatasetKey};
use currents_protocol::*;
use currents_service::*;
use currents_storage::EncryptedStore;
use currents_types::*;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
};
#[path = "../../../currents-storage/tests/support/closure.rs"]
#[allow(dead_code)]
mod closure;
pub fn id(n: u128) -> RecordId {
    RecordId(uuid::Uuid::from_u128(n))
}
fn unauthorized() -> EngineError {
    EngineError {
        class: ErrorClass::Unauthorized,
        affected_scope: vec![],
        next_action: SafeNextAction::Authenticate,
    }
}
pub struct Clock(AtomicI64);
impl Clock {
    pub fn advance(&self, n: i64) {
        self.0.fetch_add(n, Ordering::SeqCst);
    }
}
impl TrustedClock for Clock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(self.0.load(Ordering::SeqCst)).unwrap()
    }
}
pub struct Host {
    pub revoked: AtomicBool,
    pub unrelated: AtomicBool,
    session: AtomicU64,
    challenges: Mutex<BTreeMap<CommandId, ApprovalChallenge>>,
}
impl Host {
    fn new() -> Self {
        Self {
            revoked: AtomicBool::new(false),
            unrelated: AtomicBool::new(false),
            session: AtomicU64::new(1),
            challenges: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn corrupt_approval_effects(&self, command: CommandId) {
        self.challenges
            .lock()
            .unwrap()
            .get_mut(&command)
            .unwrap()
            .preview
            .effects
            .records
            .push(id(701));
    }
    pub fn reconnect(&self) {
        self.revoked.store(false, Ordering::SeqCst);
        self.session.fetch_add(1, Ordering::SeqCst);
    }
    fn ctx(&self) -> RequestContext {
        RequestContext {
            authenticated_client: id(800),
            session: SessionId(id(u128::from(self.session.load(Ordering::SeqCst)) + 800).0),
            grant_id: GrantId(id(900).0),
            instance_id: InstanceId(id(999).0),
        }
    }
}
impl HostGrants for Host {
    fn revoke_sessions(&self, _: DatasetId, _: InstanceId) {
        self.revoked.store(true, Ordering::SeqCst);
    }
    fn authenticate(
        &self,
        ctx: &RequestContext,
        dataset: DatasetId,
        capability: Option<&str>,
        now: OffsetDateTime,
    ) -> Result<(), EngineError> {
        if self.revoked.load(Ordering::SeqCst)
            || ctx.session != self.ctx().session
            || dataset != DatasetId(id(1000).0)
            || now.unix_timestamp() >= 1_800_003_600
            || capability.is_some_and(|v| {
                v != format!(
                    "synthetic-capability-{}",
                    self.session.load(Ordering::SeqCst)
                )
            })
        {
            return Err(unauthorized());
        }
        Ok(())
    }
    fn authorize_command(&self, _: &RequestContext, _: &CommandBodyV1) -> Result<(), EngineError> {
        if self.unrelated.load(Ordering::SeqCst) {
            Err(unauthorized())
        } else {
            Ok(())
        }
    }
    fn authorize_effects(&self, _: &RequestContext, _: &PreviewV1) -> Result<(), EngineError> {
        if self.unrelated.load(Ordering::SeqCst) {
            Err(unauthorized())
        } else {
            Ok(())
        }
    }
    fn publish_challenge(&self, c: &ApprovalChallenge) -> Result<(), EngineError> {
        self.challenges
            .lock()
            .unwrap()
            .insert(c.command_id, c.clone());
        Ok(())
    }
    fn verify_approval(
        &self,
        ctx: &RequestContext,
        auth: &AuthorizationEnvelope,
        c: &ApprovalChallenge,
        _: OffsetDateTime,
    ) -> Result<(), EngineError> {
        let saved = self.challenges.lock().unwrap();
        let Some(s) = saved.get(&c.command_id) else {
            return Err(unauthorized());
        };
        if auth.approval.as_deref() != Some(c.nonce.to_string().as_str())
            || c.session != ctx.session
            || s.nonce != c.nonce
            || s.preview != c.preview
        {
            return Err(unauthorized());
        }
        Ok(())
    }
    fn authorize_result(
        &self,
        ctx: &RequestContext,
        access: ResultAccess<'_>,
    ) -> Result<(), EngineError> {
        if self.unrelated.load(Ordering::SeqCst) {
            return Err(unauthorized());
        }
        match access {
            ResultAccess::Committed(c)
                if !c.decisions.iter().any(|d| {
                    d.actor == ctx.authenticated_client && d.grant_id == Some(ctx.grant_id)
                }) =>
            {
                Err(unauthorized())
            }
            ResultAccess::Admitted {
                actor, grant_id, ..
            } if actor != ctx.authenticated_client || grant_id != ctx.grant_id => {
                Err(unauthorized())
            }
            _ => Ok(()),
        }
    }
    fn authorize_query(&self, _: &RequestContext, query: &QueryV1) -> Result<(), EngineError> {
        if self.unrelated.load(Ordering::SeqCst)
            || !query.accounts.is_empty()
            || query.records.iter().any(|v| *v != id(700))
        {
            return Err(unauthorized());
        }
        Ok(())
    }
}
pub struct Planner;
impl CommandPlanner for Planner {
    fn prepare(
        &self,
        body: &CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<CommandPlan, EngineError> {
        if let OperationV1::SetReviewDisposition(request) = &body.operation {
            if prefix.through == CommitSeq(0) {
                return Err(EngineError::invalid());
            }
            let mut state = closure::initial_state(prefix.native_head.clone());
            state.through = prefix.through;
            state.accounting.as_mut().unwrap().prefix = prefix.clone();
            let accounting = AccountingIntent::NoEffect(NoEffectV1 {
                reason: "synthetic review".into(),
            });
            let record = id(body.command_id.0.as_u128() + 100_000);
            return Ok(CommandPlan {
                frozen_batch: AccountingAdapter::freeze(&accounting, &state)?,
                decision_plan: DecisionPlan {
                    records: vec![AuthoritativeRecordV1::Review(ReviewRecordV1 {
                        id: record,
                        targets: request.targets.clone(),
                        disposition: request.disposition,
                        reason: request.reason.clone(),
                    })],
                    invalidations: vec![],
                    accounting,
                },
                effects: EffectsV1 {
                    records: vec![record],
                    accounts: vec![],
                    amounts: vec![],
                    invalidations: vec![],
                },
                warnings: vec![],
                approval_scope: ApprovalScopeV1 {
                    accounts: vec![],
                    operation: DecisionKind::Review,
                    fields: vec!["review".into()],
                },
                decision_id: RecordId(body.command_id.0),
                manifest_id: id(body.command_id.0.as_u128() + 200_000),
                policy: PolicyReferenceV1 {
                    id: id(700),
                    version: 1,
                },
            });
        }
        if !matches!(body.operation, OperationV1::SetAccountCatalog(_))
            || prefix.through != CommitSeq(0)
        {
            return Err(EngineError::version());
        }
        let state = closure::initial_state(prefix.native_head.clone());
        let (frozen_batch, mut records) = closure::metadata(&state);
        records.push(AuthoritativeRecordV1::DatasetPolicy(DatasetPolicyV1 {
            id: id(700),
            version: 1,
            policies: PolicyVersionsV1 {
                selection: 1,
                recognition: 1,
                query: 1,
                calendar: 1,
                timezone_data: "synthetic".into(),
            },
            source_precedence: vec![],
            freshness_seconds: CommitSeq(300),
            opening_policy: id(700),
            withholding_policy: id(700),
        }));
        let accounting = AccountingIntent::MetadataDeclaration(MetadataDeclarationV1 {
            accounts: records
                .iter()
                .filter_map(|r| {
                    if let AuthoritativeRecordV1::AccountComponent(v) = r {
                        Some(v.clone())
                    } else {
                        None
                    }
                })
                .collect(),
            units: records
                .iter()
                .filter_map(|r| {
                    if let AuthoritativeRecordV1::UnitCatalog(v) = r {
                        Some(v.units.clone())
                    } else {
                        None
                    }
                })
                .flatten()
                .collect(),
        });
        assert_eq!(
            AccountingAdapter::freeze(&accounting, &state).unwrap(),
            frozen_batch
        );
        Ok(CommandPlan {
            decision_plan: DecisionPlan {
                records,
                invalidations: vec![id(700)],
                accounting,
            },
            frozen_batch,
            effects: EffectsV1 {
                records: vec![id(700)],
                accounts: vec![AccountId(closure::id(2).0), AccountId(closure::id(3).0)],
                amounts: vec![],
                invalidations: vec![id(700)],
            },
            warnings: vec![],
            approval_scope: ApprovalScopeV1 {
                accounts: vec![AccountId(closure::id(2).0), AccountId(closure::id(3).0)],
                operation: DecisionKind::Catalog,
                fields: vec!["catalog".into()],
            },
            decision_id: RecordId(body.command_id.0),
            manifest_id: id(600),
            policy: PolicyReferenceV1 {
                id: id(700),
                version: 1,
            },
        })
    }
    fn query(
        &self,
        query: &QueryV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<QueryResponseV1, EngineError> {
        if !matches!(query.kind, QueryKindV1::QueryReview) {
            return Err(EngineError::version());
        }
        Ok(QueryResponseV1 {
            processed_head: prefix.through,
            basis: query.basis.clone(),
            health: vec![Health::Healthy],
            output: QueryOutputV1::Review(
                [(id(700), "allowed"), (id(701), "EXCLUDED-REVIEW-CANARY")]
                    .into_iter()
                    .filter(|(record, _)| query.records.contains(record))
                    .map(|(record, reason)| ReviewRecordV1 {
                        id: record,
                        targets: vec![record],
                        disposition: ReviewDisposition::Accepted,
                        reason: reason.into(),
                    })
                    .collect(),
            ),
        })
    }
}
pub struct Fixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    pub service: Service,
    pub host: Arc<Host>,
    pub clock: Arc<Clock>,
}
impl Fixture {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("synthetic.db");
        Self::at_path_with_dir(path, dir, true).await
    }
    pub async fn at_path(path: std::path::PathBuf) -> Self {
        Self::at_path_with_dir(path, tempfile::tempdir().unwrap(), true).await
    }
    pub async fn unrecovered(path: std::path::PathBuf) -> Self {
        Self::at_path_with_dir(path, tempfile::tempdir().unwrap(), false).await
    }
    async fn at_path_with_dir(
        path: std::path::PathBuf,
        dir: tempfile::TempDir,
        recover: bool,
    ) -> Self {
        let host = Arc::new(Host::new());
        let clock = Arc::new(Clock(AtomicI64::new(1_800_000_000)));
        let service = Self::open(
            &path,
            host.clone(),
            clock.clone(),
            ServiceProfile::Synthetic,
        )
        .await
        .unwrap();
        if recover {
            service.recover().await.unwrap();
        }
        Self {
            _dir: dir,
            path,
            service,
            host,
            clock,
        }
    }
    async fn open(
        path: &std::path::Path,
        host: Arc<Host>,
        clock: Arc<Clock>,
        profile: ServiceProfile,
    ) -> Result<Service, EngineError> {
        let store = EncryptedStore::open(path, DatasetKey::new([0x10; 32]), CipherProfileV1)
            .await
            .unwrap();
        Service::new(
            store,
            DatasetId(id(1000).0),
            InstanceId(id(999).0),
            profile,
            host,
            Arc::new(Planner),
            clock,
        )
        .await
    }
    pub async fn reopen(&self) -> Service {
        self.host.reconnect();
        Self::open(
            &self.path,
            self.host.clone(),
            self.clock.clone(),
            ServiceProfile::Synthetic,
        )
        .await
        .unwrap()
    }
    pub async fn production(&self) -> Result<Service, EngineError> {
        let d = tempfile::tempdir().unwrap();
        Self::open(
            &d.path().join("production-refused.db"),
            self.host.clone(),
            self.clock.clone(),
            ServiceProfile::Production,
        )
        .await
    }
    pub fn ctx(&self) -> RequestContext {
        self.host.ctx()
    }
    pub fn body(&self, n: u128) -> CommandBodyV1 {
        CommandBodyV1 {
            protocol_version: ProtocolVersion,
            dataset_id: DatasetId(id(1000).0),
            command_id: CommandId(id(n).0),
            expected_head: CommitSeq(0),
            operation: OperationV1::SetAccountCatalog(SetAccountCatalogV1 {
                account_id: AccountId(closure::id(2).0),
                component_id: closure::id(102),
                account_class: AccountClass::Expense,
                unit_id: UnitId(closure::id(1).0),
                normal_sign: NormalSign::Debit,
                aliases: vec![],
                lifecycle: Lifecycle::Open,
                display_name: "synthetic".into(),
            }),
        }
    }
    pub fn auth(&self, approval: Option<String>) -> AuthorizationEnvelope {
        AuthorizationEnvelope {
            session: self.ctx().session,
            capability: format!(
                "synthetic-capability-{}",
                self.host.session.load(Ordering::SeqCst)
            ),
            approval,
        }
    }
    pub fn approve(&self, b: &CommandBodyV1) -> AuthorizationEnvelope {
        self.service.preview(self.ctx(), b.clone()).unwrap();
        let c = self
            .host
            .challenges
            .lock()
            .unwrap()
            .get(&b.command_id)
            .unwrap()
            .clone();
        self.auth(Some(c.nonce.to_string()))
    }
    pub fn spawn(
        &self,
        b: &CommandBodyV1,
    ) -> tokio::task::JoinHandle<Result<CommandResultV1, EngineError>> {
        let a = self.approve(b);
        let s = self.service.clone();
        let c = self.ctx();
        let b = b.clone();
        tokio::spawn(async move { s.submit(c, b, a).await })
    }
    pub fn query(&self) -> QueryV1 {
        QueryV1 {
            kind: QueryKindV1::QueryReview,
            basis: ReadBasis {
                dataset_id: DatasetId(id(1000).0),
                through_commit: CommitSeq(0),
                effective_scope: DateScope {
                    start: None,
                    end: None,
                    basis: DateBasis::Effective,
                },
                evaluation_instant: self.clock.now(),
                query_semantics_version: 1,
                selection_policy_version: 1,
                unit_catalog_version: 1,
            },
            accounts: vec![],
            records: vec![id(700)],
        }
    }
    pub async fn prefix(&self) -> AccountingPrefixV1 {
        let store = EncryptedStore::open(&self.path, DatasetKey::new([0x10; 32]), CipherProfileV1)
            .await
            .unwrap();
        let result = store.writer().audited_prefix().await.unwrap();
        store.close().await.unwrap();
        result
    }
    pub async fn assert_durable(&self, n: u64, result: &CommandResultV1) {
        let store = EncryptedStore::open(&self.path, DatasetKey::new([0x10; 32]), CipherProfileV1)
            .await
            .unwrap();
        let commits = store.history().read_prefix(CommitSeq(n)).await.unwrap();
        assert_eq!(commits.len(), n as usize);
        assert_eq!(&commits.last().unwrap().command_result, result);
        let bytes = serde_json::to_string(&commits).unwrap();
        assert!(!bytes.contains("synthetic-capability"));
        assert!(!bytes.contains("approval"));
        assert_eq!(commits[0].decisions[0].actor, id(800));
        assert_eq!(
            commits[0].command_result.effects.invalidations,
            vec![id(700)]
        );
        assert!(
            commits[0]
                .records
                .iter()
                .any(|r| matches!(r, AuthoritativeRecordV1::FrozenAccountingManifest(_)))
        );
        store.close().await.unwrap();
    }
}
pub async fn wait_for(condition: impl Fn() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("expected lifecycle point not reached");
}
pub async fn wait_pending(s: &Service, c: RequestContext, id: CommandId, digest: Digest) {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let c = RequestContext {
                authenticated_client: c.authenticated_client,
                session: c.session,
                grant_id: c.grant_id,
                instance_id: c.instance_id,
            };
            if s.command_result(c, id, digest).await.unwrap().state == CommandState::Pending {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("admission not observed");
}
