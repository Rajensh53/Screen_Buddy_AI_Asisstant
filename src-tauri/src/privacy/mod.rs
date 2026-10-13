//! Privacy rules applied to every selection before it is read or explained.

pub mod gate;

pub use gate::{AppIdentity, BlockList, Decision, PrivacyGate, SkipReason};
