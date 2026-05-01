// =============================================================================
// lib.rs — eq-core crate root
//
// This library is the standalone DSP engine. It has zero platform dependencies
// (no Windows, no Tauri, no audio I/O) — it's pure math and data structures.
//
// When we integrate into Tauri, the backend will depend on this crate.
// It can also be tested independently with `cargo test` right now.
//
// Public API surface:
//   - FilterType, BandConfig        — configuration types (filter_type.rs)
//   - Coefficients, BiquadFilter    — low-level DSP primitives (biquad.rs)
//   - FilterChain                   — the full stereo EQ engine (filter_chain.rs)
//   - MAX_BANDS                     — band count limit constant
//   - Profile, ProfileStore         — profile & routing management (profile.rs)
//   - ProfileError                  — error type for profile operations
//   - builtin_presets               — factory presets (preset.rs)
// =============================================================================

pub mod filter_type;
pub mod biquad;
pub mod filter_chain;
pub mod profile;
pub mod preset;
pub mod crossfeed;

// Re-export the most commonly used types at the crate root for convenience.
// Downstream code can do `use eq_core::FilterChain` instead of the full path.
pub use filter_type::{FilterType, BandConfig};
pub use biquad::{BiquadFilter, Coefficients};
pub use filter_chain::{FilterChain, MAX_BANDS};
pub use profile::{Profile, ProfileStore, ProfileError, DEFAULT_PROFILE_NAME};
pub use preset::builtin_presets;
pub use crossfeed::{CrossfeedConfig, CrossfeedLevel, CrossfeedProcessor};
