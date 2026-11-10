//! JSON Schema generated from the same field/variant declarations as Rust DTOs.
use serde_json::{Value, json};
pub fn required<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    d: D,
) -> Result<T, D::Error> {
    T::deserialize(d)
}
pub trait Schema {
    fn schema() -> Value;
}
impl Schema for String {
    fn schema() -> Value {
        json!({"type":"string","maxLength":1048576})
    }
}
impl Schema for bool {
    fn schema() -> Value {
        json!({"type":"boolean"})
    }
}
impl Schema for u8 {
    fn schema() -> Value {
        json!({"type":"integer","minimum":0,"maximum":255})
    }
}
impl Schema for u32 {
    fn schema() -> Value {
        json!({"type":"integer","minimum":0,"maximum":4294967295u64})
    }
}
impl<T: Schema> Schema for Vec<T> {
    fn schema() -> Value {
        json!({"type":"array","items":T::schema(),"maxItems":16384})
    }
}
impl<T: Schema> Schema for Option<T> {
    fn schema() -> Value {
        json!({"anyOf":[T::schema(),{"type":"null"}]})
    }
}
#[macro_export]
macro_rules! dto { ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
 #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
 #[serde(deny_unknown_fields)]
 pub struct $name {$(#[serde(deserialize_with="currents_types::schema::required")] pub $field:$ty),*}
 impl $crate::schema::Schema for $name {fn schema()->serde_json::Value{serde_json::json!({"type":"object","additionalProperties":false,"required":[$(stringify!($field)),*],"properties":{$(stringify!($field):<$ty as $crate::schema::Schema>::schema()),*}})}}
}; }
#[macro_export]
macro_rules! choices { ($name:ident { $($variant:ident),* $(,)? }) => {
 #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
 pub enum $name {$($variant),*}
 impl $crate::schema::Schema for $name {fn schema()->serde_json::Value{serde_json::json!({"type":"string","enum":[$(stringify!($variant)),*]})}}
}; }
#[macro_export]
macro_rules! tagged { ($name:ident { $($variant:ident($ty:ty)),* $(,)? }) => {
 #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
 #[serde(tag="kind",content="value",deny_unknown_fields)]
 pub enum $name {$($variant($ty)),*}
 impl $crate::schema::Schema for $name {fn schema()->serde_json::Value{serde_json::json!({"oneOf":[$( {"type":"object","additionalProperties":false,"required":["kind","value"],"properties":{"kind":{"const":stringify!($variant)},"value":<$ty as $crate::schema::Schema>::schema()}}),*]})}}
}; }
