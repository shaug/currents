//! Refreshes a synthetic planner from verified encrypted recovery, without restarting on every fixture step.
use super::support;
use super::support::Fixture;
use super::support_financial::{RelationshipPlanner, open, submit, template_catalog_commands};
use currents_protocol::{CommandBodyV1, QueryResponseV1, QueryV1};
use currents_service::{CommandPlan, CommandPlanner, ImportEvidence, Service, ServiceProfile};
use currents_types::*;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(super) struct FixturePlanner(Option<Arc<PlannerSlot>>);

struct PlannerSlot(Mutex<RelationshipPlanner>);

impl CommandPlanner for PlannerSlot {
    fn prepare(
        &self,
        body: &CommandBodyV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<CommandPlan, EngineError> {
        self.0
            .lock()
            .map_err(|_| EngineError::invalid())?
            .prepare(body, prefix)
    }

    fn query(
        &self,
        query: &QueryV1,
        prefix: &AccountingPrefixV1,
    ) -> Result<QueryResponseV1, EngineError> {
        self.0
            .lock()
            .map_err(|_| EngineError::invalid())?
            .query(query, prefix)
    }
}

impl FixturePlanner {
    pub(super) async fn declare_template(
        &mut self,
        f: &mut Fixture,
        path: &Path,
        template: &DomainState,
    ) {
        for (serial, operation) in template_catalog_commands(template) {
            submit(f, serial, operation).await;
            Box::pin(self.refresh(f, path, None)).await;
        }
    }

    pub(super) async fn refresh(
        &mut self,
        f: &mut Fixture,
        path: &Path,
        evidence: Option<ImportEvidence>,
    ) -> DomainState {
        support::recover_service(&f.service).await;
        let state = f.service.verified_projection().unwrap();
        let planner = RelationshipPlanner {
            state: state.clone(),
            evidence,
        };
        if let Some(slot) = &self.0 {
            // Match the original fixture's authorization/session rotation.
            f.host.reconnect();
            *slot.0.lock().unwrap() = planner;
        } else {
            // Keep the first physical restart and full recovery; it installs
            // the replaceable planner for this fixture, never for another case.
            let slot = Arc::new(PlannerSlot(Mutex::new(planner)));
            f.service.close().await.unwrap();
            f.host.reconnect();
            f.service = Service::new(
                open(path).await,
                DatasetId(support::id(1000).0),
                InstanceId(support::id(999).0),
                ServiceProfile::Synthetic,
                f.host.clone(),
                slot.clone(),
                f.clock.clone(),
            )
            .await
            .unwrap();
            support::recover_service(&f.service).await;
            self.0 = Some(slot);
        }
        let verified = f.service.verified_projection().unwrap();
        assert_eq!(state.through, verified.through);
        assert_eq!(state.domain_digest, verified.domain_digest);
        assert_eq!(
            state.accounting.as_ref().map(|a| &a.prefix),
            verified.accounting.as_ref().map(|a| &a.prefix)
        );
        state
    }
}
