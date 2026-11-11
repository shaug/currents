//! CUR010 qualification through the service surface and real encrypted storage.
#[path = "support/lifecycle.rs"]
mod support;
use currents_accounting::test_fixtures::{self, ClosureFault};
use currents_protocol::*;
use currents_service as service_api;
use currents_types::*;
use std::sync::Arc;
use support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cur_010_1() {
    let f = Fixture::new().await;
    let body = f.body(10);
    let digest = command_digest(&body).unwrap();
    let control = Arc::new(test_fixtures::fault_control(
        ClosureFault::LostCommitAcknowledgment,
    ));
    f.service
        .set_test_control(body.command_id, control.clone())
        .unwrap();
    let auth = f.approve(&body);
    let caller = tokio::spawn({
        let s = f.service.clone();
        let b = body.clone();
        let c = f.ctx();
        async move { s.submit(c, b, auth).await }
    });
    wait_for(|| control.commit_submitted()).await;
    caller.abort(); // Lost caller, not cancellation of the admitted writer.
    test_fixtures::release_control(&control);
    f.service.drain().await.unwrap();
    f.service.close().await.unwrap();
    let reopened = f.reopen().await;
    assert_eq!(
        reopened
            .command_result(f.ctx(), body.command_id, digest)
            .await
            .unwrap()
            .state,
        CommandState::ResultUnknown
    );
    reopened.recover().await.unwrap();
    f.host.reconnect(); // Same original scope, new session and fresh capability.
    let retry = reopened
        .submit(f.ctx(), body.clone(), f.auth(None))
        .await
        .unwrap();
    assert_eq!(retry.state, CommandState::Committed);
    assert_eq!(retry.commit, Some(CommitSeq(1)));
    assert_eq!(
        retry,
        reopened
            .command_result(f.ctx(), body.command_id, digest)
            .await
            .unwrap()
    );
    let mut changed = body.clone();
    changed.expected_head = CommitSeq(1);
    assert!(
        reopened
            .submit(f.ctx(), changed, f.auth(None))
            .await
            .is_err()
    );
    f.host
        .unrelated
        .store(true, std::sync::atomic::Ordering::SeqCst);
    for d in [digest, Digest([99; 32])] {
        assert_eq!(
            reopened
                .command_result(f.ctx(), body.command_id, d)
                .await
                .unwrap_err()
                .class,
            ErrorClass::Unauthorized
        );
    }
    assert_eq!(
        reopened
            .cancel(f.ctx(), body.command_id)
            .await
            .unwrap_err()
            .class,
        ErrorClass::Unauthorized
    );
    f.host
        .unrelated
        .store(false, std::sync::atomic::Ordering::SeqCst);
    reopened.close().await.unwrap();
    f.assert_durable(1, &retry).await;
    // Settled committed commands cannot exhaust the bounded in-flight ledger.
    let f = Fixture::new().await;
    let first = f.body(100);
    f.service
        .submit(f.ctx(), first.clone(), f.approve(&first))
        .await
        .unwrap();
    for head in 1..=1025 {
        let mut body = f.body(1000 + u128::from(head));
        body.expected_head = CommitSeq(head);
        body.operation = OperationV1::SetReviewDisposition(ReviewDispositionV1 {
            targets: vec![id(700)],
            disposition: ReviewDisposition::Accepted,
            reason: "synthetic".into(),
        });
        let result = f
            .service
            .submit(f.ctx(), body.clone(), f.approve(&body))
            .await
            .unwrap();
        assert_eq!(result.commit, Some(CommitSeq(head + 1)));
    }
    f.service.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cur_010_2() {
    let f = Fixture::new().await;
    let first = f.body(20);
    let second = f.body(21);
    let held = Arc::new(test_fixtures::fault_control(
        ClosureFault::LostCommitAcknowledgment,
    ));
    f.service
        .set_test_control(first.command_id, held.clone())
        .unwrap();
    let a = f.spawn(&first);
    wait_for(|| held.commit_submitted()).await;
    let b = f.spawn(&second);
    wait_pending(
        &f.service,
        f.ctx(),
        second.command_id,
        command_digest(&second).unwrap(),
    )
    .await;
    assert_eq!(
        f.service
            .command_result(f.ctx(), second.command_id, command_digest(&second).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::Pending
    );
    test_fixtures::release_control(&held);
    assert_eq!(a.await.unwrap().unwrap().state, CommandState::ResultUnknown);
    assert!(b.await.unwrap().is_err());
    f.service.close().await.unwrap();
    let reopened = f.reopen().await;
    assert_eq!(
        reopened
            .command_result(f.ctx(), second.command_id, command_digest(&second).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::ResultUnknown
    );
    reopened.recover().await.unwrap();
    assert_eq!(
        reopened
            .command_result(f.ctx(), second.command_id, command_digest(&second).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::DefinitelyNotCommitted
    );
    assert_eq!(
        reopened
            .command_result(f.ctx(), first.command_id, command_digest(&first).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::Committed
    );
    reopened.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cur_010_3() {
    let f = Fixture::new().await;
    let first = f.body(30);
    let other = f.body(31);
    let a = f.approve(&first);
    let b = f.approve(&other);
    let (x, y) = tokio::join!(
        f.service.submit(f.ctx(), first, a),
        f.service.submit(f.ctx(), other, b)
    );
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    assert_eq!(x.err().or(y.err()).unwrap().class, ErrorClass::StaleBasis);
    f.service.close().await.unwrap();
    // A queued command must sample current revocation and clock, not admission time.
    for expire in [false, true] {
        let f = Fixture::new().await;
        let held = Arc::new(test_fixtures::fault_control(
            ClosureFault::LostCommitAcknowledgment,
        ));
        let first = f.body(32);
        f.service
            .set_test_control(first.command_id, held.clone())
            .unwrap();
        let a = f.spawn(&first);
        wait_for(|| held.commit_submitted()).await;
        let second = f.body(33);
        let b = f.spawn(&second);
        wait_pending(
            &f.service,
            f.ctx(),
            second.command_id,
            command_digest(&second).unwrap(),
        )
        .await;
        if expire {
            f.clock.advance(301);
        } else {
            f.host
                .revoked
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        test_fixtures::release_control(&held);
        let _ = a.await.unwrap();
        assert_eq!(
            b.await.unwrap().unwrap_err().class,
            ErrorClass::Unauthorized
        );
        f.service.close().await.unwrap();
    }
    for mismatch in 0..5 {
        let f = Fixture::new().await;
        let mut b = f.body(40);
        let mut a = f.approve(&b);
        match mismatch {
            0 => {
                let OperationV1::SetAccountCatalog(v) = &mut b.operation else {
                    panic!()
                };
                v.display_name = "changed-body".into();
            }
            1 => {
                f.host.corrupt_approval_effects(b.command_id);
            }
            2 => {
                a.session = SessionId(id(777).0);
            }
            3 => {
                f.clock.advance(300);
            } // Expiry boundary is exclusive.
            _ => {
                f.host.reconnect();
                a = f.auth(a.approval);
            }
        }
        assert!(f.service.submit(f.ctx(), b, a).await.is_err());
        f.service.close().await.unwrap();
    }
    let f = Fixture::new().await;
    assert!(f.production().await.is_err());
    let query = f.query();
    let response = f.service.query(f.ctx(), query.clone()).await.unwrap();
    assert_eq!(response.basis, query.basis);
    let bytes = canonical_bytes(&response).unwrap();
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("EXCLUDED-REVIEW-CANARY")
    );
    let QueryOutputV1::Review(rows) = &response.output else {
        panic!("wrong output")
    };
    assert_eq!(rows.len(), 1);
    let mut forbidden = query.clone();
    forbidden.records.push(id(701));
    assert_eq!(
        f.service.query(f.ctx(), forbidden).await.unwrap_err().class,
        ErrorClass::Unauthorized
    );
    f.host
        .unrelated
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        f.service.query(f.ctx(), query).await.unwrap_err().class,
        ErrorClass::Unauthorized
    );
    f.service.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cur_010_4() {
    if let Some(path) = std::env::var_os("CUR010_CRASH_PATH") {
        let path = std::path::PathBuf::from(path);
        let f = Fixture::at_path(path.clone()).await;
        let body = f.body(70);
        let control = Arc::new(test_fixtures::fault_control(
            ClosureFault::LostCommitAcknowledgment,
        ));
        f.service
            .set_test_control(body.command_id, control.clone())
            .unwrap();
        let _task = f.spawn(&body);
        wait_for(|| control.commit_submitted()).await;
        std::fs::write(path.with_extension("submitted"), b"submitted").unwrap();
        std::future::pending::<()>().await;
        return;
    }
    let f = Fixture::new().await;
    let first = f.body(50);
    let queued = f.body(51);
    let held = Arc::new(test_fixtures::fault_control(
        ClosureFault::LostCommitAcknowledgment,
    ));
    f.service
        .set_test_control(first.command_id, held.clone())
        .unwrap();
    let a = f.spawn(&first);
    wait_for(|| held.commit_submitted()).await;
    let b = f.spawn(&queued);
    wait_pending(
        &f.service,
        f.ctx(),
        queued.command_id,
        command_digest(&queued).unwrap(),
    )
    .await;
    assert_eq!(
        f.service
            .cancel(f.ctx(), queued.command_id)
            .await
            .unwrap()
            .state,
        CommandState::DefinitelyNotCommitted
    );
    assert_eq!(
        f.service
            .cancel(f.ctx(), first.command_id)
            .await
            .unwrap()
            .state,
        CommandState::Pending
    );
    test_fixtures::release_control(&held);
    assert_eq!(a.await.unwrap().unwrap().state, CommandState::ResultUnknown);
    assert_eq!(
        b.await.unwrap().unwrap().state,
        CommandState::DefinitelyNotCommitted
    );
    f.service.close().await.unwrap();
    let reopened = f.reopen().await;
    reopened.recover().await.unwrap();
    assert_eq!(
        reopened
            .cancel(f.ctx(), first.command_id)
            .await
            .unwrap()
            .state,
        CommandState::Committed
    );
    assert_eq!(
        reopened
            .command_result(f.ctx(), queued.command_id, command_digest(&queued).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::DefinitelyNotCommitted
    );
    reopened.close().await.unwrap();
    // A real precommit native closure rolls back all native/domain/result effects.
    let f = Fixture::new().await;
    let initial = f.prefix().await;
    let body = f.body(52);
    let control = Arc::new(test_fixtures::fault_control(
        ClosureFault::PauseBeforeCommit,
    ));
    f.service
        .set_test_control(body.command_id, control.clone())
        .unwrap();
    let task = f.spawn(&body);
    wait_for(|| control.completed_operations() == 5).await;
    assert_eq!(
        f.service
            .cancel(f.ctx(), body.command_id)
            .await
            .unwrap()
            .state,
        CommandState::Pending
    );
    let result = task.await.unwrap();
    assert_eq!(result.unwrap_err().class, ErrorClass::Canceled);
    assert_eq!(
        f.service
            .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::DefinitelyNotCommitted
    );
    f.service.close().await.unwrap();
    let reopened = f.reopen().await;
    reopened.recover().await.unwrap();
    assert_eq!(
        reopened
            .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::DefinitelyNotCommitted
    );
    reopened.close().await.unwrap();
    assert_eq!(
        f.prefix().await,
        initial,
        "rollback must preserve the exact native/domain prefix"
    );
    // Lock/close cancels queued work and drains a submitted COMMIT before close.
    for submitted in [false, true] {
        let f = Fixture::new().await;
        let first = f.body(60);
        let queued = f.body(61);
        let control = Arc::new(test_fixtures::fault_control(if submitted {
            ClosureFault::LostCommitAcknowledgment
        } else {
            ClosureFault::PauseBeforeCommit
        }));
        f.service
            .set_test_control(first.command_id, control.clone())
            .unwrap();
        let a = f.spawn(&first);
        wait_for(|| {
            if submitted {
                control.commit_submitted()
            } else {
                control.completed_operations() == 5
            }
        })
        .await;
        let b = f.spawn(&queued);
        wait_pending(
            &f.service,
            f.ctx(),
            queued.command_id,
            command_digest(&queued).unwrap(),
        )
        .await;
        let close = tokio::spawn({
            let service = f.service.clone();
            async move { service.close().await }
        });
        wait_for(|| f.host.revoked.load(std::sync::atomic::Ordering::SeqCst)).await;
        if submitted {
            assert!(!close.is_finished());
            test_fixtures::release_control(&control);
        }
        close.await.unwrap().unwrap();
        let result = a.await.unwrap();
        if submitted {
            assert_eq!(result.unwrap().state, CommandState::ResultUnknown);
        } else {
            assert_eq!(result.unwrap_err().class, ErrorClass::Canceled);
        }
        assert_eq!(
            b.await.unwrap().unwrap().state,
            CommandState::DefinitelyNotCommitted
        );
        assert_eq!(
            f.service
                .command_result(f.ctx(), first.command_id, command_digest(&first).unwrap())
                .await
                .unwrap_err()
                .class,
            ErrorClass::Locked
        );
        let reopened = f.reopen().await;
        reopened.recover().await.unwrap();
        assert_eq!(
            reopened
                .command_result(f.ctx(), first.command_id, command_digest(&first).unwrap())
                .await
                .unwrap()
                .state,
            if submitted {
                CommandState::Committed
            } else {
                CommandState::DefinitelyNotCommitted
            }
        );
        reopened.close().await.unwrap();
    }
    // Process death after COMMIT submission may resolve to absence, never assume
    // that result until the reopened encrypted authority audit has completed.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("killed.db");
    let output = std::fs::File::create(directory.path().join("child.log")).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cur_010_4", "--nocapture"])
        .env("CUR010_CRASH_PATH", &path)
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn()
        .unwrap();
    wait_for(|| path.with_extension("submitted").exists()).await;
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let f = Fixture::unrecovered(path).await;
    let body = f.body(70);
    assert_eq!(
        f.service
            .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::ResultUnknown
    );
    f.service.recover().await.unwrap();
    let body = f.body(70);
    assert_eq!(
        f.service
            .command_result(f.ctx(), body.command_id, command_digest(&body).unwrap())
            .await
            .unwrap()
            .state,
        CommandState::DefinitelyNotCommitted
    );
    assert_eq!(f.prefix().await.through, CommitSeq(0));
    f.service.close().await.unwrap();
}
