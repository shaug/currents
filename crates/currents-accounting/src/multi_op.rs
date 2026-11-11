//! Opaque C04 execution input. C05 alone freezes financial instructions.
use crate::PreparedNativeOperation;
use currents_types::{EngineError, ErrorClass, SafeNextAction};
use doubleentry::store::Op;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

pub const MAX_CLOSURE_OPERATIONS: usize = 20_000;
pub const MAX_CLOSURE_BYTES: usize = 64 * 1_048_576;

/// Constructed only by the trusted accounting adapter from validated frozen input.
pub struct PreparedNativeOperations {
    pub(crate) operations: Vec<PreparedNativeOperation>,
    pub(crate) canonical_instructions: Vec<u8>,
}
impl PreparedNativeOperations {
    pub fn validate_limits(&self) -> Result<(), EngineError> {
        if self.canonical_instructions.len() > MAX_CLOSURE_BYTES
            || self.operation_count()? > MAX_CLOSURE_OPERATIONS
        {
            return Err(EngineError::invalid());
        }
        Ok(())
    }
    pub fn operation_count(&self) -> Result<usize, EngineError> {
        self.operations.iter().try_fold(0usize, |n, op| {
            n.checked_add(match &op.operation {
                Op::Batch(items) => items.len(),
                _ => 1,
            })
            .ok_or_else(EngineError::invalid)
        })
    }
    pub fn instruction_bytes(&self) -> &[u8] {
        &self.canonical_instructions
    }
}

impl crate::AccountingAdapter {
    /// Prepare only validated, canonically frozen C05 instructions. Financial
    /// records are partitioned into bounded native batches without reordering.
    pub fn prepare_closure(
        batch: &currents_types::AccountingBatchV1,
    ) -> Result<PreparedNativeOperations, EngineError> {
        let prepared = Self::prepare_operations(batch)?;
        let mut operations = Vec::new();
        let mut records = Vec::new();
        for prepared in prepared {
            match prepared.operation {
                Op::Record(entry) => {
                    records.push(doubleentry::BatchItem::Record(*entry));
                    if records.len() == 600 {
                        operations.push(PreparedNativeOperation {
                            operation: Op::Batch(std::mem::take(&mut records)),
                        });
                    }
                }
                operation => {
                    if !records.is_empty() {
                        operations.push(PreparedNativeOperation {
                            operation: Op::Batch(std::mem::take(&mut records)),
                        });
                    }
                    operations.push(PreparedNativeOperation { operation });
                }
            }
        }
        if !records.is_empty() {
            operations.push(PreparedNativeOperation {
                operation: Op::Batch(records),
            });
        }
        let result = PreparedNativeOperations {
            operations,
            canonical_instructions: Self::instruction_bytes(batch)?,
        };
        result.validate_limits()?;
        Ok(result)
    }
}

/// A cancellation request is not a rollback receipt. Only the writer's completed
/// result can establish rollback; after submission, resolve persisted command status.
#[derive(Default)]
pub struct ClosureControl {
    claimed: AtomicBool,
    phase: AtomicU8,
    completed: AtomicUsize,
    #[cfg(feature = "synthetic-test-fixtures")]
    pub(crate) test_fault: AtomicU8,
    #[cfg(feature = "synthetic-test-fixtures")]
    pub(crate) observed_full: AtomicBool,
    #[cfg(feature = "synthetic-test-fixtures")]
    released: AtomicBool,
    #[cfg(feature = "synthetic-test-fixtures")]
    waker: std::sync::Mutex<Option<std::task::Waker>>,
}
impl ClosureControl {
    /// Returns true if cancellation was requested before COMMIT submission.
    /// Await the writer result before claiming definite rollback.
    pub fn request_cancel(&self) -> bool {
        let requested = self
            .phase
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
            || self.phase.load(Ordering::SeqCst) == 1;
        #[cfg(feature = "synthetic-test-fixtures")]
        if let Ok(waker) = self.waker.lock()
            && let Some(waker) = &*waker
        {
            waker.wake_by_ref();
        }
        requested
    }
    pub fn completed_operations(&self) -> usize {
        self.completed.load(Ordering::SeqCst)
    }
    pub fn claim(&self) -> Result<(), EngineError> {
        if self.claimed.swap(true, Ordering::SeqCst) {
            Err(EngineError::invalid())
        } else {
            Ok(())
        }
    }
    pub fn check_cancelled(&self) -> Result<(), EngineError> {
        if self.phase.load(Ordering::SeqCst) == 1 {
            Err(EngineError {
                class: ErrorClass::Canceled,
                affected_scope: vec![],
                next_action: SafeNextAction::RetrySameCommand,
            })
        } else {
            Ok(())
        }
    }
    pub fn submit_commit(&self) -> Result<(), EngineError> {
        self.phase
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| {
                self.check_cancelled()
                    .err()
                    .unwrap_or_else(EngineError::invalid)
            })
    }
    /// Final rollback-capable boundary. The pause exists only in synthetic builds.
    pub async fn before_commit(&self) -> Result<(), EngineError> {
        #[cfg(feature = "synthetic-test-fixtures")]
        if self.test_fault.load(Ordering::SeqCst) == 5 {
            self.test_pause().await;
        }
        self.check_cancelled()
    }
    pub fn settle(&self) {
        // Preserve submission evidence even after a lost acknowledgment resolves.
        let mut phase = self.phase.load(Ordering::SeqCst);
        while phase != 2 {
            match self
                .phase
                .compare_exchange(phase, 3, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(current) => phase = current,
            }
        }
    }
    pub(crate) fn advance(&self, count: usize) {
        self.completed.fetch_add(count, Ordering::SeqCst);
    }
    #[cfg(feature = "synthetic-test-fixtures")]
    pub(crate) async fn test_pause(&self) {
        std::future::poll_fn(|cx| {
            if self.released.load(Ordering::SeqCst) || self.phase.load(Ordering::SeqCst) == 1 {
                return std::task::Poll::Ready(());
            }
            if let Ok(mut waker) = self.waker.lock() {
                *waker = Some(cx.waker().clone());
            }
            if self.released.load(Ordering::SeqCst) || self.phase.load(Ordering::SeqCst) == 1 {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
        .await;
    }
    #[cfg(feature = "synthetic-test-fixtures")]
    pub(crate) fn test_release(&self) {
        self.released.store(true, Ordering::SeqCst);
        if let Ok(waker) = self.waker.lock()
            && let Some(waker) = &*waker
        {
            waker.wake_by_ref();
        }
    }
    /// True once COMMIT submission won the cancellation race. This is not a
    /// successful commit receipt; lookup/audit still resolves an unknown outcome.
    pub fn commit_submitted(&self) -> bool {
        self.phase.load(Ordering::SeqCst) == 2
    }
}
