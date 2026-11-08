//! Pedal graph: model (contract, phase 0), loading, compilation and
//! execution (task A1, see docs/PLAN.md §7).

pub mod compile;
pub mod exec;
pub mod load;
pub mod model;

#[allow(unused_imports)] // GraphError: for the integration
pub use compile::{compile, GraphError, NodeMap, ParamChange};
pub use exec::Plan;
