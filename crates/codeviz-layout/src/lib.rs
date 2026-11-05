//! The codebase as codeviz lays it out: [`map()`] gives every function a tile, grouped
//! by container; [`CallGraph`] holds the calls between them; and [`place`] puts every
//! module and every function somewhere in 3D, once per document.

mod graph;
mod map;
pub mod place;

pub use graph::CallGraph;
pub use map::{CodebaseMap, map};
