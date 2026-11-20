//! Private C03 service. Production signed-host and domain semantics remain gated.
mod admission;
mod commands;
mod service;
pub use commands::{ApprovalChallenge, CommandPlan, CommandPlanner, HostGrants, ResultAccess};
pub use service::{Service, ServiceProfile};

#[cfg(test)]
use crate as service_api;
// Shared synthetic helpers are public only in the unit-test harness.
#[cfg(test)]
#[path = "../tests/support/lifecycle.rs"]
pub mod test_support;

mod replay;
pub use replay::ReplayCoordinator;

mod import;
pub use import::ImportEvidence;

mod correction_preflight;
#[cfg(feature = "synthetic-test-fixtures")]
#[doc(hidden)]
pub use correction_preflight::synthetic_commit_verified_correction;
pub use correction_preflight::verify_correction_preflight;
