//! Exact native scale and explicit rational rounding. No floating point.
use currents_types::{Amount, EngineError, RecordId, UnitDefinition};
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
pub fn native_amount(amount: &Amount, unit: &UnitDefinition) -> Result<i128, EngineError> {
    unit.validate()?;
    amount.validate()?;
    if amount.unit_id != unit.id || (unit.native_code == "USD" && unit.exponent != 2) {
        return Err(EngineError::precision());
    }
    amount.checked_rescale(unit.exponent)?;
    Ok(amount.checked_rescale(18)?.coefficient)
}
pub fn checked_sum(values: &[i128]) -> Result<i128, EngineError> {
    values.iter().try_fold(0i128, |sum, v| {
        if *v == i128::MIN {
            return Err(EngineError::precision());
        }
        let n = sum.checked_add(*v).ok_or_else(EngineError::precision)?;
        if n == i128::MIN {
            Err(EngineError::precision())
        } else {
            Ok(n)
        }
    })
}
#[derive(Debug, PartialEq, Eq)]
pub struct RoundedRatio {
    pub rounded: i128,
    pub remainder_numerator: i128,
    pub denominator: i128,
}
#[expect(
    clippy::arithmetic_side_effects,
    reason = "Bounded BigInt intermediates; C05 exact rational arithmetic"
)]
pub fn half_even(numerator: i128, denominator: i128) -> Result<RoundedRatio, EngineError> {
    if numerator == i128::MIN || denominator <= 0 {
        return Err(EngineError::precision());
    }
    let n = BigInt::from(numerator);
    let d = BigInt::from(denominator);
    let q = &n / &d;
    let r = &n % &d;
    let twice = if r < BigInt::zero() { -&r * 2 } else { &r * 2 };
    let round = twice > d || (twice == d && !(&q % 2u8).is_zero());
    let q = if round {
        q + if numerator < 0 { -1 } else { 1 }
    } else {
        q
    };
    let remainder = &n - &q * &d;
    Ok(RoundedRatio {
        rounded: q.to_i128().ok_or_else(EngineError::precision)?,
        remainder_numerator: remainder.to_i128().ok_or_else(EngineError::precision)?,
        denominator,
    })
}
#[expect(
    clippy::arithmetic_side_effects,
    reason = "Bounded nonnegative BigInt products and sums; C05 exact allocation"
)]
pub fn largest_remainder(
    total: i128,
    weights: &[(RecordId, i128)],
) -> Result<Vec<(RecordId, i128)>, EngineError> {
    if total < 0 || weights.is_empty() || weights.len() > 20_000 {
        return Err(EngineError::precision());
    }
    let mut ordered = weights.to_vec();
    ordered.sort_by_key(|x| x.0);
    if ordered.windows(2).any(|w| w[0].0 == w[1].0) || ordered.iter().any(|x| x.1 < 0) {
        return Err(EngineError::invalid());
    }
    let denominator: BigInt = ordered.iter().map(|x| BigInt::from(x.1)).sum();
    if denominator.is_zero() {
        return Err(EngineError::precision());
    }
    let mut rows = Vec::with_capacity(ordered.len());
    let mut allocated = 0i128;
    for (id, w) in ordered {
        let n = BigInt::from(total) * w;
        let q = (&n / &denominator)
            .to_i128()
            .ok_or_else(EngineError::precision)?;
        allocated = allocated
            .checked_add(q)
            .ok_or_else(EngineError::precision)?;
        rows.push((id, q, n % &denominator));
    }
    let left = total
        .checked_sub(allocated)
        .ok_or_else(EngineError::precision)?;
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    for row in rows
        .iter_mut()
        .take(usize::try_from(left).map_err(|_| EngineError::precision())?)
    {
        row.1 = row.1.checked_add(1).ok_or_else(EngineError::precision)?;
    }
    rows.sort_by_key(|x| x.0);
    Ok(rows.into_iter().map(|(id, n, _)| (id, n)).collect())
}
