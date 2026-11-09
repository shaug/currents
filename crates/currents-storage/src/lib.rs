//! Private storage boundary. Financial behavior is implemented by its owning tickets.
mod writer;
pub use writer::{CommitFailure, Writer};
