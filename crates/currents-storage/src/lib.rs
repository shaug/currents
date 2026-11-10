//! Private storage boundary. Financial behavior is implemented by its owning tickets.
mod writer;
pub use writer::{CommitFailure, Writer};
mod cipher;
mod schema;
pub use cipher::EncryptedStore;
pub use schema::SchemaAttestation;
