# eq-core

The DSP library for soundEQ. Platform-independent, pure Rust — no Windows or Tauri
dependencies. Contains the filter math, filter chain, profile/preset store, and
crossfeed processor.

## Modules

| Module | Contents |
|---|---|
| `biquad.rs` | `Coefficients` (7 filter formulas), `BiquadFilter`, `magnitude_db_at()` |
| `filter_type.rs` | `FilterType` enum, `BandConfig` struct, validation, serde |
| `filter_chain.rs` | `FilterChain` — cascaded stereo biquad processing, `frequency_response_curve()` |
| `crossfeed.rs` | `CrossfeedProcessor` — Linkwitz-style headphone crossfeed (LP at 700 Hz + inter-aural delay) |
| `profile.rs` | `Profile`, `ProfileStore` — named EQ profiles, per-app routing, persistence |
| `preset.rs` | Built-in factory presets (Bass Boost, Gaming, Vocal Clarity, etc.) |

## Development

```bash
cargo test          # run the unit test suite (19 tests)
cargo test -- --ignored   # also run hardware-dependent capture tests
```

## Project structure

See the root `README.md` for the full project layout and architecture overview.
