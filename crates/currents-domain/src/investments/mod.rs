//! C11 investments. CUR-021 declares securities and positions and recognizes
//! trades; openings and valuation belong to later leaves.
pub mod quantity;
pub(crate) mod replay;
pub use quantity::InvestmentEngine;
