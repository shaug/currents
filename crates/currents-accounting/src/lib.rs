//! Private accounting boundary. Financial behavior is implemented by its owning tickets.
mod atomic;
pub use atomic::{AtomicFailure, AtomicReceipt, AtomicStore, PreparedNativeOperation};

mod authority;
pub use authority::{AuthorityTransaction, verify_native_checkpoint};

#[cfg(feature = "synthetic-test-fixtures")]
#[doc(hidden)]
pub mod test_fixtures;

mod batch;
mod catalog;
mod dates;
pub mod numbers;
mod templates;
pub use batch::AccountingAdapter;
