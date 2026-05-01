# soundEQ — Project Memory

## What this project is
Two separate apps that share the same eq-core DSP library:

**soundEQ (VB-Cable edition)** — this repo's main Tauri app. Captures all audio
output via WASAPI loopback through VB-Cable, applies DSP filtering, routes
processed audio back out through the real output device. Complete and shipping.

**soundEQ (APO edition)** — a separate Tauri app (separate scaffold, no VB-Cable
pipeline). Routes EQ through a native Windows APO COM DLL (`eq-apo/`) loaded by
`audiodg.exe` as an MFX. Shares `eq-core` and the profile/config JSON format so
users can migrate without losing their settings. Under active development.

Both apps are built with Tauri v2 (Rust backend + React frontend).

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
- Crossfeed is per-profile (not global), stored in Profile.crossfeed, applied after the EQ chain in the capture loop; three fixed presets (Mild 25%, Moderate 40%, Strong 55%), no custom slider

## What is already built
- `eq-core/` — complete and tested
    - `filter_type.rs` — FilterType enum, BandConfig struct, validation, serde
    - `biquad.rs` — Coefficients (all 7 formulas), BiquadFilter, magnitude_db_at()
    - `filter_chain.rs` — FilterChain, stereo interleaved processing, frequency_response_curve()
    - `FilterChain::bypass_all()` sets active_count = 0 (fast-path passthrough, no allocation)
    - `crossfeed.rs` — CrossfeedProcessor (Mild/Moderate/Strong presets); Linkwitz-style algorithm: LP at 700 Hz (Butterworth Q=0.707) + ~0.3 ms inter-aural delay via fixed-size [f64; 64] ring buffers; no heap allocation; applied after FilterChain in the capture hot loop
    - `CrossfeedConfig` stored per-profile with `#[serde(default)]` — old saves load cleanly with crossfeed disabled
    - 19 passing unit tests covering all filter types and edge cases
- `eq-audio/` — WASAPI loopback + render pipeline
    - Sessions deduped by process_name (WASAPI creates multiple sessions per process)
    - `list_audio_sessions()` filters to `AudioSessionStateActive` only — paused/background apps excluded
    - `capture.rs` — `LoopbackCapture::start()` accepts `Arc<Mutex<CrossfeedProcessor>>`; applied per buffer after FilterChain
- `src-tauri/src/engine.rs` — AudioEngine
    - `bypassed: bool` — when true, `apply_profile()` immediately calls `bypass_all()` on the chain
    - `crossfeed: Arc<Mutex<CrossfeedProcessor>>` — shared with capture thread; updated by `apply_profile()` and `apply_crossfeed_config()`
    - `preferred_app: Option<String>` — process name of the app the user last explicitly assigned; routing uses this as tiebreaker when multiple apps are simultaneously active
- `src-tauri/src/lib.rs` — all Tauri IPC commands
    - `set_eq_bypass` / `is_eq_bypassed` — global bypass toggle
    - `set_profile_crossfeed(profile_name, config)` — updates crossfeed config in the store and applies live if this is the active profile
    - `apply_app_profile(process_name)` — immediately applies an app's assigned profile + sets preferred_app
    - `select_profile_for_sessions` checks preferred_app first (if still active), then falls back to WASAPI order
    - `export_profile(name) -> String` — returns a pretty-printed JSON string of the Profile struct; frontend triggers the file download via Blob URL
    - `import_profile(json) -> String` — deserializes + validates a Profile JSON, resolves name collisions with " (2)" / " (3)" suffix, persists, returns the final name used
    - `is_startup_launch() -> bool` — returns true when the process was launched with `--minimized` (the Windows startup registry entry); the frontend delays `startEngine` by 10 s when true, letting the audio subsystem finish initializing at login
    - `make_state_icon(r, g, b)` — renders a 32×32 anti-aliased circle as `Image<'static>` from raw RGBA; called by `update_tray_status` to swap the tray icon per state (gray=stopped, amber=bypassed, green=active)
- `src/components/AppPanel.tsx` — replaces SessionPanel; shows manually managed app assignments
    - Apps are added explicitly (never auto-added); "+" expands an inline form with running-app suggestions
    - Old `SessionPanel.tsx` is superseded but still present (unused)

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

## Changelog discipline
- All significant changes must be recorded in `CHANGELOG.md` under `[Unreleased]` at the end of every session.
- Format: feature/fix name, one-sentence description, affected files.

## Key architectural constraints (do not re-litigate)
- The EQ is applied post-mix to the entire WASAPI loopback output — one profile at a time for all audio. Per-app routing switches the active profile when a different assigned app becomes the active audio source; it does not process streams independently.
- Lock ordering in Tauri commands: always lock `store` before `engine`, or lock one, drop it, then lock the other. Never hold both simultaneously (deadlock risk). See `set_eq_bypass` and `set_active_profile` for the established pattern.
- `apply_profile()` on `AudioEngine` must be called with only the engine lock held (never also holding the store lock). Clone the profile out of the store first, drop the store lock, then call apply_profile.

## APO app plan (separate product — not a replacement for the VB-Cable app)
The APO app is a distinct Tauri app that uses a native Windows APO COM DLL
(`eq-apo/`) instead of VB-Cable. The VB-Cable app stays as-is. Both apps share
`eq-core` and the same profile JSON format.

Advantage of APO: the MFX position sits *after* endpoint volume, so the Windows
volume slider works correctly. VB-Cable captures before endpoint volume, so
volume is fixed at 100% regardless of the slider.

### APO app roadmap
- [x] Phase 1 — COM shell: DLL compiles, exports DllGetClassObject/DllCanUnloadNow,
      all APO interfaces implemented as passthrough stubs. `register.ps1` installs it
      on all render endpoints as MFX. Registration confirmed working (2026-04-27).
      Files: `eq-apo/src/{lib,apo,factory}.rs`, `eq-apo/register.ps1`
- [ ] Phase 2 — DSP wiring: `APOProcess` reads active profile from the JSON file
      the Tauri app writes, builds a `FilterChain`, applies EQ in-place each buffer period.
      LockForProcess captures sample rate / channel count from the negotiated format.
- [ ] Phase 3 — Live IPC: APO watches the profile JSON (ReadDirectoryChangesW or a
      named pipe) so profile switches take effect without restarting the audio service.
- [ ] Phase 4 — Separate Tauri scaffold: build the APO app's own Tauri frontend/backend
      with no VB-Cable pipeline, no capture/render engine, no setup banner.
- [ ] Phase 5 — Installer polish: `register.ps1` runs at install time; unregister on
      uninstall; CA code-signing cert required (Azure Trusted Signing ~$3/month).

### Key APO constraints
- `APOProcess` is called on the audiodg.exe audio thread — no allocation (same rule as eq-audio)
- MFX (Mode Effects) placement: after endpoint volume, before hardware. Do NOT use LFX (per-stream, before mix) or GFX (before endpoint volume).
- CLSID: {8C2A5F3E-B47D-4A1C-9E8F-D0C3B6A2E1F4} — must match in lib.rs and register.ps1
- DLL path at release: `target\release\eq_apo.dll`
- To rebuild: `cargo build --release -p eq-apo`
- To register: run `eq-apo\register.ps1` as Administrator, then `net stop audiosrv && net start audiosrv`
- To unregister: `eq-apo\register.ps1 -Unregister`

## Do not suggest
- Electron (we chose Tauri deliberately)
- Web Audio API for DSP (processing happens in Rust, not the browser)
- Any approach that allocates in the audio thread
- Rewriting eq-core (it is complete and tested)
- Auto-adding detected audio sessions to the Apps panel (user manages apps manually)
- Merging the VB-Cable app and APO app into one app — they are intentionally separate products