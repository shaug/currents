//! Exact bounded decimal arithmetic. No implicit rounding or unit conversion.
use crate::{EngineError, UnitId};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amount {
    pub coefficient: i128,
    pub scale: u8,
    pub unit_id: UnitId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    coefficient: String,
    scale: u8,
    unit_id: UnitId,
}
impl Amount {
    pub fn new(coefficient: i128, scale: u8, unit_id: UnitId) -> Result<Self, EngineError> {
        let v = Self {
            coefficient,
            scale,
            unit_id,
        };
        v.validate()?;
        Ok(v)
    }
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.scale > 18 || self.coefficient == i128::MIN {
            Err(EngineError::precision())
        } else {
            Ok(())
        }
    }
    pub fn normalized(self) -> Result<Self, EngineError> {
        self.validate()?;
        let mut v = self;
        if v.coefficient == 0 {
            v.scale = 0;
        } else {
            while v.scale > 0 && v.coefficient % 10 == 0 {
                v.coefficient /= 10;
                v.scale -= 1;
            }
        }
        Ok(v)
    }
    pub fn checked_rescale(&self, scale: u8) -> Result<Self, EngineError> {
        self.validate()?;
        if scale > 18 {
            return Err(EngineError::precision());
        }
        let mut n = BigInt::from(self.coefficient);
        if scale >= self.scale {
            n *= BigInt::from(10).pow((scale - self.scale).into());
        } else {
            let d = BigInt::from(10).pow((self.scale - scale).into());
            if !(&n % &d).is_zero() {
                return Err(EngineError::precision());
            }
            n /= d;
        }
        Self::new(
            n.to_i128().ok_or_else(EngineError::precision)?,
            scale,
            self.unit_id,
        )
    }
    pub fn checked_neg(&self) -> Result<Self, EngineError> {
        self.validate()?;
        Self::new(-self.coefficient, self.scale, self.unit_id)
    }
    pub fn checked_add(&self, other: &Self) -> Result<Self, EngineError> {
        if self.unit_id != other.unit_id {
            return Err(EngineError::invalid());
        }
        let scale = self.scale.max(other.scale);
        let a = self.checked_rescale(scale)?;
        let b = other.checked_rescale(scale)?;
        Self::new(
            a.coefficient
                .checked_add(b.coefficient)
                .ok_or_else(EngineError::precision)?,
            scale,
            self.unit_id,
        )
    }
    /// Multiply by an exact bounded rational; a nonintegral result is rejected.
    /// At most three i128-sized factors and 10^18 are materialized, never arbitrary input integers.
    pub fn checked_ratio(
        &self,
        numerator: i128,
        denominator: i128,
        scale: u8,
    ) -> Result<Self, EngineError> {
        self.validate()?;
        if scale > 18 || numerator == i128::MIN || denominator == 0 || denominator == i128::MIN {
            return Err(EngineError::precision());
        }
        let n = BigInt::from(self.coefficient)
            * BigInt::from(numerator)
            * BigInt::from(10).pow(scale.into());
        let d = BigInt::from(denominator) * BigInt::from(10).pow(self.scale.into());
        if !(&n % &d).is_zero() {
            return Err(EngineError::precision());
        }
        Self::new(
            (n / d).to_i128().ok_or_else(EngineError::precision)?,
            scale,
            self.unit_id,
        )
    }
    pub fn decimal_string(&self) -> Result<String, EngineError> {
        let v = self.normalized()?;
        let neg = if v.coefficient < 0 { "-" } else { "" };
        let digits = v.coefficient.abs().to_string();
        if v.scale == 0 {
            return Ok(format!("{neg}{digits}"));
        }
        let padded = format!("{:0>width$}", digits, width = v.scale as usize + 1);
        let p = padded.len() - v.scale as usize;
        Ok(format!("{neg}{}.{}", &padded[..p], &padded[p..]))
    }
}
impl Serialize for Amount {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = self.normalized().map_err(serde::ser::Error::custom)?;
        Wire {
            coefficient: v.coefficient.to_string(),
            scale: v.scale,
            unit_id: v.unit_id,
        }
        .serialize(s)
    }
}
impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let digits = w.coefficient.strip_prefix('-').unwrap_or(&w.coefficient);
        if w.coefficient.len() > 40
            || digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || (digits.len() > 1 && digits.starts_with('0'))
        {
            return Err(serde::de::Error::custom("invalid coefficient"));
        }
        let c = w
            .coefficient
            .parse()
            .map_err(|_| serde::de::Error::custom("coefficient overflow"))?;
        Self::new(c, w.scale, w.unit_id)
            .and_then(Self::normalized)
            .map_err(serde::de::Error::custom)
    }
}
impl crate::schema::Schema for Amount {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["coefficient","scale","unit_id"],"properties":{"coefficient":{"type":"string","pattern":"^-?(0|[1-9][0-9]{0,38})$","maxLength":40},"scale":{"type":"integer","minimum":0,"maximum":18},"unit_id":<UnitId as crate::schema::Schema>::schema()}})
    }
}
