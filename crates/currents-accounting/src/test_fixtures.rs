//! Test-only fixed synthetic inputs, not a native command construction API.
//! Enabled only by downstream dev-dependencies, never by production defaults.
use crate::PreparedNativeOperation;
use doubleentry::{currency::USD, date, store::Op};

/// A single native event for qualifying domain-prefix isolation on a fresh dataset.
/// No caller-controlled native values, wire bytes, SQL or handles cross the boundary.
pub fn currency_declaration() -> PreparedNativeOperation {
    PreparedNativeOperation {
        operation: Op::Currency {
            code: USD,
            exponent: 2,
            effective: date!("0001-01-01"),
            actor: "synthetic".into(),
        },
    }
}
