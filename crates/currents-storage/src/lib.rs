//! Private storage boundary. Financial behavior is implemented by its owning tickets.
mod writer;
pub use writer::{CommitFailure, Writer};
mod cipher;
mod schema;
pub use cipher::EncryptedStore;
pub use schema::SchemaAttestation;
mod history;
mod references;
mod staging;
pub use history::{ContentStream, HistoryAppend, HistoryReader, HistoryStream};
pub use staging::StagedBatchHandle;
