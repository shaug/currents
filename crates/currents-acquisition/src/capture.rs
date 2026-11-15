//! Bound untrusted bytes before parsing. No path resolution, network or plaintext quarantine.
use currents_types::*;
pub const MAX_FILE_BYTES: usize = 104_857_600;
pub const MAX_CHUNK_BYTES: usize = 1_048_576;
pub const MAX_OBSERVATIONS: usize = 10_000;
/// Owns bounded input. Rejected bytes never become history or a success receipt.
pub struct BoundedEvidence {
    pub(crate) bytes: Vec<u8>,
    pub(crate) context: ImportContextV1,
}
impl BoundedEvidence {
    pub fn new(name: &str, bytes: Vec<u8>, context: ImportContextV1) -> Result<Self, EngineError> {
        if bytes.len() > MAX_FILE_BYTES
            || name.is_empty()
            || name.len() > 128
            || name == "."
            || name == ".."
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
            || context.row_ids.len() > MAX_OBSERVATIONS
        {
            return Err(EngineError::invalid());
        }
        Ok(Self { bytes, context })
    }
    pub fn chunks(&self) -> impl Iterator<Item = &[u8]> {
        self.bytes.chunks(MAX_CHUNK_BYTES)
    }
}
