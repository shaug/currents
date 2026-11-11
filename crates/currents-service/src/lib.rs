//! Private C03 service. Production signed-host and domain semantics remain gated.
mod admission;
mod commands;
mod service;
pub use commands::{ApprovalChallenge, CommandPlan, CommandPlanner, HostGrants, ResultAccess};
pub use service::{Service, ServiceProfile};
