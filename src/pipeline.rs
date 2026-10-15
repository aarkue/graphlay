//! The per-phase straightness profile of long edges, for the evaluation harness. Empty without
//! the `fixtures` feature.

#[cfg(feature = "fixtures")]
pub use crate::layered::profile::{chain_steps, front_end, phase_profile, PhaseSteps};
