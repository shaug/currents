//! RFC8785 preimages. Bounds apply before deserialization and canonicalization.
use currents_types::{Digest, DomainCommitV1, EngineError};
use serde::{
    Serialize,
    de::{DeserializeOwned, DeserializeSeed},
};
use sha2::{Digest as _, Sha256};
use std::io::{self, Write};
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_DEPTH: usize = 64;
pub const MAX_COLLECTION_ITEMS: usize = 16384;
/// Resource budget selected by a trusted history/closure owner, never a request DTO.
/// This does not establish a production history-envelope limit or change transport admission.
#[derive(Clone, Copy, Debug)]
pub struct CanonicalLimits {
    pub max_bytes: usize,
    pub max_collection_items: usize,
}
const MESSAGE_LIMITS: CanonicalLimits = CanonicalLimits {
    max_bytes: MAX_MESSAGE_BYTES,
    max_collection_items: MAX_COLLECTION_ITEMS,
};
struct Bounded(Vec<u8>, usize);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.1.saturating_sub(self.0.len()) {
            return Err(io::Error::other("message limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// Pure bounded decode. Errors never contain serde's input snippets or caller values.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, EngineError> {
    let value = parse(bytes, MESSAGE_LIMITS)?;
    serde_json::from_value(value).map_err(|_| EngineError::invalid())
}
fn parse(bytes: &[u8], limits: CanonicalLimits) -> Result<serde_json::Value, EngineError> {
    if bytes.len() > limits.max_bytes {
        return Err(EngineError::invalid());
    }
    // Scan structural depth before serde recursion or allocation; ignore delimiters in strings.
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for &b in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                quoted = false;
            }
        } else {
            match b {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth = depth.checked_add(1).ok_or_else(EngineError::invalid)?;
                    if depth > MAX_DEPTH {
                        return Err(EngineError::invalid());
                    }
                }
                b'}' | b']' => {
                    depth = depth.checked_sub(1).ok_or_else(EngineError::invalid)?;
                }
                _ => {}
            }
        }
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = UniqueSeed(limits.max_collection_items)
        .deserialize(&mut deserializer)
        .map_err(|_| EngineError::invalid())?
        .0;
    deserializer.end().map_err(|_| EngineError::invalid())?;
    Ok(value)
}
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, EngineError> {
    canonical_bytes_with_limits(value, MESSAGE_LIMITS)
}
/// Canonicalize pure data under an explicit caller budget; depth and duplicate-key rules remain fixed.
pub fn canonical_bytes_with_limits<T: Serialize>(
    value: &T,
    limits: CanonicalLimits,
) -> Result<Vec<u8>, EngineError> {
    let mut writer = Bounded(Vec::new(), limits.max_bytes);
    serde_json::to_writer(&mut writer, value).map_err(|_| EngineError::invalid())?;
    let value = parse(&writer.0, limits)?;
    let bytes = serde_json_canonicalizer::to_vec(&value).map_err(|_| EngineError::invalid())?;
    if bytes.len() > limits.max_bytes {
        return Err(EngineError::invalid());
    }
    Ok(bytes)
}
/// SHA256(RFC8785(CommandBodyV1)); there is no session/capability input.
pub fn command_digest(body: &crate::CommandBodyV1) -> Result<Digest, EngineError> {
    Ok(Digest(Sha256::digest(canonical_bytes(body)?).into()))
}
/// SHA256 of all DomainCommitV1 fields except its own digest; ordered arrays preserved.
pub fn commit_digest(commit: &DomainCommitV1) -> Result<Digest, EngineError> {
    commit_digest_with_limits(commit, MESSAGE_LIMITS)
}
/// History owner supplies the budget; this is not a large-message transport decoder.
/// Includes the same fields as `commit_digest` and excludes only the commit's own digest.
pub fn commit_digest_with_limits(
    commit: &DomainCommitV1,
    limits: CanonicalLimits,
) -> Result<Digest, EngineError> {
    let mut value = parse(&canonical_bytes_with_limits(commit, limits)?, limits)?;
    value
        .as_object_mut()
        .ok_or_else(EngineError::invalid)?
        .remove("digest");
    Ok(Digest(
        Sha256::digest(canonical_bytes_with_limits(&value, limits)?).into(),
    ))
}
// Duplicate keys are not legal canonical input. Ordinary Value deserialization would lose them.
struct UniqueValue(serde_json::Value);
struct UniqueSeed(usize);
impl<'de> DeserializeSeed<'de> for UniqueSeed {
    type Value = UniqueValue;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(UniqueVisitor(self.0))
    }
}
struct UniqueVisitor(usize);
impl<'de> serde::de::Visitor<'de> for UniqueVisitor {
    type Value = UniqueValue;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("bounded unique JSON")
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(v.into()))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(v.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(v.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(v)
            .map(|v| UniqueValue(v.into()))
            .ok_or_else(|| E::custom("nonfinite"))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(v.into()))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(serde_json::Value::Null))
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
        let mut out = vec![];
        while let Some(v) = a.next_element_seed(UniqueSeed(self.0))? {
            if out.len() >= self.0 {
                return Err(serde::de::Error::custom("collection limit"));
            }
            out.push(v.0);
        }
        Ok(UniqueValue(out.into()))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(key) = a.next_key::<String>()? {
            if out.len() >= self.0 || out.contains_key(&key) {
                return Err(serde::de::Error::custom(
                    "duplicate key or collection limit",
                ));
            }
            out.insert(key, a.next_value_seed(UniqueSeed(self.0))?.0);
        }
        Ok(UniqueValue(out.into()))
    }
}
