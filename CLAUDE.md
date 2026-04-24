# soundEQ — Project Memory

## What this project is
A system-wide parametric EQ for Windows. Captures all audio output via WASAPI
loopback, applies DSP filtering, routes processed audio back out through a
virtual audio device. Built with Tauri v2 (Rust backend + React frontend).

## Architecture
## Key decisions (do not re-discuss these)
- Parametric EQ, up to 16 bands per profile (MAX_BANDS = 16)
- 7 filter types: Peak, LowShelf, HighShelf, LowPass, HighPass, Notch, Bandpass
- Per-app profiles with global default profile as fallback
- Unknown apps automatically use the global default profile
- Built-in presets ship with the app (Bass Boost, Gaming, Vocal Clarity, etc.)
- f64 internal DSP math, f32 at the WASAPI I/O boundary
- Strictly no heap allocation in the audio thread (no Vec, Box, String)
- Interleaved stereo buffer format: [L, R, L, R, ...]
- Output clamped to [-1.0, 1.0] to prevent digital clipping
- System tray on startup, with user toggle to disable startup behavior
- Spectrum analyzer deferred to Phase 2

## What is already built
- `eq-core/` — complete and tested
    - `filter_type.rs` — FilterType enum, BandConfig struct, validation, serde
    - `biquad.rs` — Coefficients (all 7 formulas), BiquadFilter, magnitude_db_at()
    - `filter_chain.rs` — FilterChain, stereo interleaved processing, frequency_response_curve()
    - 19 passing unit tests covering all filter types and edge cases

## Build order (check off as completed)
- [x] Step 1a: eq-core DSP library
- [x] Step 1b: Profile & preset store (pure Rust, no platform deps)
- [x] Step 2a: WASAPI loopback capture (windows-rs)
- [x] Step 2b: Virtual audio device routing
- [x] Step 2c: Per-app audio session detection
- [x] Step 3: Tauri v2 project scaffold + IPC bridge
- [x] Step 4: React frontend (EQ curve, band controls, preset manager)
- [x] Step 5: System tray + Windows startup integration
- [x] Step 6: End-to-end integration + testing

## Code standards (always follow these)
- Comment every major section with a block explaining WHAT and WHY
- Inline comments on anything mathematically complex
- Assume the reader is a CS/software engineering student
- Explain audio concepts when they first appear (e.g. what sample rate means)
- All Rust: use `thiserror` for error types, `serde` for serialization
- All TypeScript: strict mode, no `any`
- No unsafe Rust except where windows-rs requires it, and always with a comment explaining why

## Tech stack
| Layer | Technology |
|---|---|
| App framework | Tauri v2 |
| Frontend | React + TypeScript + Tailwind |
| Frontend build | Vite |
| EQ curve visualization | Canvas API (custom, no library) |
| Backend / DSP | Rust |
| Audio API | Windows WASAPI (windows-rs crate) |
| Presets / config | JSON files in AppData |
| IPC | Tauri commands + events |

## Crate versions (use these exactly)
- tauri = "2"
- windows = "0.58" (with features: Win32_Media_Audio, Win32_System_Com)
- serde = "1.0" with derive feature
- serde_json = "1.0"
- thiserror = "1.0"
- tokio = "1" with full features (async runtime for Tauri)

## File/folder naming conventions
- Rust modules: snake_case
- React components: PascalCase
- TypeScript files: camelCase for utilities, PascalCase for components
- Preset JSON files: kebab-case (e.g. bass-boost.json)
- Config stored in: %APPDATA%\WindowsEQ\

## Do not suggest
- Electron (we chose Tauri deliberately)
- Web Audio API for DSP (processing happens in Rust, not the browser)
- Any approach that allocates in the audio thread
- Rewriting eq-core (it is complete and tested)