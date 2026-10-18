//! Realtime node-graph audio engine for robat-synth.
//!
//! The UI thread owns a [`Session`] (the project model); every edit is
//! mirrored to the audio thread's [`EngineCore`] through lock-free ring
//! buffers. Nothing on the audio thread allocates, locks or frees.

mod core;
pub mod dsp;
pub mod model;
pub mod nodes;
mod session;

pub use crate::core::{new_engine, EngineCore, EngineHandle, Meters, BLOCK};
pub use model::{NodeKind, NodeModel, Project};
pub use session::{render_offline, EditError, Session};
