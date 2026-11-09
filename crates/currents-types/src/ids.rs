//! Identifiers carry type, not economic identity. Creation belongs to acceptance.
use crate::schema::Schema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
macro_rules! ids { ($($name:ident),+ $(,)?) => { $(
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct $name(pub uuid::Uuid);
    impl Schema for $name { fn schema() -> serde_json::Value { serde_json::json!({"type":"string","format":"uuid"}) } }
)+ }; }
ids!(
    DatasetId,
    InstanceId,
    CommandId,
    RecordId,
    AccountId,
    UnitId,
    LegId,
    RecognitionId,
    CommitmentId,
    OccurrenceId,
    GrantId,
    SessionId,
    PairingId
);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitSeq(pub u64);
impl Serialize for CommitSeq {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for CommitSeq {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        if text.is_empty()
            || text.len() > 20
            || !text.bytes().all(|b| b.is_ascii_digit())
            || (text.len() > 1 && text.starts_with('0'))
        {
            return Err(serde::de::Error::custom("invalid sequence"));
        }
        text.parse()
            .map(Self)
            .map_err(|_| serde::de::Error::custom("sequence overflow"))
    }
}
impl Schema for CommitSeq {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$","maxLength":20})
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(pub [u8; 32]);
impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(
            &self
                .0
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
    }
}
impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() != 64
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(serde::de::Error::custom("invalid digest"));
        }
        let mut out = [0; 32];
        for (i, chunk) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
        }
        Ok(Self(out))
    }
}
impl Schema for Digest {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"string","pattern":"^[0-9a-f]{64}$"})
    }
}
