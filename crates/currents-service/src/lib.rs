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
