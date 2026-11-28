//! C11 investments. CUR-021 declares securities and positions, recognizes
//! trades, reinvested dividends and splits, keeps other corporate actions
//! unresolved and publishes holdings snapshots; openings and valuation
//! belong to later leaves.
pub mod actions;
pub mod holdings;
pub mod quantity;
pub(crate) mod replay;
pub use quantity::InvestmentEngine;
