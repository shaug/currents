//! RFC8785 preimages. Bounds apply before deserialization and canonicalization.
use currents_types::{Digest, DomainCommitV1, EngineError};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use std::io::{self, Write};
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_DEPTH: usize = 64;
pub const MAX_COLLECTION_ITEMS: usize = 16384;
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_MESSAGE_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("message limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn check(value: &serde_json::Value, depth: usize) -> Result<(), EngineError> {
    if depth > MAX_DEPTH {
        return Err(EngineError::invalid());
    }
    match value {
        serde_json::Value::Array(a) => {
            if a.len() > MAX_COLLECTION_ITEMS {
                return Err(EngineError::invalid());
            }
            for v in a {
                check(v, depth + 1)?;
            }
        }
        serde_json::Value::Object(m) => {
            if m.len() > MAX_COLLECTION_ITEMS {
                return Err(EngineError::invalid());
            }
            for v in m.values() {
                check(v, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Pure bounded decode. Errors never contain serde's input snippets or caller values.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, EngineError> {
    let value = parse(bytes)?;
    serde_json::from_value(value).map_err(|_| EngineError::invalid())
}
fn parse(bytes: &[u8]) -> Result<serde_json::Value, EngineError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
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
                    depth += 1;
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
    let value = serde_json::from_slice::<UniqueValue>(bytes)
        .map_err(|_| EngineError::invalid())?
        .0;
    check(&value, 0)?;
    Ok(value)
}
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, EngineError> {
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, value).map_err(|_| EngineError::invalid())?;
    let value = parse(&writer.0)?;
    let bytes = serde_json_canonicalizer::to_vec(&value).map_err(|_| EngineError::invalid())?;
    if bytes.len() > MAX_MESSAGE_BYTES {
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
    let mut value = parse(&canonical_bytes(commit)?)?;
    value
        .as_object_mut()
        .ok_or_else(EngineError::invalid)?
        .remove("digest");
    Ok(Digest(Sha256::digest(canonical_bytes(&value)?).into()))
}
// Duplicate keys are not legal canonical input. Ordinary Value deserialization would lose them.
struct UniqueValue(serde_json::Value);
impl<'de> serde::Deserialize<'de> for UniqueValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(UniqueVisitor)
    }
}
struct UniqueVisitor;
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
        while let Some(v) = a.next_element::<UniqueValue>()? {
            if out.len() >= MAX_COLLECTION_ITEMS {
                return Err(serde::de::Error::custom("collection limit"));
            }
            out.push(v.0);
        }
        Ok(UniqueValue(out.into()))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
        let mut out = serde_json::Map::new();
        while let Some(key) = a.next_key::<String>()? {
            if out.len() >= MAX_COLLECTION_ITEMS || out.contains_key(&key) {
                return Err(serde::de::Error::custom(
                    "duplicate key or collection limit",
                ));
            }
            out.insert(key, a.next_value::<UniqueValue>()?.0);
        }
        Ok(UniqueValue(out.into()))
    }
}
