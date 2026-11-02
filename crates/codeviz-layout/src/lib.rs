//! The codebase as codeviz lays it out: [`map()`] gives every function a tile, grouped
//! by container; [`ForceGraph`] settles them in 3D as a graph of calls, once, to seed
//! where each module's functions sit inside its sphere; and [`atlas`] places every
//! module and its functions for browsing, once per document.

pub mod atlas;
mod graph;
mod map;

pub use graph::{ForceConfig, ForceGraph};
pub use map::{CodebaseMap, map};
