//! Private domain boundary. Financial behavior is implemented by its owning tickets.

pub mod catalog;
pub mod evidence;
pub mod replay;
pub use replay::ReplayEngine;

pub mod recognition;

pub mod segments;

pub mod policy_history;

mod recognition_replay;
