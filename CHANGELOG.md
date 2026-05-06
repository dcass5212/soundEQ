# Changelog

## [1.0.0] - 2026-05-05

### Fixed
- **Icon added to README header** — `src-tauri/icons/app-icon-source.png` (1024×1024) rendered at 100 px above the title using centered HTML. GitHub social preview must be set manually via repo Settings → Social preview.
  - `README.md`
- **Documentation review** — audited README.md, DECISIONS.md, and CHANGELOG.md for stale content. Fixed: uninstall instructions now describe the installer prompt (added this session) instead of saying "delete manually"; keyboard navigation table was missing `F2`, `Space`, `m`, `s`; no mention of `Ctrl+Shift+B` bypass hotkey, output gain control, or per-app volume slider. Added sections for Output Gain and updated EQ Bypass with the hotkey. Updated ADR-007 to reflect the spectrum analyzer shipped in Phase 2. Fixed duplicate `### Changed` headers and missing blank line in CHANGELOG.
  - `README.md`, `DECISIONS.md`, `CHANGELOG.md`
- **Uninstaller left `%APPDATA%\com.soundeq.app` on disk** — added `NSIS_HOOK_POSTUNINSTALL` in `src-tauri/nsis-hooks.nsh`. After removing the binary and shortcuts, the uninstaller now checks for the data directory and offers a Yes/No prompt: Yes deletes it (clean uninstall), No preserves it (useful before reinstalling). The dialog is skipped entirely if the directory doesn't exist.
  - `src-tauri/nsis-hooks.nsh`, `src-tauri/tauri.conf.json`
- **App icon missing from Task Manager / Alt+Tab / taskbar** — WebView2 does not inherit the executable's embedded resource icon automatically. Added an explicit `win.set_icon(app.default_window_icon())` call in the setup handler so the icon is pushed to the window on every launch.
  - `src-tauri/src/lib.rs`
- **`GetBufferSize` called once per render cycle instead of per loop** — `IAudioClient::GetBufferSize()` returns a constant value fixed at `Initialize()` time, but was being called on every render-loop iteration (~200×/second). Moved the call before the loop; only `GetCurrentPadding()` (which changes each cycle) remains inside.
  - `eq-audio/src/render.rs`
- **Stale hardware test signatures** — two `#[ignore]` capture tests called `LoopbackCapture::start()` with the pre-crossfeed 3-argument signature, causing compile errors when running the test suite. Updated to pass a `CrossfeedProcessor` as the third argument.
  - `eq-audio/src/capture.rs`

### Changed
- **Version bumped to 1.0.0** — updated across all manifests: `eq-core/Cargo.toml`, `eq-audio/Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `package.json`.
- **GitHub Actions CI added** — `.github/workflows/ci.yml` runs `cargo check`, `cargo clippy -- -D warnings`, `cargo test`, and `npm run build` on every push and PR to master using `windows-latest`.
  - `.github/workflows/ci.yml`
- **Clippy clean** — resolved all `clippy::needless_range_loop`, `clippy::unnecessary_map_or`, `clippy::io_other_error`, and `clippy::redundant_closure` warnings across `eq-core`, `eq-audio`, and `src-tauri`. Zero warnings with `-D warnings`.
  - `eq-core/src/filter_chain.rs`, `eq-audio/src/capture.rs`, `eq-audio/src/render.rs`, `src-tauri/src/engine.rs`, `src-tauri/src/persistence.rs`
- **`.gitignore` tightened** — generalized `bash.exe.stackdump` to `*.stackdump` to cover any shell crash dump; added `eq-core/Cargo.lock` (spurious workspace-member lock file that should not be tracked).
  - `.gitignore`
- **Dead code removed from audio layer** — `make_render_waveformatex` in `render.rs` was defined but never called (not even in tests); removed. `make_waveformatex` in `capture.rs` is test-only; marked `#[cfg(test)]` to eliminate the dead_code lint.
  - `eq-audio/src/render.rs`, `eq-audio/src/capture.rs`
- **Removed eq-apo from repo** — APO edition moved to its own separate repository; `eq-apo/` directory deleted, `Cargo.toml` workspace members updated, `CLAUDE.md` stripped of APO plan sections.
  - `Cargo.toml`, `CLAUDE.md`

### Fixed
- **Audio-thread allocation guard** — `processing_buf` in the capture loop was pre-allocated at 4096 samples but could silently reallocate if a WASAPI packet exceeded that size (e.g. 192 kHz exclusive mode). Increased capacity to 8192 and added an explicit guard that logs and skips oversized packets rather than reallocating on the audio thread.
  - `eq-audio/src/capture.rs`
- **`handleSetVolume` missing rollback** — the per-app volume slider applied an optimistic store update before the IPC call; on failure the error was shown but the UI remained at the wrong value. Now calls `refreshStore()` in the catch block to restore the true value, matching the pattern already used by `handleCrossfeedChange`.
  - `src/App.tsx`

### Changed
- **`apply_bypass` redundant engine lock removed** — the function previously locked the engine a third time just to read `bypassed` after already setting it; `bypassed` is simply `enabled` after either branch, so the extra lock/unlock is dropped.
  - `src-tauri/src/lib.rs`
- **Mute/solo IPC errors now log a warning** — previously `.catch(() => {})` silently swallowed `applyBandsLive` failures in the mute and solo handlers. Failures now emit `console.warn` so they are visible in dev tools without being user-facing (mute/solo is transient, so no rollback is needed).
  - `src/App.tsx`
- **`update_band` out-of-range guard now asserts in debug builds** — the silent early-return on `index >= MAX_BANDS` is kept for production safety, but a `debug_assert!` is added so the condition is caught loudly during development and testing.
  - `eq-core/src/filter_chain.rs`
- **Lock ordering documented in lib.rs** — the global lock hierarchy (`store` before `engine`, never held simultaneously) is now stated once in the "App state" section comment, not only in per-function comments.
  - `src-tauri/src/lib.rs`

### Added
- **Distribution prep** — added end-user Installation section to README (covers VB-Cable setup, SmartScreen warning note, uninstall steps); moved dev prerequisites into the Development section. Configured `tauri.conf.json` with Windows NSIS bundle metadata (publisher, category, descriptions, homepage, signing placeholder, `installMode: currentUser`). Added LICENSE file as High Priority item in ROADMAP.
  - `README.md`, `src-tauri/tauri.conf.json`, `ROADMAP.md`

### Added
- **Tray icon state indicator** — the system tray icon now changes colour to reflect the current engine state: gray circle when stopped, amber when bypassed, green when active. Icons are rendered from raw RGBA at runtime (32×32, anti-aliased circle, no extra asset files). The change fires in sync with the existing tooltip and status menu item updates via `update_tray_status`.
  - `src-tauri/src/lib.rs` — `make_state_icon(r, g, b)` helper; `update_tray_status` now calls `handle.tray.set_icon(Some(icon))`

### Added
- **Profile import / export** — each profile row now shows a `↓` download button on hover; clicking it exports the profile as a `<name>.json` file download (pretty-printed JSON matching the internal Profile format). An `↑ Import` button sits alongside `+ New` at the bottom of the profile list; clicking it opens a file picker and imports the selected `.json` file into the store. Name collisions are resolved automatically by appending a numeric suffix (" (2)", " (3)", …). After import the new profile is selected and a toast confirms the name used.
  - `src-tauri/src/lib.rs` — `export_profile` and `import_profile` Tauri commands; registered in `invoke_handler`
  - `src/lib/api.ts` — `exportProfile()` and `importProfile()` typed wrappers
  - `src/components/ProfilePanel.tsx` — `onExport` and `onImportFile` props; `↓` button per profile row; `↑ Import` button + hidden `<input type="file">` ref
  - `src/App.tsx` — `handleExportProfile()` (Blob download) and `handleImportProfile()` (store refresh + select + notice) handlers; props wired to `ProfilePanel`

### Fixed
- **10-second startup delay when launched at Windows login** — when soundEQ is started by the Windows startup registry entry (`--minimized` flag), the audio engine auto-start is now deferred by 10 seconds. The app window, profiles, and all UI load immediately; only the WASAPI engine open is delayed. Prevents silent engine failures on systems where the audio subsystem takes a few seconds to finish initializing at login. Hardcoded; no user-facing option.
  - `src-tauri/src/lib.rs` — `is_startup_launch` Tauri command (returns true when `--minimized` is in argv); registered in invoke handler
  - `src/lib/api.ts` — `isStartupLaunch()` wrapper
  - `src/App.tsx` — auto-start useEffect calls `isStartupLaunch()` and awaits a 10 s `setTimeout` before `startEngine()` when true

### Added
- **Per-profile headphone crossfeed** — three preset levels (Mild 25%, Moderate 40%, Strong 55%) with an On/Off toggle, selectable via a segmented button bar above the band list. Crossfeed mixes a low-pass filtered, 0.3 ms-delayed copy of the opposite channel into each ear, simulating the natural acoustic crosstalk of speaker listening to reduce headphone fatigue. Implemented with a Linkwitz-style algorithm (LP at 700 Hz, Butterworth Q, ~14-sample interaural delay at 48 kHz). Stored per-profile in JSON (`#[serde(default)]` for backward compatibility with old saves). Takes effect on the next audio buffer without restart or dropout.
  - `eq-core/src/crossfeed.rs` — new: `CrossfeedLevel` enum, `CrossfeedConfig` struct, `CrossfeedProcessor` (fixed-size ring buffers + two biquad LP filters, no heap allocation in audio thread)
  - `eq-core/src/lib.rs` — `pub mod crossfeed`; re-exports `CrossfeedConfig`, `CrossfeedLevel`, `CrossfeedProcessor`
  - `eq-core/src/profile.rs` — `crossfeed: CrossfeedConfig` field on `Profile` (`#[serde(default)]`); `Profile::new()` and `Profile::with_bands()` initialize with `CrossfeedConfig::default()`
  - `eq-audio/src/capture.rs` — `LoopbackCapture::start()` accepts `Arc<Mutex<CrossfeedProcessor>>`; `capture_thread_main` calls `cf.set_sample_rate()` at startup; `run_capture_loop` applies crossfeed after the EQ chain per buffer
  - `src-tauri/src/engine.rs` — `crossfeed: Arc<Mutex<CrossfeedProcessor>>` field; wired into `start()` and `apply_profile()`; `apply_crossfeed_config()` helper for live updates without full profile rebuild
  - `src-tauri/src/lib.rs` — `set_profile_crossfeed` Tauri command; registered in `invoke_handler`
  - `src/lib/api.ts` — `CrossfeedLevel` type, `CrossfeedConfig` interface, `crossfeed?` on `Profile`, `setProfileCrossfeed()` wrapper
  - `src/App.tsx` — `crossfeedOption` derived state; `handleCrossfeedChange()` with optimistic store update; crossfeed segmented button bar between EqCanvas and band list

### Added
- **Keyboard navigation for band rows** — band rows now participate in the Tab order via a roving tabindex (one row owns `tabIndex=0` at a time, follows focus). Shortcuts at row-level focus: `↑`/`↓` move between rows, `Shift+↑`/`Shift+↓` reorder, `Enter`/`F2` enter edit mode, `Delete`/`Backspace` delete, `Space` toggle enabled, `m` toggle mute, `s` toggle solo, `Escape` exits edit mode back to the row. Focus ring improved from `ring-1/30%` to `ring-2/60%` for visibility. Roving index is clamped on band add/delete and synced when `pendingFocusRef` fires.
  - `src/App.tsx` — `rovingBandIdx` state; `onFocus={() => setRovingBandIdx(i)}` on each row wrapper; `tabIndex={rovingBandIdx === i ? 0 : -1}`; new `Space`/`m`/`s`/`F2` shortcuts in row-level `onKeyDown`; clamp effect on `bands.length`; `pendingFocusRef` effect now also calls `setRovingBandIdx`

### Fixed
- **AppPanel "Manage" popup no longer disrupts layout at small window sizes** — the expanded panel is now `position: absolute; bottom: 100%` (floats above the band list) instead of in-flow, so opening it never compresses the EQ canvas or band list regardless of window height.
  - `src/components/AppPanel.tsx` — root div gains `relative`; expanded div changed to `absolute bottom-full left-0 right-0 z-20` with `shadow-xl`

### Added
- **Per-app volume control** — each app in the Apps panel now has a volume slider (0–100%) that adjusts its WASAPI session volume independently of the master Windows volume. Implemented via `ISimpleAudioVolume::SetMasterVolume` on all WASAPI sessions matching the app's process name. Volume overrides are persisted in `ProfileStore.app_volumes` (JSON, backward-compatible with `#[serde(default)]`) and reset to 1.0 when an app is removed from the panel. Sliders update live (optimistic local state) while the IPC call applies the change to the audio session.
  - `eq-audio/src/session.rs` — `set_process_volume(process_name, volume)` iterates WASAPI sessions and calls `ISimpleAudioVolume::SetMasterVolume`; added `ISimpleAudioVolume` import
  - `eq-audio/src/lib.rs` — re-exported `set_process_volume`
  - `eq-core/src/profile.rs` — `app_volumes: HashMap<String, f32>` field on `ProfileStore` (`#[serde(default)]`); `get_app_volume()`, `set_app_volume()`, `app_volumes()` methods; `unassign_app()` now also removes the volume entry
  - `src-tauri/src/lib.rs` — `set_app_volume` Tauri command; `unassign_app` resets WASAPI session to 1.0 on removal; registered in `invoke_handler`
  - `src/lib/api.ts` — `app_volumes` field on `ProfileStore` interface; `setAppVolume()` wrapper
  - `src/components/AppPanel.tsx` — `appVolumes` + `onSetVolume` props; compact range slider per app row showing live percentage
  - `src/App.tsx` — `handleSetVolume` with optimistic store update; passes `appVolumes` and `onSetVolume` to AppPanel

### Added
- **Output gain control** — a `Vol −/+` control in the header lets users compensate for VB-Cable's inherently lower signal level vs. direct headset output. Steps in 5% increments from 50% to 200% (−6 dB to +6 dB). The gain is a linear multiplier applied in the render loop alongside the VolumeMonitor scalar, with hard clamp to [−1.0, 1.0] to prevent digital clipping. Persisted in config.json so the setting survives restarts.
  - `eq-audio/src/render.rs` — `output_gain: Arc<AtomicU32>` field on `WasapiRenderer`; `gain_handle()` method; `run_render_loop` applies `vol × gain` with clamp
  - `src-tauri/src/engine.rs` — `output_gain_saved: f32` + `output_gain_arc: Option<Arc<AtomicU32>>`; `set_output_gain()` / `get_output_gain()`; wired into `start()` and `stop()`
  - `src-tauri/src/persistence.rs` — `output_gain: f32` field on `AppConfig` (serde default 1.0)
  - `src-tauri/src/lib.rs` — `get_output_gain` / `set_output_gain` Tauri commands; saved gain applied to engine on startup
  - `src/lib/api.ts` — `getOutputGain()` / `setOutputGain()` wrappers
  - `src/App.tsx` — `outputGain` state; `handleOutputGainChange(delta)`; `Vol −/+ %` control in header

### Fixed
- **Windows volume slider now works while soundEQ is running (VB-Cable route)** — WASAPI loopback capture taps the stream before the endpoint's volume stage, so the system volume slider previously had no effect. A new `VolumeMonitor` background thread polls `IAudioEndpointVolume` on the capture device every 50 ms and writes the scalar into an `Arc<AtomicU32>`; the render loop reads it each buffer and multiplies all samples by it, making the slider work transparently. Mute is also handled (scalar → 0.0).
  - `eq-audio/src/volume.rs` — new file: `VolumeMonitor` struct with `start(device_id, volume_arc)` / `stop()` lifecycle; `open_endpoint_volume()` opens `IAudioEndpointVolume` by endpoint ID or system default; `read_volume()` checks mute flag then reads `GetMasterVolumeLevelScalar()`
  - `eq-audio/src/render.rs` — added `volume: Arc<AtomicU32>` field to `WasapiRenderer`; `volume_handle()` method; `run_render_loop` reads the atomic and multiplies samples before `ReleaseBuffer`
  - `eq-audio/src/lib.rs` — added `pub mod volume; pub use volume::VolumeMonitor;`
  - `eq-audio/Cargo.toml` — added `Win32_Media_Audio_Endpoints` windows feature for `IAudioEndpointVolume`
  - `src-tauri/src/engine.rs` — added `vol_monitor: VolumeMonitor` field; started in `start()` using `renderer.volume_handle()`; stopped first in `stop()`

### Added
- **ROADMAP.md** — added shipping checklist and future feature backlog.
  - `ROADMAP.md`

### Added
- **Profile sidebar polish** — text no longer shrinks when the panel is dragged slightly narrower; shrinking only begins below 160 px (previously started from the very first pixel dragged). Rename (✎) and delete (×) buttons are larger: pencil is now `max(12px, fontSize)` instead of `fontSize − 2`, and × is `max(16px, fontSize + 2)` with wider click padding.
  - `src/components/ProfilePanel.tsx` — new `fontSize` formula with 160 px breakpoint; pencil and × `fontSize` and `px-*` class updated

- **Keyboard navigation for band rows** — band rows are now keyboard-navigable without disrupting existing Tab/input behavior. Shortcuts when a row container is focused: ↑/↓ moves focus between rows; Shift+↑/↓ reorders the band one position (undo-able); Enter enters edit mode (focuses the first field); Delete/Backspace removes the band (focus shifts to the next row). Pressing Escape from inside any field (freq, gain, Q, filter-type select) exits edit mode and returns focus to the row container. A subtle indigo ring appears on the focused row.
  - `src/App.tsx` — wrapper `<div>` per row gains `tabIndex={-1}`, `focus-visible:ring-1`, and `onKeyDown` handler; `pendingFocusRef` + `useEffect` restores focus to the correct row after async reorder/delete

- **Fix static/clicks during live band editing** — `apply_bands_live` previously swapped in a brand-new `FilterChain` on every UI drag frame, resetting all biquad delay lines (x1, x2, y1, y2) to zero and producing a discontinuity audible as static. It now calls `set_bands()` on the existing chain, which updates coefficients in-place via `update_coefficients()` and preserves delay line state so the filter transitions smoothly.
  - `src-tauri/src/lib.rs` — `apply_bands_live`: replaced `profile.to_filter_chain()` + full chain swap with `filter_chain.lock().set_bands(&bands)` in-place update; also skips the update entirely when bypass is active

- **Tray icon state indicator** — the system tray tooltip and the context menu now reflect the current engine state in real time. A disabled status line at the top of the tray menu shows `○ Stopped`, `⊘ Bypassed`, or `● Active — <profile name>`. The tooltip mirrors the same state (e.g. `soundEQ | Bass Boost`). Updates on engine start/stop, bypass toggle, manual profile switch, focus-triggered profile switch, and device auto-restart.
  - `src-tauri/src/lib.rs` — `TrayHandle` struct; `tray: Mutex<TrayHandle>` field in `AppState`; `update_tray_status()` helper; `setup_tray()` returns `(TrayIcon<Wry>, MenuItem<Wry>)` and adds the status item; `start_engine`, `stop_engine`, `set_eq_bypass`, `set_active_profile` take `app: tauri::AppHandle` and call `update_tray_status`; `focus_tick` calls `update_tray_status` on profile switch; `device_watch_tick` calls `update_tray_status` on restart/error

### Added
- **Band solo / mute** — each band row now has **M** (mute) and **S** (solo) buttons. Muting silences a band in real time without removing it from the profile. Soloing a band silences all others. Both are non-destructive: the stored profile is unchanged and solo/mute state resets on profile switch. The EQ curve and band dots on the canvas update instantly to reflect the effective state. Soloed bands are highlighted; silenced rows and dots fade out.
  - `src-tauri/src/lib.rs` — `apply_bands_live` command: directly swaps the live filter chain without persisting; `get_frequency_response_live` command: computes the curve for arbitrary bands without a profile lookup; both registered in `invoke_handler`
  - `src/lib/api.ts` — `applyBandsLive()` and `getFrequencyResponseLive()` wrappers
  - `src/App.tsx` — `mutedBands: Set<number>` and `soloedBand: number | null` state; `computeEffective()` pure function; `effectiveBands` and `silencedIndices` memos; `handleToggleMute` / `handleToggleSolo` handlers; `applyBands()` calls `applyBandsLive(effectiveBands)` instead of `setActiveProfile` when solo/mute is active; curve refresh useEffect uses `getFrequencyResponseLive` when solo/mute is active; solo/mute cleared on profile switch/delete
  - `src/components/BandRow.tsx` — `isMuted`, `isSoloed`, `anySoloed`, `onMute`, `onSolo` props; M button (amber when muted) and S button (yellow when soloed); silenced rows rendered at 35% opacity
  - `src/components/EqCanvas.tsx` — `allBands?: BandConfig[]` prop for dot drawing / hit testing; `silenced?: ReadonlySet<number>` prop; silenced dots drawn at 20% fill opacity so they remain visible and draggable; gradient uses `bands` (effective) so only un-silenced bands color the curve

- **Undo / redo for band edits** — Ctrl+Z undoes the last band change; Ctrl+Y (or Ctrl+Shift+Z) redoes it. Up to 50 steps. ↩ / ↪ buttons in the header provide mouse access. History is cleared on explicit profile switch. Canvas dot drags count as one undo step regardless of how many frames fire. All other changes — BandRow field commits, filter-type changes, enable toggle, color picks, add, delete, reorder, and preset loads — are individually undoable.
  - `src/lib/useBandHistory.ts` — new hook; `useReducer` with push / undo / redo / clear actions; `latestRef` pattern avoids stale closure in callbacks
  - `src/components/EqCanvas.tsx` — added `onBandDragStart` prop; fired once in `handleMouseDown` after hit-test succeeds so a single snapshot is recorded per drag
  - `src/App.tsx` — imported hook; extracted `applyBands()` (no history side-effects); `handleBandsChange` calls `hist.push()` then `applyBands`; `handleBandDragStart` calls `hist.push()` only; `handleCanvasBandChange` calls `applyBands` only; `handleUndo` / `handleRedo` apply snapshots via `applyBands`; keyboard effect registered once using `undoRef` / `redoRef` to avoid stale closures; `hist.clear()` called in `handleSelectProfile` and `handleDeleteProfile`; ↩ / ↪ buttons added to header

- **Band list redesign + drag-to-reorder fix** — complete visual overhaul of the band rows and repaired drag-to-reorder. Rows are now dark cards with a 3 px left accent bar in each band's color, pill enable/disable toggle, SVG grip dots, and a delete × that fades in on hover. The "Add Band" button spans the full width. Drag-to-reorder replaced HTML5 DnD (unreliable in Tauri WebView) with mouse events: `mousedown` on the grip adds `window` `mousemove`/`mouseup` listeners that track which row the cursor is nearest to and highlight it with a colored outline ring; releasing commits the reorder.
  - `src/components/BandRow.tsx` — complete rewrite: 3 px color bar, SVG 6-dot grip, pill toggle, round color swatch, SVG ✕ delete; accepts `isDragging`, `isDragTarget`, `dragTargetColor`, `onGripMouseDown` props
  - `src/App.tsx` — removed `dragIndexRef` and HTML5 DnD handlers; added `rowRefs`, `draggingIndex` state, `startDrag()` function; band list uses `gap-1.5` flex column; empty-state message improved

- **Per-band color customization** — each band has a colored swatch in its row; clicking it opens the native OS color picker to change the band's color. The EQ curve line and fill become a horizontal gradient that flows between all active band colors at their frequency positions. Band dots on the canvas use their individual color. New bands are auto-assigned a distinct color from a 16-color palette. Colors persist with the profile JSON. Old profiles without stored colors get consistent palette fallbacks based on band index.
  - `eq-core/src/filter_type.rs` — `color: Option<String>` field on `BandConfig`; `#[serde(skip_serializing_if = "Option::is_none", default)]` so old profiles deserialize correctly
  - `eq-core/src/preset.rs` — `color: None` added to the internal `band()` helper for builtin presets
  - `src/lib/api.ts` — `color?: string` added to `BandConfig` interface
  - `src/lib/colors.ts` — new module: `BAND_COLORS` palette (16 colors), `bandColor(i, override?)`, `hexToRgba(hex, alpha)` helpers shared by App and EqCanvas
  - `src/App.tsx` — `handleAddBand` assigns the next palette color to each new band
  - `src/components/BandRow.tsx` — color swatch button + hidden `<input type="color">` per band; enable-toggle dot now uses the band's color instead of hardcoded indigo
  - `src/components/EqCanvas.tsx` — `buildBandGradient()` helper; fill uses clip + horizontal gradient + vertical background fade to simulate 2D gradient; line stroke uses the same horizontal gradient; dots use per-band colors with white highlight ring

### Added
- **Spectrum analyzer** — real-time FFT-based frequency display rendered behind the EQ curve on the canvas. Uses a 2048-sample Hann-windowed FFT, mapped to 80 log-spaced bands (20 Hz–20 kHz) with fast-attack/slow-release smoothing. Clears immediately when the engine stops.
  - `eq-audio/src/spectrum.rs` — `SpectrumAnalyzer` struct; all buffers pre-allocated at construction, hot path is allocation-free (satisfies the audio-thread constraint)
  - `eq-audio/src/lib.rs` — `pub mod spectrum`; re-exports `SpectrumAnalyzer` and `SPECTRUM_BANDS`
  - `eq-audio/Cargo.toml` — added `rustfft = "6"`
  - `src-tauri/src/engine.rs` — `spectrum: Arc<Mutex<Vec<f32>>>` field; wired into `start()` via `Arc<Mutex<Option<SpectrumAnalyzer>>>` cell so correct sample_rate is used; reset to −100 dB on `stop()`
  - `src-tauri/src/lib.rs` — `get_spectrum` Tauri command; registered in `invoke_handler`
  - `src/lib/api.ts` — `getSpectrum()` wrapper
  - `src/App.tsx` — `spectrumData` state; 50 ms polling interval while engine is running; passed to `EqCanvas`
  - `src/components/EqCanvas.tsx` — `spectrumData` prop; teal gradient bars drawn before the EQ curve; dims when bypassed

- **Profile rename** — hover any profile row in the sidebar and click the ✎ pencil icon to edit the name inline. Press Enter to confirm or Escape to cancel. All app assignments and the default pointer update automatically. Works on the Default profile too.
  - `eq-core/src/profile.rs` — `rename_profile()` on `ProfileStore`; new `ProfileError::EmptyName` variant
  - `src-tauri/src/lib.rs` — `rename_profile` Tauri command
  - `src/lib/api.ts` — `renameProfile()` wrapper
  - `src/App.tsx` — `handleRenameProfile`; follows `activeName` when the active profile is renamed
  - `src/components/ProfilePanel.tsx` — inline input on pencil click; ✎ and × both appear on hover

- **Single-instance guard** — launching soundEQ a second time (e.g. from the Start Menu while it is already running in the tray) now brings the existing window to the foreground instead of opening a duplicate process.
  - `src-tauri/Cargo.toml` — added `tauri-plugin-single-instance = "2"`
  - `src-tauri/src/lib.rs` — `.plugin(tauri_plugin_single_instance::init(...))` calls `show_main_window` in the already-running process

- **No console window** — removed the `cfg_attr(not(debug_assertions), ...)` guard so the Windows subsystem flag applies in all builds (dev and release). The command prompt no longer appears when the app launches.
  - `src-tauri/src/main.rs` — `windows_subsystem = "windows"` now unconditional

- **Device change detection / auto-restart** — a background thread polls every 2 s for WASAPI thread failures. When a device is unplugged or the audio session is invalidated, the engine restarts automatically. Tries the original device IDs first; if the render device is gone, falls back to the Windows default. Emits `"device-restarted"` (frontend shows a teal notice + refreshes device dropdowns) or `"device-error"` (frontend clears running state + shows error).
  - `eq-audio/src/capture.rs` — `has_crashed()`: thread finished but stop flag never set
  - `eq-audio/src/render.rs` — same
  - `src-tauri/src/engine.rs` — `started_capture_device` / `started_render_device` fields; `has_audio_thread_crashed()`; fields cleared on `stop()`
  - `src-tauri/src/lib.rs` — `device_watch_tick()`; 2-second background thread
  - `src/App.tsx` — `notice` state + `showNotice()`; event listeners for `"device-restarted"` / `"device-error"`; teal notice toast

- **AppPanel collapsed bar — 50% larger** — text, badges, and Manage button scaled up for readability
  - `src/components/AppPanel.tsx` — `h-[60px]→h-[90px]`, labels `text-[10px]→text-[15px]`, button `text-xs→text-[18px]`

- **Apps bar chips: clickable toggle + more spacing** — clicking an app chip in the collapsed bar now toggles its enabled/disabled state directly. Chip gap increased from `gap-1.5` to `gap-3` and padding from `px-2 py-0.5` to `px-2.5 py-1`. Hover states added for all chip variants.
  - `src/components/AppPanel.tsx` — chips changed from `<span>` to `<button>` with `onClick={() => onSetEnabled(processName, !isEnabled)}`; gap and padding increased

- **Apps bar shows inline app chips** — the collapsed Apps bar now lists all assigned apps as pills directly in the bar instead of showing a count badge. The focused app is highlighted emerald with an animated dot. Chips that don't fit are clipped by `overflow-hidden`; the Manage button is always visible on the right. Disabled apps are shown dimmed.
  - `src/components/AppPanel.tsx` — replaced count badge + focused-app display with a flex chip list; `overflow-hidden` container clips excess chips; empty-state hint when no apps are assigned

- **Drag-to-reorder bands** — each band row has a ⠿ grip handle on the left; drag it to reorder bands within the list. Drop target highlights with an indigo ring. Drag is gated to the handle so number inputs inside each row are unaffected.
  - `src/App.tsx` — `dragIndexRef`, `dragHandleActiveRef`, `dragOverIndex` state; `handleReorder(from, to)` splices bands and saves; draggable wrapper divs with HTML5 DnD event handlers around each `BandRow`

- **Resizable EQ canvas** — drag the handle at the bottom of the visualization window to resize it vertically (120–600 px, default 260 px).
  - `src/components/EqCanvas.tsx` — `canvasHeight` state; bottom drag handle div with `cursor-row-resize`; `startHeightDrag` handler using window mousemove/mouseup pattern

- **Draggable band dots on EQ canvas** — click and drag any band marker dot to adjust its frequency (horizontal) and gain (vertical) directly on the canvas. Dots grow and brighten while active. Filter types without gain (LowPass, HighPass, Notch, Bandpass) only allow horizontal dragging. Visual position updates immediately via an optimistic local copy; backend calls are throttled to one per animation frame via `requestAnimationFrame` to avoid IPC flooding. Cursor shows `grab` on hover and `grabbing` while dragging.
  - `src/components/EqCanvas.tsx` — added `xToFreq`/`yToDb` inverse transforms, `hitTestBand`, `dragBandsRef`, `dragRef`, `drawRef`; mouse event handlers on canvas; `onBandChange` prop
  - `src/App.tsx` — passes `onBandChange={handleBandChange}` to EqCanvas

- **EQ canvas sharp rendering** — the visualization is no longer blurry when the window is wide or on high-DPI screens. Canvas backing store now tracks its CSS layout size via `ResizeObserver` and multiplies by `devicePixelRatio`, with `ctx.setTransform(dpr,…)` so all draw calls stay in CSS-pixel coordinates. Canvas CSS height increased from 220 px to 260 px for more vertical space.
  - `src/components/EqCanvas.tsx` — removed hardcoded `width={900} height={220}` attributes; added `ResizeObserver` inside the draw `useEffect`; `draw()` called on mount and on every resize

- **Auto-start engine on launch** — the audio engine starts automatically when the app opens, using the saved device selection. Skips auto-start if no VB-Cable device is detected (shows the setup banner instead) or if the engine is already running (single-instance re-focus path).
  - `src/App.tsx` — added auto-start block at the end of the initial load `useEffect`; calls `startEngine` + `setActiveProfile` when `chosenCapture` and `chosen` output device are both available and engine is not yet running

- **Resizable panels with dynamic text scaling** — the Profiles/Presets sidebar and the Apps bar are now manually resizable; all text and UI elements scale proportionally with the panel dimensions.
  - `src/components/ProfilePanel.tsx` — `width` state (default 192 px, range 140–450); right-edge drag handle (`cursor-col-resize`); all text/button sizes replaced with inline `fontSize` computed as `clamp(10, width × 0.073, 20)`
  - `src/components/AppPanel.tsx` — `barHeight` state (default 90 px, range 60–180); top-edge drag handle (`cursor-row-resize`); bar text and button sizes computed from `barHeight`; dot size scales with bar height



### Added
- **EQ bypass toggle** — amber "Bypass" button in the header passes audio through completely unmodified while the engine is running. The EQ curve dims and a dashed amber 0 dB line appears on the canvas to indicate bypass is active. Resets automatically when the engine is stopped.
  - `src-tauri/src/engine.rs` — `bypassed: bool` field; `set_bypass()` method; `apply_profile()` now respects the bypass flag so the background routing thread cannot accidentally re-enable EQ while bypass is on
  - `src-tauri/src/lib.rs` — `set_eq_bypass` and `is_eq_bypassed` Tauri commands
  - `src/lib/api.ts` — `setEqBypass`, `isEqBypassed` wrappers
  - `src/App.tsx` — `bypassed` state, `handleToggleBypass`, bypass button in header
  - `src/components/EqCanvas.tsx` — `bypassed` prop; dimmed curve + dashed flat line + "BYPASSED" label when active

- **Apps panel** — replaces the old auto-detecting Sessions bar with a manually managed Apps panel (`src/components/AppPanel.tsx`).
  - Apps are added explicitly via a "+" expand form; nothing is added automatically
  - The expand form shows currently playing audio apps as quick-add suggestion chips
  - Each app entry shows its process name, a profile dropdown, and a × remove button
  - Removing an app reverts it to the global default profile

- **Immediate profile apply on assignment** — when the user changes an app's profile in the Apps panel and that app is currently focused, the new profile is applied to the engine immediately rather than waiting for the next focus tick
  - `src-tauri/src/lib.rs` — `apply_app_profile` command applies the profile for a given process name
  - `src/lib/api.ts` — `applyAppProfile` wrapper
  - `src/App.tsx` — `handleAssign` calls `applyAppProfile` when the app matches `focusedApp`

- **Focus-based profile routing** — the per-app EQ profile now switches automatically when the user tabs between applications, based on which window has keyboard focus (`GetForegroundWindow`), rather than relying on WASAPI audio session enumeration order
  - `eq-audio/src/session.rs` — new `get_foreground_process_name()` function using `GetForegroundWindow` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW`
  - `eq-audio/src/lib.rs` — `get_foreground_process_name` added to public exports
  - `eq-audio/Cargo.toml` — added `Win32_UI_WindowsAndMessaging` feature
  - `src-tauri/src/lib.rs` — replaced 3-second `routing_tick` / `select_profile_for_sessions` with 500 ms `focus_tick`; `FocusEvent` struct emitted as `"active-app-changed"` Tauri event; `Emitter` trait imported
  - `src/App.tsx` — `focusedApp` state; `listen("active-app-changed", ...)` subscriber keeps `focusedApp` + `activeName` in sync with Rust focus events
  - `src/components/AppPanel.tsx` — `focusedApp` prop; green dot + emerald highlight on the currently-focused app chip

### Fixed
- **Duplicate app sessions** (`eq-audio/src/session.rs`) — WASAPI can create multiple sessions per process (one per audio stream). Added `HashSet`-based deduplication by `process_name` after enumeration so each app appears only once.

- **Double "Default" in profile dropdown** — the old SessionPanel showed a hardcoded `<option value="">Default</option>` alongside the profile named "Default", producing two identical entries. The new AppPanel shows all profiles directly with no hardcoded option; removing an app from the list is how you revert to the default profile.

- **Per-app routing clarification** — added explanation to README and AppPanel UI that per-app profiles switch the active EQ when apps change audio focus; they cannot apply different EQ to multiple apps simultaneously due to the post-mix loopback architecture

- **Wrong app winning routing** (`eq-audio/src/session.rs`, `src-tauri/src/lib.rs`) — the routing engine previously picked the first session in WASAPI enumeration order, which is arbitrary and unrelated to which app the user is actually using. Replaced entirely with focus-based routing (see "Focus-based profile routing" above). `list_audio_sessions()` now also filters to `AudioSessionStateActive` sessions only, so paused/background processes no longer influence routing.
