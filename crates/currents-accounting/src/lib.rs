//! Private accounting boundary. Financial behavior is implemented by its owning tickets.
mod atomic;
pub use atomic::{AtomicFailure, AtomicReceipt, AtomicStore, PreparedNativeOperation};

mod authority;
pub use authority::{AuthorityTransaction, verify_native_checkpoint};
