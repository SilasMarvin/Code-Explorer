//! The language-agnostic core of codeviz.
//!
//! Language frontends produce a [`CallGraph`]: a static set of [`Function`]s whose
//! [`CallSite`]s point at other functions. A [`Trace`] unrolls that graph from a chosen
//! entry point into a tree of invocations ordered on a logical clock, which is what
//! renderers lay out in space.

mod graph;
mod trace;

pub use graph::{
    CallGraph, CallSite, Callee, Function, FunctionId, GraphError, Location, Owner, Resolution,
};
pub use trace::{StopReason, Trace, TraceNode, TraceNodeId, TraceOptions, TraceTarget};
