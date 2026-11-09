//! Private accounting boundary. Financial behavior is implemented by its owning tickets.
mod atomic;
pub use atomic::{AtomicFailure, AtomicReceipt, AtomicStore, PreparedNativeOperation};
