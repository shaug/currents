//! Full frozen reconstruction into an explicit trusted-host destination.
use currents_storage::{EncryptedStore, HistoryReader, ReplayReport};
use currents_types::*;
pub struct ReplayCoordinator;
impl ReplayCoordinator {
    /// Consumes and explicitly closes the destination on success and failure.
    /// A failure leaves that disposable destination untrusted, never activated.
    pub async fn rebuild(
        source: HistoryReader,
        destination: EncryptedStore,
    ) -> Result<ReplayReport, EngineError> {
        let result = Self::rebuild_inner(&source, &destination)
            .await
            .map(|(report, _)| report);
        let closed = destination.close().await;
        match result {
            Ok(report) => {
                closed?;
                Ok(report)
            }
            Err(error) => {
                let _ = closed;
                Err(error)
            }
        }
    }
    pub(crate) async fn rebuild_inner(
        source: &HistoryReader,
        destination: &EncryptedStore,
    ) -> Result<(ReplayReport, DomainState), EngineError> {
        let before = source.replay_evidence().await?;
        let empty = destination.writer().audited_prefix().await?;
        if empty.through != CommitSeq(0) {
            return Err(EngineError::invalid());
        }
        let mut state = DomainState {
            accounting: Some(AccountingIndexV1::empty(empty.native_head)),
            ..DomainState::default()
        };
        let mut stream = source.stream_prefix(before.through).await?;
        while let Some(commit) = stream.next().await? {
            // Verify descriptive typed rows against the actual authenticated raw
            // object before any destination publication or projection activation.
            for record in &commit.records {
                if let AuthoritativeRecordV1::RelationshipParseRevision(p) = record {
                    let receipt = commit
                        .records
                        .iter()
                        .find_map(|r| match r {
                            AuthoritativeRecordV1::ImportReceipt(r) if r.id == p.receipt_id => {
                                Some(r)
                            }
                            _ => None,
                        })
                        .ok_or_else(EngineError::invalid)?;
                    if p.profile.byte_count.0 > 1_048_576 {
                        return Err(EngineError::invalid());
                    }
                    let mut content = source.stream_content(commit.seq, p.profile.digest).await?;
                    let mut bytes = Vec::new();
                    while let Some(chunk) = content.next().await? {
                        if bytes.len().saturating_add(chunk.len()) > 1_048_576 {
                            return Err(EngineError::invalid());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    currents_domain::relationship_parse::ValidatedRelationshipParseV1::new(
                        RelationshipParseBatchV1 {
                            receipt: receipt.clone(),
                            revision: p.clone(),
                            contents: vec![EvidenceContentV1 {
                                digest: p.profile.digest,
                                bytes,
                            }],
                        },
                    )?;
                }
                if let AuthoritativeRecordV1::RelationshipCapture(capture) = record {
                    let receipt = commit
                        .records
                        .iter()
                        .find_map(|r| match r {
                            AuthoritativeRecordV1::ImportReceipt(r)
                                if r.id == capture.receipt_id =>
                            {
                                Some(r)
                            }
                            _ => None,
                        })
                        .ok_or_else(EngineError::invalid)?;
                    if capture.raw.byte_count.0 > 1_048_576 {
                        return Err(EngineError::invalid());
                    }
                    let mut content = source
                        .stream_content(commit.seq, capture.raw.digest)
                        .await?;
                    let mut bytes = Vec::new();
                    while let Some(chunk) = content.next().await? {
                        if bytes.len().saturating_add(chunk.len()) > 1_048_576 {
                            return Err(EngineError::invalid());
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    currents_domain::relationship_evidence::ValidatedRelationshipCaptureV1::new(
                        RelationshipCaptureBatchV1 {
                            receipt: receipt.clone(),
                            capture: capture.clone(),
                            contents: vec![EvidenceContentV1 {
                                digest: capture.raw.digest,
                                bytes,
                            }],
                        },
                    )?;
                }
            }
            let mut projection = state.clone();
            currents_domain::ReplayEngine::apply(&mut projection, &commit)?;
            let next = source.rebuild_commit(destination, &state, &commit).await?;
            projection.accounting = Some(next);
            currents_domain::recognition::RecognitionEngine::audit(&projection)?;
            currents_domain::associations::IdentityEngine::audit(&projection)?;
            state = projection;
        }
        let mut after = destination.history().audit_report().await?;
        after.problems = before.problems.clone();
        if !before.matches(&after) || source.replay_evidence().await? != before {
            return Err(EngineError {
                class: ErrorClass::IntegrityFailure,
                affected_scope: vec![],
                next_action: SafeNextAction::Recover,
            });
        }
        Ok((after, state))
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::Fixture;
    use currents_types::ErrorClass;
    #[tokio::test]
    async fn startup_requires_full_destination() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture::unrecovered(dir.path().join("source.db")).await;
        let error = fixture.service.recover().await.unwrap_err();
        assert_eq!(error.class, ErrorClass::IncompleteBasis);
        fixture.service.close().await.unwrap();
    }
}

#[cfg(test)]
mod independence {
    use super::*;
    use crate::{
        CommandPlan, CommandPlanner, Service, ServiceProfile,
        test_support::{Fixture, id, recover_service},
    };
    use currents_cipher::{CipherProfileV1, DatasetKey};
    use currents_protocol::*;
    use std::sync::Arc;
    struct UnavailableAmbient;
    impl TrustedClock for UnavailableAmbient {
        fn now(&self) -> OffsetDateTime {
            panic!("replay must not read the clock")
        }
    }
    impl CommandPlanner for UnavailableAmbient {
        fn prepare(
            &self,
            _: &CommandBodyV1,
            _: &AccountingPrefixV1,
        ) -> Result<CommandPlan, EngineError> {
            panic!("replay must not apply current command rules")
        }
        fn query(
            &self,
            _: &QueryV1,
            _: &AccountingPrefixV1,
        ) -> Result<QueryResponseV1, EngineError> {
            panic!("replay must not ask current query policy")
        }
    }
    #[tokio::test]
    async fn replay_never_consults_current_clock_or_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.db");
        let fixture = Fixture::at_path(path.clone()).await;
        let body = fixture.body(10);
        fixture
            .service
            .submit(fixture.ctx(), body.clone(), fixture.approve(&body))
            .await
            .unwrap();
        fixture.service.close().await.unwrap();
        let store = EncryptedStore::open(&path, DatasetKey::new([0x10; 32]), CipherProfileV1)
            .await
            .unwrap();
        let service = Service::new(
            store,
            DatasetId(id(1000).0),
            InstanceId(id(999).0),
            ServiceProfile::Synthetic,
            fixture.host.clone(),
            Arc::new(UnavailableAmbient),
            Arc::new(UnavailableAmbient),
        )
        .await
        .unwrap();
        recover_service(&service).await;
        assert_eq!(service.replay_health().unwrap(), Health::Healthy);
        service.close().await.unwrap();
    }
    #[tokio::test]
    async fn current_queries_require_content_verified_shadow() {
        let fixture = Fixture::new().await;
        assert!(
            fixture
                .service
                .verified_projection()
                .unwrap()
                .records
                .is_empty()
        );
        let body = fixture.body(91);
        fixture
            .service
            .preview(fixture.ctx(), body.clone())
            .unwrap();
        fixture
            .service
            .submit(fixture.ctx(), body.clone(), fixture.approve(&body))
            .await
            .unwrap();
        assert_eq!(
            fixture.service.verified_projection().unwrap_err().class,
            ErrorClass::IncompleteBasis
        );
        let mut query = fixture.query();
        query.basis.through_commit = CommitSeq(1);
        assert_eq!(
            fixture
                .service
                .query(fixture.ctx(), query.clone())
                .await
                .unwrap_err()
                .class,
            ErrorClass::IncompleteBasis
        );
        recover_service(&fixture.service).await;
        let projection = fixture.service.verified_projection().unwrap();
        let Some(AuthoritativeRecordV1::DatasetPolicy(policy)) = projection.records.get(&id(700))
        else {
            panic!("rebuilt projection must contain accepted policy content");
        };
        assert_eq!(policy.freshness_seconds, CommitSeq(300));
        assert_eq!(policy.policies.timezone_data, "synthetic");
        assert!(!projection.records.is_empty());
        fixture.service.query(fixture.ctx(), query).await.unwrap();
        fixture.service.close().await.unwrap();
    }
    #[tokio::test]
    async fn unsupported_semantics_never_enable_financial_commands() {
        let fixture = Fixture::new().await;
        let dir = tempfile::tempdir().unwrap();
        let source = EncryptedStore::open(
            &dir.path().join("source.db"),
            DatasetKey::new([0x22; 32]),
            CipherProfileV1,
        )
        .await
        .unwrap();
        source
            .writer()
            .append(currents_storage::HistoryAppend {
                prior_head: CommitSeq(0),
                records: vec![AuthoritativeRecordV1::Association(AssociationV1 {
                    id: id(50000),
                    kind: RelationshipKind::Transfer,
                    legs: vec![],
                    allocations: vec![],
                    survivor: None,
                    corrections: vec![],
                    reason: "unimplemented association semantics".into(),
                })],
                decisions: vec![],
                staged: vec![],
                native: None,
                result: CommandResultV1 {
                    command_id: CommandId(id(50001).0),
                    digest: Digest([2; 32]),
                    state: CommandState::Committed,
                    commit: None,
                    effects: EffectsV1 {
                        records: vec![],
                        accounts: vec![],
                        amounts: vec![],
                        invalidations: vec![],
                    },
                    health: vec![],
                },
            })
            .await
            .unwrap();
        let service = Service::new(
            source,
            DatasetId(id(1000).0),
            InstanceId(id(999).0),
            ServiceProfile::Synthetic,
            fixture.host.clone(),
            Arc::new(UnavailableAmbient),
            fixture.clock.clone(),
        )
        .await
        .unwrap();
        let destination = EncryptedStore::open(
            &dir.path().join("destination.db"),
            DatasetKey::new([0x23; 32]),
            CipherProfileV1,
        )
        .await
        .unwrap();
        let error = service.recover_into(destination).await.unwrap_err();
        assert_eq!(error.class, ErrorClass::UnsupportedVersion);
        assert_eq!(service.replay_health().unwrap(), Health::MigrationRequired);
        assert_eq!(
            service
                .preview(fixture.ctx(), fixture.body(90))
                .unwrap_err()
                .class,
            ErrorClass::ResultUnknown
        );
        service.close().await.unwrap();
        let readonly = currents_storage::ReadOnlyStore::open(
            &dir.path().join("source.db"),
            DatasetKey::new([0x22; 32]),
            CipherProfileV1,
        )
        .await
        .unwrap();
        let forensic = readonly.into_forensic(error);
        assert_eq!(forensic.failure().class, ErrorClass::UnsupportedVersion);
        forensic.close().await;
        fixture.service.close().await.unwrap();
    }
}
