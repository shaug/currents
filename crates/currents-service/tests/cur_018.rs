//! CUR-018 position queries and explanations through the encrypted service.
use currents_domain::position::PositionEngine;
use currents_protocol::{OperationV1, QueryKindV1, QueryOutputV1, QueryV1};
use currents_service as service_api;
use currents_types::*;

#[path = "../../currents-domain/tests/support/recognition.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "support/lifecycle.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/financial_service.rs"]
#[allow(dead_code)]
mod support_financial;
use support_financial::*;

#[tokio::test]
async fn cur_018_4_service_query_uses_one_basis() {
    let dir = tempfile::tempdir().unwrap();
    let (mut f, _) = bootstrap(&dir.path().join("one-basis.db")).await;
    // The test host authorizes queries that name record 700 only, so the
    // component is record 700.
    let component = support::id(700);
    let mut request = fixture::catalog();
    request.revision_id = support::id(702);
    request.catalog.component_id = component;
    request.catalog.account_id = AccountId(support::id(703).0);
    request.ownership.account_id = request.catalog.account_id;
    request.catalog.display_name = "Synthetic position".into();
    submit(
        &mut f,
        56_001,
        OperationV1::SetAccountCatalogComplete(request),
    )
    .await;
    let state = refresh_financial(&mut f, None).await;
    let head = f.prefix().await.through;
    assert_eq!(state.through, head);

    let basis: ReadBasis = serde_json::from_value(serde_json::json!({
        "dataset_id": support::id(1000),
        "through_commit": head,
        "effective_scope": {"start": null, "end": "2026-11-15", "basis": "Effective"},
        "evaluation_instant": "2026-11-15T12:00:00Z",
        "query_semantics_version": 1,
        "selection_policy_version": 1,
        "unit_catalog_version": 1
    }))
    .unwrap();
    let scope = AccountScope {
        accounts: vec![],
        components: vec![component],
    };
    let cases = [
        (
            QueryKindV1::QueryPosition,
            QueryOutputV1::Position(PositionEngine::query(&state, &basis, &scope).unwrap()),
        ),
        (
            QueryKindV1::Explain,
            QueryOutputV1::Explanation(PositionEngine::explain(&state, &basis, component).unwrap()),
        ),
    ];
    for (kind, expected) in cases {
        let query = QueryV1 {
            kind,
            basis: basis.clone(),
            accounts: vec![],
            records: vec![component],
        };
        let response = f.service.query(f.ctx(), query.clone()).await.unwrap();
        assert_eq!(response.basis, basis);
        assert_eq!(response.processed_head, head);
        assert_eq!(response.output, expected);
        // The same query naming the previous commit is refused.
        let mut previous = query;
        previous.basis.through_commit = CommitSeq(head.0 - 1);
        let error = f.service.query(f.ctx(), previous).await.unwrap_err();
        assert_eq!(error.class, ErrorClass::StaleBasis);
    }
}
