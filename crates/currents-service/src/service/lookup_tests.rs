//! Authentication may expire while a cancellation waits for storage access.
use super::*;
use crate::test_support::{Fixture, wait_pending};
use std::{future::Future, pin::pin, sync::atomic::Ordering, task::Context};

#[tokio::test]
async fn missing_cancel_lookup_refreshes_authentication_before_changing_admission() {
    for invalidation in ["revoked", "replaced", "expired"] {
        let f = Fixture::new().await;
        let body = f.body(80);
        // Admit real work, but keep it queued while the cancellation is tested.
        let serial = f.service.inner.serial.lock().await;
        let task = f.spawn(&body);
        wait_pending(
            &f.service,
            f.ctx(),
            body.command_id,
            command_digest(&body).unwrap(),
        )
        .await;

        // The exclusive store guard makes the first poll stop at the awaited
        // storage read, after both initial authentication checks. No sleeps or
        // assumptions about the SQLite worker's scheduling are involved.
        let store = f.service.inner.store.write().await;
        let mut cancel = pin!(f.service.cancel(f.ctx(), body.command_id));
        let mut context = Context::from_waker(std::task::Waker::noop());
        assert!(cancel.as_mut().poll(&mut context).is_pending());
        assert!(f.service.inner.admissions.try_lock().is_err());
        match invalidation {
            "revoked" => f.host.revoked.store(true, Ordering::SeqCst),
            "replaced" => f.host.reconnect(),
            "expired" => f.clock.advance(3_600),
            _ => unreachable!(),
        }
        drop(store);
        let result = cancel.await;
        let admissions = f.service.inner.admissions.lock().await;
        let entry = &admissions[&body.command_id];
        let unchanged = entry.phase == Phase::Queued && entry.control.check_cancelled().is_ok();
        drop(admissions);
        drop(serial);
        let _ = task.await.unwrap();
        f.service.close().await.unwrap();

        assert_eq!(
            result.unwrap_err().class,
            ErrorClass::Unauthorized,
            "{invalidation}"
        );
        assert!(
            unchanged,
            "{invalidation} cancellation changed admitted work"
        );
    }
}
