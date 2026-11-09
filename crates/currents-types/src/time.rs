//! Explicit source precision; no ambient timezone or clock lookups.
use crate::{CommitSeq, DatasetId, choices, schema::Schema};
use ::time::format_description::well_known::Rfc3339;
pub use ::time::{Date, OffsetDateTime};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
choices!(TemporalPrecision {
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
    Unknown
});
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalValue {
    Date {
        civil: Date,
        original_text: String,
    },
    Instant {
        utc: OffsetDateTime,
        original_offset: Option<String>,
        original_zone: Option<String>,
        precision: TemporalPrecision,
        original_text: String,
    },
    Unknown {
        original_text: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Wire {
    Date {
        civil: String,
        original_text: String,
    },
    Instant {
        utc: String,
        #[serde(deserialize_with = "crate::schema::required")]
        original_offset: Option<String>,
        #[serde(deserialize_with = "crate::schema::required")]
        original_zone: Option<String>,
        precision: TemporalPrecision,
        original_text: String,
    },
    Unknown {
        original_text: String,
    },
}
fn date_format() -> Vec<::time::format_description::BorrowedFormatItem<'static>> {
    ::time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
        .expect("constant date format")
}
impl Serialize for TemporalValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let wire = match self {
            Self::Date {
                civil,
                original_text,
            } => Wire::Date {
                civil: civil
                    .format(&date_format())
                    .map_err(serde::ser::Error::custom)?,
                original_text: original_text.clone(),
            },
            Self::Instant {
                utc,
                original_offset,
                original_zone,
                precision,
                original_text,
            } => Wire::Instant {
                utc: utc
                    .to_offset(::time::UtcOffset::UTC)
                    .format(&Rfc3339)
                    .map_err(serde::ser::Error::custom)?,
                original_offset: original_offset.clone(),
                original_zone: original_zone.clone(),
                precision: *precision,
                original_text: original_text.clone(),
            },
            Self::Unknown { original_text } => Wire::Unknown {
                original_text: original_text.clone(),
            },
        };
        wire.serialize(s)
    }
}
impl<'de> Deserialize<'de> for TemporalValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Wire::deserialize(d)? {
            Wire::Date {
                civil,
                original_text,
            } => Self::Date {
                civil: Date::parse(&civil, &date_format()).map_err(serde::de::Error::custom)?,
                original_text,
            },
            Wire::Instant {
                utc,
                original_offset,
                original_zone,
                precision,
                original_text,
            } => {
                let utc =
                    OffsetDateTime::parse(&utc, &Rfc3339).map_err(serde::de::Error::custom)?;
                if utc.offset() != ::time::UtcOffset::UTC {
                    return Err(serde::de::Error::custom("utc must use zero offset"));
                }
                Self::Instant {
                    utc,
                    original_offset,
                    original_zone,
                    precision,
                    original_text,
                }
            }
            Wire::Unknown { original_text } => Self::Unknown { original_text },
        })
    }
}
impl Schema for TemporalValue {
    fn schema() -> serde_json::Value {
        use serde_json::json;
        let string = json!({"type":"string"});
        json!({"oneOf":[
         {"type":"object","additionalProperties":false,"required":["kind","civil","original_text"],"properties":{"kind":{"const":"Date"},"civil":{"type":"string","format":"date"},"original_text":string}},
         {"type":"object","additionalProperties":false,"required":["kind","utc","original_offset","original_zone","precision","original_text"],"properties":{"kind":{"const":"Instant"},"utc":{"type":"string","format":"date-time"},"original_offset":Option::<String>::schema(),"original_zone":Option::<String>::schema(),"precision":TemporalPrecision::schema(),"original_text":string}},
         {"type":"object","additionalProperties":false,"required":["kind","original_text"],"properties":{"kind":{"const":"Unknown"},"original_text":string}}
        ]})
    }
}
/// Explicit states are not `Option<T>`: absence, a missing value, null and deletion differ.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", deny_unknown_fields)]
pub enum SourceValue<T> {
    Missing,
    Unknown,
    Null,
    Absent,
    Deleted,
    Known(T),
}
impl<T: Schema> Schema for SourceValue<T> {
    fn schema() -> serde_json::Value {
        use serde_json::json;
        let mut variants = vec![];
        for state in ["Missing", "Unknown", "Null", "Absent", "Deleted"] {
            variants.push(json!({"type":"object","additionalProperties":false,"required":["state"],"properties":{"state":{"const":state}}}));
        }
        variants.push(json!({"type":"object","additionalProperties":false,"required":["state","value"],"properties":{"state":{"const":"Known"},"value":T::schema()}}));
        json!({"oneOf":variants})
    }
}
choices!(DateBasis { Effective, Posting });
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DateScope {
    #[serde(with = "optional_date")]
    pub start: Option<Date>,
    #[serde(with = "optional_date")]
    pub end: Option<Date>,
    pub basis: DateBasis,
}
mod optional_date {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Option<Date>, s: S) -> Result<S::Ok, S::Error> {
        v.map(|v| v.format(&date_format()))
            .transpose()
            .map_err(serde::ser::Error::custom)?
            .serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Date>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|v| Date::parse(&v, &date_format()))
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
impl Schema for DateScope {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["start","end","basis"],"properties":{"start":{"type":["string","null"],"format":"date"},"end":{"type":["string","null"],"format":"date"},"basis":DateBasis::schema()}})
    }
}
/// An accepted instant, encoded as RFC3339 UTC, not an inferred source date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedInstant(pub OffsetDateTime);
impl Serialize for RecordedInstant {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0
            .to_offset(::time::UtcOffset::UTC)
            .format(&Rfc3339)
            .map_err(serde::ser::Error::custom)?
            .serialize(s)
    }
}
impl<'de> Deserialize<'de> for RecordedInstant {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let t = OffsetDateTime::parse(&String::deserialize(d)?, &Rfc3339)
            .map_err(serde::de::Error::custom)?;
        if t.offset() != ::time::UtcOffset::UTC {
            return Err(serde::de::Error::custom("instant must use UTC"));
        }
        Ok(Self(t))
    }
}
impl Schema for RecordedInstant {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"string","format":"date-time"})
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadBasis {
    pub dataset_id: DatasetId,
    pub through_commit: CommitSeq,
    pub effective_scope: DateScope,
    #[serde(with = "instant")]
    pub evaluation_instant: OffsetDateTime,
    pub query_semantics_version: u32,
    pub selection_policy_version: u32,
    pub unit_catalog_version: u32,
}
pub mod instant {
    use super::*;
    pub fn serialize<S: Serializer>(v: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
        RecordedInstant(*v).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OffsetDateTime, D::Error> {
        RecordedInstant::deserialize(d).map(|v| v.0)
    }
}
impl Schema for ReadBasis {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["dataset_id","through_commit","effective_scope","evaluation_instant","query_semantics_version","selection_policy_version","unit_catalog_version"],"properties":{"dataset_id":DatasetId::schema(),"through_commit":CommitSeq::schema(),"effective_scope":DateScope::schema(),"evaluation_instant":RecordedInstant::schema(),"query_semantics_version":u32::schema(),"selection_policy_version":u32::schema(),"unit_catalog_version":u32::schema()}})
    }
}
