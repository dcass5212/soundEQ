// =============================================================================
// lib.rs — Tauri app: state, IPC commands, and run()
//
// WHAT THIS DOES:
// 1. Defines AppState (ProfileStore + AppConfig + AudioEngine + data directory)
// 2. Exposes Tauri IPC commands to the React frontend
// 3. Persists profiles and config to disk after every mutating command
// 4. Sets up the system tray (open, startup toggle, quit)
// 5. Overrides window close to "hide to tray" instead of quit
// 6. Reads --minimized flag to start hidden when launched at Windows startup
// 7. Runs a background thread (500 ms poll) that detects the foreground window
//    and automatically switches the EQ profile when the focused app changes
//
// COMMAND GROUPS:
//   Device     — list_render_devices, list_audio_sessions
//   Profiles   — get_profiles, create_profile, delete_profile,
//                update_profile_bands, assign_app, unassign_app,
//                set_app_enabled, get_builtin_presets
//   Engine     — start_engine, stop_engine, is_engine_running,
//                set_active_profile, apply_app_profile, get_frequency_response,
//                set_eq_bypass, is_eq_bypassed
//   Config     — get_config, set_device_config
//   Startup    — is_startup_enabled, set_startup_enabled
// =============================================================================

use std::{path::PathBuf, sync::Mutex, time::Duration};

use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};

mod engine;
mod persistence;
mod startup;

use engine::AudioEngine;
use eq_audio::{
    get_foreground_process_name, set_process_volume,
    list_audio_sessions as audio_sessions, list_render_devices as audio_devices,
    AudioDeviceInfo, AudioSessionInfo, StreamFormat,
};
use eq_core::{builtin_presets, BandConfig, CrossfeedConfig, Profile, ProfileStore};
use persistence::{AppConfig, load_config, load_profiles, save_active_profile, save_config, save_profiles};

// ---------------------------------------------------------------------------
// App state
//
// Everything the app needs across command invocations lives here. Tauri
// injects `State<'_, AppState>` into any command that declares it.
//
// `data_dir` is read-only after setup so it does not need a Mutex.
//
// Lock ordering: always acquire `store` before `engine`, or acquire one,
// clone what you need, drop it, then acquire the other. Never hold both
// simultaneously — doing so in different orders across commands deadlocks.
// ---------------------------------------------------------------------------

/// Handles needed to update the system tray appearance at runtime.
///
/// `tray` is ref-counted (cheap to clone); we store it so we can call
/// `set_tooltip` without a global app lookup. `status_item` is the
/// disabled menu item at the top of the context menu that displays the
/// current engine/bypass state as text.
struct TrayHandle {
    tray: TrayIcon<tauri::Wry>,
    status_item: MenuItem<tauri::Wry>,
}

struct AppState {
    store: Mutex<ProfileStore>,
    engine: Mutex<AudioEngine>,
    config: Mutex<AppConfig>,
    /// Path to the directory where profiles.json and config.json are stored.
    /// Typically %APPDATA%\com.soundeq.app\ on Windows.
    data_dir: PathBuf,
    tray: Mutex<TrayHandle>,
}

// ---------------------------------------------------------------------------
// Persistence helpers
//
// These are called after every command that mutates store or config.
// Failures are logged to stderr but do not surface as command errors —
// the app still works fine in-memory even if the disk write fails.
// ---------------------------------------------------------------------------

fn persist_profiles(state: &AppState) {
    let store = state.store.lock().unwrap();
    if let Err(e) = save_profiles(&state.data_dir, &store) {
        eprintln!("soundEQ: failed to save profiles: {e}");
    }
}

fn persist_config(state: &AppState) {
    let config = state.config.lock().unwrap();
    if let Err(e) = save_config(&state.data_dir, &config) {
        eprintln!("soundEQ: failed to save config: {e}");
    }
}

/// Writes the active profile + sample rate to the APO shared state file so
/// the APO DLL (running in audiodg.exe) picks up the new profile on its next
/// LockForProcess call. Non-fatal: the APO falls back to passthrough on miss.
fn persist_active_profile(profile: &Profile, sample_rate: u32) {
    if let Err(e) = save_active_profile(profile, sample_rate) {
        eprintln!("soundEQ: failed to write APO active profile: {e}");
    }
}

// ---------------------------------------------------------------------------
// Tray icon generation
//
// Produces a 32×32 RGBA filled circle for each engine state so the tray icon
// itself changes colour rather than just the tooltip or menu text:
//
//   Gray   (107,114,128) — Stopped:  no EQ running
//   Amber  (245,158, 11) — Bypassed: running but EQ chain is zeroed
//   Green  ( 52,211,153) — Active:   EQ is being applied
//
// 32×32 is chosen so it stays sharp on HiDPI (150%/200%) displays; Windows
// scales it down cleanly at 100% DPI.
// ---------------------------------------------------------------------------

/// Renders a 32×32 anti-aliased filled circle as a heap-owned RGBA image.
///
/// The circle is inset 1.5 px from the icon boundary so it doesn't clip
/// on any DPI scale. The edge is smoothed over a 1 px band to avoid jagged
/// pixels (poor man's anti-aliasing without any image library dependency).
fn make_state_icon(r: u8, g: u8, b: u8) -> tauri::image::Image<'static> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let cx = SIZE as f32 / 2.0;
    let cy = SIZE as f32 / 2.0;
    let outer = SIZE as f32 / 2.0 - 1.5;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            // Linear falloff over the outermost 1 px gives smooth edges.
            let alpha = ((outer + 0.5 - dist).clamp(0.0, 1.0) * 255.0) as u8;
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i]     = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = alpha;
        }
    }

    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

// ---------------------------------------------------------------------------
// Tray status update
//
// Reads the current engine state and rewrites the tray icon, tooltip, and
// status line at the top of the tray context menu simultaneously.
//
// Three visual states:
//   ○ Stopped   — engine not running (no EQ processing)    → gray icon
//   ⊘ Bypassed  — engine running but EQ chain is zeroed    → amber icon
//   ● Active    — engine running and EQ is applied         → green icon
// ---------------------------------------------------------------------------

fn update_tray_status(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();

    let (running, bypassed, profile_name) = {
        let engine = state.engine.lock().unwrap();
        (engine.is_running(), engine.bypassed, engine.active_profile_name.clone())
    };

    let (label, tooltip, icon) = if !running {
        (
            "○  Stopped".to_string(),
            "soundEQ  |  Stopped".to_string(),
            make_state_icon(107, 114, 128), // gray-500
        )
    } else if bypassed {
        (
            "⊘  Bypassed".to_string(),
            "soundEQ  |  Bypassed".to_string(),
            make_state_icon(245, 158, 11),  // amber-400
        )
    } else {
        let name = if profile_name.is_empty() { "Default".to_string() } else { profile_name };
        (
            format!("●  Active  —  {name}"),
            format!("soundEQ  |  {name}"),
            make_state_icon(52, 211, 153),  // emerald-400
        )
    };

    let handle = state.tray.lock().unwrap();
    let _ = handle.status_item.set_text(&label);
    let _ = handle.tray.set_tooltip(Some(tooltip));
    let _ = handle.tray.set_icon(Some(icon));
}

// ---------------------------------------------------------------------------
// Device commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_render_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    audio_devices().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_audio_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    audio_sessions().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Profile commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_profiles(state: State<'_, AppState>) -> ProfileStore {
    state.store.lock().unwrap().clone()
}

#[tauri::command]
fn get_builtin_presets() -> Vec<Profile> {
    builtin_presets()
}

#[tauri::command]
fn create_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        store.add_profile(Profile::new(name)).map_err(|e| e.to_string())?;
    }
    persist_profiles(&state);
    Ok(())
}

#[tauri::command]
fn delete_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        store.remove_profile(&name).map(|_| ()).map_err(|e| e.to_string())?;
    }
    persist_profiles(&state);
    Ok(())
}

#[tauri::command]
fn update_profile_bands(
    name: String,
    bands: Vec<BandConfig>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    Profile::with_bands(name.clone(), bands.clone())
        .validate()
        .map_err(|e| e.to_string())?;

    {
        let mut store = state.store.lock().unwrap();
        let profile = store
            .get_profile_mut(&name)
            .ok_or_else(|| format!("profile '{name}' not found"))?;
        profile.bands = bands;
    }
    persist_profiles(&state);
    Ok(())
}

#[tauri::command]
fn assign_app(
    process_name: String,
    profile_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        store
            .assign_app(&process_name, &profile_name)
            .map_err(|e| e.to_string())?;
    }
    persist_profiles(&state);
    Ok(())
}

#[tauri::command]
fn unassign_app(process_name: String, state: State<'_, AppState>) {
    state.store.lock().unwrap().unassign_app(&process_name);
    persist_profiles(&state);
    // Reset the WASAPI session volume to 1.0 when an app is removed so it
    // returns to its normal volume immediately rather than staying at the
    // stored level until the next restart.
    let _ = set_process_volume(&process_name, 1.0);
}

/// Sets the per-session WASAPI volume for a named application and persists it
/// to the profile store so the slider position is remembered across sessions.
///
/// `volume` is a linear scalar in [0.0, 1.0]: 1.0 = full session volume,
/// 0.0 = silent. Changes take effect immediately on the live WASAPI session.
/// No-op for the WASAPI call when the app has no active audio session.
#[tauri::command]
fn set_app_volume(process_name: String, volume: f32, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        store.set_app_volume(&process_name, volume);
    }
    persist_profiles(&state);
    set_process_volume(&process_name, volume).map_err(|e| e.to_string())
}

/// Enables or disables automatic profile switching for a specific app.
///
/// Disabled apps keep their profile assignment so it is not lost, but
/// focus_tick will not apply it when that app is focused. Re-enabling
/// immediately resumes auto-switching on the next focus tick.
/// Renames an existing profile, updating all app assignments and the default
/// pointer automatically. Returns an error if the new name is blank or taken.
#[tauri::command]
fn rename_profile(old_name: String, new_name: String, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut store = state.store.lock().unwrap();
        store.rename_profile(&old_name, &new_name).map_err(|e| e.to_string())?;
    }
    persist_profiles(&state);
    Ok(())
}

#[tauri::command]
fn set_app_enabled(process_name: String, enabled: bool, state: State<'_, AppState>) {
    state.store.lock().unwrap().set_app_enabled(&process_name, enabled);
    persist_profiles(&state);
}

/// Updates the crossfeed configuration for the named profile and persists it.
///
/// If the profile is currently active and the engine is running, the new
/// crossfeed config takes effect on the very next audio buffer without any
/// restart or glitch.
///
/// Lock ordering: store lock is dropped before engine lock is acquired.
#[tauri::command]
fn set_profile_crossfeed(
    profile_name: String,
    config: CrossfeedConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Update the store first.
    {
        let mut store = state.store.lock().unwrap();
        let profile = store
            .get_profile_mut(&profile_name)
            .ok_or_else(|| format!("profile '{profile_name}' not found"))?;
        profile.crossfeed = config.clone();
    }
    persist_profiles(&state);

    // Apply to the live engine if this is the currently active profile.
    // Takes effect on the next audio buffer with no dropout.
    {
        let engine = state.engine.lock().unwrap();
        if engine.is_running() && engine.active_profile_name == profile_name {
            engine.apply_crossfeed_config(&config);
        }
    }

    Ok(())
}

/// Serializes a single profile to a pretty-printed JSON string.
///
/// The frontend receives this string and triggers a file download — no file
/// I/O happens in Rust. The exported format is the Profile struct itself
/// (name, bands, crossfeed), which is also the import format.
#[tauri::command]
fn export_profile(name: String, state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().unwrap();
    let profile = store
        .get_profile(&name)
        .ok_or_else(|| format!("profile '{name}' not found"))?;
    serde_json::to_string_pretty(profile).map_err(|e| e.to_string())
}

/// Parses a profile JSON string and adds it to the store.
///
/// If a profile with the same name already exists, a numeric suffix is
/// appended (" (2)", " (3)", …) to avoid collision. Returns the final
/// name the profile was saved under so the frontend can select it.
#[tauri::command]
fn import_profile(json: String, state: State<'_, AppState>) -> Result<String, String> {
    let mut profile: Profile = serde_json::from_str(&json)
        .map_err(|e| format!("invalid profile JSON: {e}"))?;
    profile.validate().map_err(|e| e.to_string())?;

    let base_name = profile.name.trim().to_string();
    if base_name.is_empty() {
        return Err("profile name cannot be empty".to_string());
    }

    let final_name = {
        let mut store = state.store.lock().unwrap();
        // Resolve name collision: append an incrementing counter suffix.
        let resolved = if store.get_profile(&base_name).is_none() {
            base_name
        } else {
            let mut n = 2u32;
            loop {
                let candidate = format!("{base_name} ({n})");
                if store.get_profile(&candidate).is_none() {
                    break candidate;
                }
                n += 1;
            }
        };
        profile.name = resolved.clone();
        store.add_profile(profile).map_err(|e| e.to_string())?;
        resolved
    };

    persist_profiles(&state);
    Ok(final_name)
}

// ---------------------------------------------------------------------------
// Engine commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn start_engine(
    capture_device_id: Option<String>,
    device_id: Option<String>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<StreamFormat, String> {
    let result = state
        .engine
        .lock()
        .unwrap()
        .start(capture_device_id, device_id)
        .map_err(|e| e.to_string())?;
    update_tray_status(&app);
    Ok(result)
}

#[tauri::command]
fn stop_engine(state: State<'_, AppState>, app: tauri::AppHandle) {
    state.engine.lock().unwrap().stop();
    update_tray_status(&app);
}

#[tauri::command]
fn is_engine_running(state: State<'_, AppState>) -> bool {
    state.engine.lock().unwrap().is_running()
}

#[tauri::command]
fn set_active_profile(name: String, state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    // Clone the profile out of the store before locking the engine,
    // so we never hold both locks at once (would risk deadlock).
    let profile = {
        let store = state.store.lock().unwrap();
        store
            .get_profile(&name)
            .ok_or_else(|| format!("profile '{name}' not found"))?
            .clone()
    };
    let sample_rate = {
        let mut engine = state.engine.lock().unwrap();
        engine.apply_profile(&profile);
        engine.sample_rate
    };
    persist_active_profile(&profile, sample_rate);
    update_tray_status(&app);
    Ok(())
}

/// Immediately applies the profile assigned to `process_name`.
///
/// Called by the frontend when the user explicitly changes an app's profile —
/// makes the change take effect instantly rather than waiting for the next
/// focus tick to detect a window switch.
#[tauri::command]
fn apply_app_profile(process_name: String, state: State<'_, AppState>) -> Result<(), String> {
    let profile = {
        let store = state.store.lock().unwrap();
        store.profile_for_app(&process_name).clone()
    };
    let sample_rate = {
        let mut engine = state.engine.lock().unwrap();
        engine.apply_profile(&profile);
        engine.sample_rate
    };
    persist_active_profile(&profile, sample_rate);
    Ok(())
}

#[tauri::command]
fn get_frequency_response(
    name: String,
    state: State<'_, AppState>,
) -> Result<Vec<(f64, f64)>, String> {
    let profile = {
        let store = state.store.lock().unwrap();
        store
            .get_profile(&name)
            .ok_or_else(|| format!("profile '{name}' not found"))?
            .clone()
    };
    let sample_rate = state.engine.lock().unwrap().sample_rate as f64;
    let chain = profile.to_filter_chain(sample_rate);
    Ok(chain.frequency_response_curve(256))
}

/// Shared bypass logic used by the IPC command and the global hotkey handler.
///
/// `enabled` = true → bypass on (silence the EQ chain).
/// `enabled` = false → bypass off (re-apply the active profile).
///
/// Emits "bypass-changed" so the frontend button stays in sync regardless
/// of whether the change came from the UI or the hotkey.
///
/// Lock ordering: store lock is always dropped before engine lock is acquired.
fn apply_bypass(enabled: bool, state: &AppState, app: &tauri::AppHandle) {
    if enabled {
        state.engine.lock().unwrap().set_bypass(true);
    } else {
        let active_name = {
            let mut engine = state.engine.lock().unwrap();
            engine.set_bypass(false);
            engine.active_profile_name.clone()
        };
        let profile = {
            let store = state.store.lock().unwrap();
            store.get_profile(&active_name).cloned()
        };
        if let Some(p) = profile {
            state.engine.lock().unwrap().apply_profile(&p);
        }
    }
    // bypassed == enabled after either branch above; no need for a third lock.
    let bypassed = enabled;
    // Emit directly to the "main" window rather than broadcasting via app.emit().
    // app.emit() can fail silently when called from non-tokio threads (e.g. the
    // global shortcut OS callback thread); window.emit() uses a direct channel
    // that works regardless of which thread the caller is on.
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.emit("bypass-changed", bypassed);
    }
    update_tray_status(app);
}

/// Enables or disables the global EQ bypass.
///
/// Lock ordering: never hold store and engine locks simultaneously.
/// We follow the same pattern as set_active_profile — clone what we need
/// before acquiring the second lock.
#[tauri::command]
fn set_eq_bypass(enabled: bool, state: State<'_, AppState>, app: tauri::AppHandle) -> Result<(), String> {
    apply_bypass(enabled, &state, &app);
    Ok(())
}

/// Returns true if the global EQ bypass is currently active.
#[tauri::command]
fn is_eq_bypassed(state: State<'_, AppState>) -> bool {
    state.engine.lock().unwrap().bypassed
}

/// Updates the live filter chain in-place from `bands`, without persisting
/// anything or changing `active_profile_name`.
///
/// Used by the solo/mute feature and canvas drag: the frontend computes which
/// bands are effectively silenced, sets their `enabled` flag to false, and
/// calls this command so the audio thread immediately hears only the
/// un-silenced bands. The stored profile is unchanged — unmuting/releasing
/// the drag restores the full profile via `set_active_profile`.
///
/// WHY in-place (set_bands) instead of a full chain swap:
/// Swapping in a new FilterChain resets all biquad delay lines (x1,x2,y1,y2)
/// to zero. The audio thread then processes the next packet with no sample
/// history, producing a discontinuity — audible as static or clicks. Calling
/// set_bands() on the existing chain calls update_coefficients() per band,
/// which explicitly preserves the delay lines so the filter transitions
/// smoothly from the old response to the new one.
///
/// Lock ordering: engine lock first, then filter_chain lock — same ordering as
/// apply_profile in engine.rs. The audio thread only ever takes filter_chain,
/// never engine, so there is no deadlock risk.
///
/// No-op when the engine is not running or bypass is active.
#[tauri::command]
fn apply_bands_live(bands: Vec<BandConfig>, state: State<'_, AppState>) -> Result<(), String> {
    let engine = state.engine.lock().unwrap();
    if !engine.is_running() || engine.bypassed {
        return Ok(());
    }
    // Update coefficients in-place — delay line state is preserved, no clicks.
    engine.filter_chain.lock().unwrap().set_bands(&bands);
    Ok(())
}

/// Computes the frequency response curve for an arbitrary set of bands
/// without looking up a profile from the store.
///
/// Used when solo/mute is active so the EQ canvas shows the effective curve
/// (only the live, un-silenced bands) instead of the stored profile's curve.
///
/// Returns 256 (frequency_hz, gain_db) pairs, log-spaced 20 Hz–20 kHz.
#[tauri::command]
fn get_frequency_response_live(
    bands: Vec<BandConfig>,
    state: State<'_, AppState>,
) -> Vec<(f64, f64)> {
    let sample_rate = state.engine.lock().unwrap().sample_rate as f64;
    let profile = Profile::with_bands("__live__".to_string(), bands);
    let chain   = profile.to_filter_chain(sample_rate);
    chain.frequency_response_curve(256)
}

/// Returns the latest spectrum magnitude data as SPECTRUM_BANDS dB values.
///
/// Values are log-spaced from 20 Hz to 20 kHz. Each element is a dB magnitude
/// (negative; roughly −60 to −10 dB for typical audio; −100 dB = silence).
/// The frontend polls this at ~20 Hz to drive the spectrum display.
#[tauri::command]
fn get_spectrum(state: State<'_, AppState>) -> Vec<f32> {
    state.engine.lock().unwrap().spectrum.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

/// Returns the current persisted app configuration (e.g. saved device ID).
#[tauri::command]
fn get_config(state: State<'_, AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

/// Saves both audio device selections so they are restored on next launch.
#[tauri::command]
fn set_device_config(
    device_id: Option<String>,
    capture_device_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut config = state.config.lock().unwrap();
        config.last_device_id = device_id;
        config.capture_device_id = capture_device_id;
    }
    persist_config(&state);
    Ok(())
}

/// Returns the current output gain (linear, [0.0, 4.0]).
#[tauri::command]
fn get_output_gain(state: State<'_, AppState>) -> f32 {
    state.engine.lock().unwrap().get_output_gain()
}

/// Sets the output gain and persists it to config.json.
///
/// Takes effect on the next render buffer — no dropout or glitch.
/// `gain` is a linear multiplier: 1.0 = unity, 1.5 = +3.5 dB, 2.0 = +6 dB.
#[tauri::command]
fn set_output_gain(gain: f32, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut engine = state.engine.lock().unwrap();
        engine.set_output_gain(gain);
    }
    {
        let mut config = state.config.lock().unwrap();
        config.output_gain = gain.clamp(0.0, 4.0);
    }
    persist_config(&state);
    Ok(())
}

// ---------------------------------------------------------------------------
// Startup commands
// ---------------------------------------------------------------------------

/// Opens the VB-Audio Virtual Cable download page in the default system browser.
///
/// Uses `cmd /c start` — the standard Windows shell verb for launching URLs.
/// Fire-and-forget: errors are silently ignored since the UI can't do anything
/// useful if the shell fails to open a browser.
#[tauri::command]
fn open_vb_cable_download() {
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", "https://vb-audio.com/Cable/index.htm"])
        .spawn();
}

/// Returns true if this process was launched by the Windows startup entry
/// (i.e. was invoked with the --minimized flag).
///
/// The frontend uses this to add a 10-second delay before auto-starting the
/// audio engine, giving the Windows audio subsystem time to fully initialize
/// before soundEQ tries to open a WASAPI loopback session. Without the delay,
/// the engine start can silently fail on some systems at login time.
#[tauri::command]
fn is_startup_launch() -> bool {
    std::env::args().any(|a| a == "--minimized")
}

/// Returns true if soundEQ is registered to launch at Windows startup.
#[tauri::command]
fn is_startup_enabled() -> bool {
    startup::is_enabled()
}

/// Enables or disables the Windows startup registry entry.
#[tauri::command]
fn set_startup_enabled(enabled: bool) -> Result<(), String> {
    if enabled {
        startup::enable().map_err(|e| e.to_string())
    } else {
        startup::disable().map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Focus-based per-app profile routing
//
// A background thread (started in setup()) calls focus_tick() every 500 ms.
// It reads the foreground window's process name and, if that process has an
// explicit profile assignment in the store, applies that profile to the engine.
//
// This gives a seamless experience: tabbing from Spotify to Chrome automatically
// switches the EQ profile without any manual action.
//
// If the focused app has no assignment the engine's current profile is left
// unchanged — so the user's manual profile selection is always respected.
// ---------------------------------------------------------------------------

/// Payload emitted to the frontend when the focused app changes.
/// The frontend uses this to highlight the currently-focused app in the UI.
#[derive(serde::Serialize, Clone)]
struct FocusEvent {
    /// Executable name of the focused process (e.g. "chrome.exe").
    /// Empty string when no foreground window is detectable.
    process_name: String,
    /// The engine's current active profile name after this focus transition.
    /// Always Some when the engine is running so the frontend EQ viewer
    /// stays in sync even when the focused app has no explicit assignment.
    profile_name: Option<String>,
}

/// Detects the foreground window, switches EQ profile if the focused app has
/// an assignment, and emits an "active-app-changed" event to the frontend.
///
/// `last_focused` tracks the previously-seen process name so we only act on
/// transitions — no profile rebuild or event emission when focus hasn't moved.
fn focus_tick(app: &tauri::AppHandle, last_focused: &mut String) {
    let state = app.state::<AppState>();

    // Short-circuit if the engine isn't running — nothing to route.
    if !state.engine.lock().unwrap().is_running() {
        return;
    }

    let current = get_foreground_process_name().unwrap_or_default();

    // No change in focus — skip the profile lookup and event emission entirely.
    if current == *last_focused {
        return;
    }
    *last_focused = current.clone();

    // Look up whether the newly-focused app has an enabled profile assignment.
    // Clone the profile out before releasing the store lock.
    let profile = if !current.is_empty() {
        let store = state.store.lock().unwrap();
        if store.app_assignments().contains_key(&current) && store.is_app_enabled(&current) {
            Some(store.profile_for_app(&current).clone())
        } else {
            None
        }
        // store lock released here
    } else {
        None
    };

    // Apply the profile if one was found, but only when it differs from the
    // currently active profile — avoids unnecessary biquad coefficient rebuilds.
    // Capture the engine's active profile name after any switch so we can
    // send it to the frontend even when no assignment exists for this app.
    let mut profile_switched = false;
    let mut switched_sample_rate: u32 = 0;
    let active_profile_name: Option<String> = {
        let mut engine = state.engine.lock().unwrap();
        if let Some(ref p) = profile {
            if engine.active_profile_name != p.name {
                engine.apply_profile(p);
                profile_switched = true;
                switched_sample_rate = engine.sample_rate;
            }
        }
        // Always read the current name so the frontend knows what's active
        // even when the focused app has no assignment and no switch occurred.
        if engine.active_profile_name.is_empty() {
            None
        } else {
            Some(engine.active_profile_name.clone())
        }
    };

    // Notify the frontend of the focus change regardless of whether a profile
    // was applied — the UI uses this to show which app is currently focused
    // and to keep the EQ viewer in sync with the running engine.
    let _ = app.emit("active-app-changed", FocusEvent {
        process_name: current,
        profile_name: active_profile_name,
    });

    // Keep the tray tooltip and status item in sync when the active profile
    // changes (e.g. user tabs from Spotify to Chrome with different profiles).
    if profile_switched {
        if let Some(ref p) = profile {
            persist_active_profile(p, switched_sample_rate);
        }
        update_tray_status(app);
    }
}

// ---------------------------------------------------------------------------
// Device change detection + auto-restart
//
// A background thread polls every 2 seconds for signs of a WASAPI failure.
// When either the capture or render thread exits unexpectedly (device
// unplugged, audio session invalidated, etc.) device_watch_tick() is called.
//
// Restart strategy:
//   1. Try the same device IDs the engine was started with.
//   2. If that fails (device gone), fall back to None for the render side
//      so WASAPI picks the new system default.
//   3. Emit "device-restarted" on success (frontend refreshes device list),
//      or "device-error" on failure (frontend shows an error and clears the
//      running state so the user can manually restart).
// ---------------------------------------------------------------------------

fn device_watch_tick(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();

    // Snapshot the engine state under a short-lived lock.
    let (crashed, capture_dev, render_dev, active_name) = {
        let engine = state.engine.lock().unwrap();
        if !engine.is_running() { return; }
        (
            engine.has_audio_thread_crashed(),
            engine.started_capture_device.clone(),
            engine.started_render_device.clone(),
            engine.active_profile_name.clone(),
        )
    };

    if !crashed { return; }

    eprintln!("[soundEQ] audio thread crashed — attempting auto-restart");

    // Clone the active profile BEFORE locking the engine (lock-ordering rule:
    // never hold store and engine simultaneously).
    let profile = {
        let store = state.store.lock().unwrap();
        store.get_profile(&active_name).cloned()
    };

    // Stop and restart under the engine lock.
    // First attempt: same device IDs as before.
    // Second attempt: keep the capture device (virtual cable doesn't change)
    // but fall back to None for render so WASAPI uses the new default output.
    let result = {
        let mut engine = state.engine.lock().unwrap();
        engine.stop();
        let first = engine.start(capture_dev.clone(), render_dev);
        if first.is_err() {
            engine.start(capture_dev, None)
        } else {
            first
        }
    };

    match result {
        Ok(_) => {
            // Re-apply the active EQ profile on the fresh engine.
            if let Some(p) = profile {
                state.engine.lock().unwrap().apply_profile(&p);
            }
            // Tell the frontend to refresh its device list — the active
            // device may have changed (e.g. headphones → speakers fallback).
            let _ = app.emit("device-restarted", ());
            update_tray_status(app);
        }
        Err(e) => {
            eprintln!("[soundEQ] auto-restart failed: {e}");
            // Engine is stopped; tell the frontend so it updates the UI.
            let _ = app.emit("device-error", e.to_string());
            update_tray_status(app); // shows "○ Stopped" since engine is now stopped
        }
    }
}

// ---------------------------------------------------------------------------
// System tray
//
// Menu items:
//   ○ Stopped              — status line (disabled; updated dynamically)
//   ──────────
//   Open soundEQ           — shows and focuses the main window
//   ──────────
//   [✓] Start with Windows — toggles the HKCU Run registry entry
//   ──────────
//   Quit                   — exits the process
// ---------------------------------------------------------------------------

/// Builds the system tray icon and context menu.
///
/// Returns the live `TrayIcon` handle and the `status_item` so the caller can
/// store them in `AppState` and update them dynamically via `update_tray_status`.
fn setup_tray(app: &mut tauri::App) -> tauri::Result<(TrayIcon<tauri::Wry>, MenuItem<tauri::Wry>)> {
    // Status item — always disabled; text is rewritten by update_tray_status
    // to reflect whether the engine is stopped, bypassed, or actively EQ'ing.
    let status_item = MenuItem::with_id(app, "status", "○  Stopped", false, None::<&str>)?;
    // Clone so the returned handle and the menu reference point to the same item.
    let status_ref = status_item.clone();

    let open_item = MenuItem::with_id(app, "open", "Open soundEQ", true, None::<&str>)?;

    // CheckMenuItem reflects and toggles the startup registration state.
    let startup_item = CheckMenuItem::with_id(
        app,
        "startup",
        "Start with Windows",
        true,
        startup::is_enabled(),
        None::<&str>,
    )?;
    // Clone so the on_menu_event closure can read/write its checked state.
    // Tauri menu items are reference-counted internally, so clone is cheap.
    let startup_ref = startup_item.clone();

    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &PredefinedMenuItem::separator(app)?,
            &open_item,
            &PredefinedMenuItem::separator(app)?,
            &startup_item,
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )?;

    let tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("soundEQ  |  Stopped")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show_main_window(app),

            "startup" => {
                let new_state = !startup_ref.is_checked().unwrap_or(false);
                let _ = startup_ref.set_checked(new_state);
                if new_state {
                    let _ = startup::enable();
                } else {
                    let _ = startup::disable();
                }
            }

            "quit" => app.exit(0),

            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click the tray icon → show the window (same as "Open").
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok((tray, status_ref))
}

/// Shows, unminimizes, and focuses the main application window.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() {
    // The --minimized flag is set in the Windows startup registry entry so
    // the app can skip showing its window when launched at login.
    let start_hidden = std::env::args().any(|a| a == "--minimized");

    tauri::Builder::default()
        // Single-instance guard: if a second soundEQ process is launched (e.g.
        // from the Start Menu while the app is already running in the tray),
        // the plugin intercepts it, fires this callback in the existing process,
        // and exits the new one. We just bring the window to the foreground.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(
            // Register the bypass hotkey at plugin-init time (before the app event
            // loop starts) so we avoid the run_on_main_thread deadlock that occurs
            // when registering shortcuts inside setup().
            // with_shortcut + with_handler is the correct builder pattern for Rust-side-only use.
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcut(
                    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyB),
                )
                .expect("bypass shortcut (Ctrl+Shift+B) is a valid key combination")
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let state = app.state::<AppState>();
                        let currently_bypassed = state.engine.lock().unwrap().bypassed;
                        apply_bypass(!currently_bypassed, &state, app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            // Resolve the per-user app data directory.
            // On Windows this is typically %APPDATA%\com.soundeq.app\.
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data directory");

            // Load previously saved state from disk. Both functions return safe
            // defaults when their file is absent (first launch) or corrupt.
            let store = load_profiles(&data_dir);
            let config = load_config(&data_dir);

            // Build the tray icon first so we get back the live TrayIcon and
            // status MenuItem handles, which we store in AppState so any command
            // can call update_tray_status() to reflect engine/bypass changes.
            let (tray_icon, status_item) = setup_tray(app)?;

            // Build the engine and immediately apply the saved output gain so
            // the user's boost setting is active before the first Start click.
            let mut engine = AudioEngine::new();
            engine.set_output_gain(config.output_gain);

            // Register shared state. manage() must be called before any
            // commands run, so setup() is the right place.
            app.manage(AppState {
                store: Mutex::new(store),
                engine: Mutex::new(engine),
                config: Mutex::new(config),
                data_dir,
                tray: Mutex::new(TrayHandle { tray: tray_icon, status_item }),
            });

            // Spawn the background focus-routing thread.
            // Polls every 250 ms — fast enough to feel instant when tabbing
            // between apps, cheap enough to not matter in a sleep loop.
            // AppHandle is Send + Clone, so it can be moved into the thread.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last_focused = String::new();
                loop {
                    std::thread::sleep(Duration::from_millis(250));
                    focus_tick(&handle, &mut last_focused);
                }
            });

            // Spawn the device-watch thread.
            // Polls every 2 s — frequent enough to detect a disconnected device
            // within two seconds, but rare enough to be negligible overhead.
            let watch_handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    device_watch_tick(&watch_handle);
                }
            });

            if let Some(win) = app.get_webview_window("main") {
                // Push the app icon onto the window so Task Manager, Alt+Tab,
                // and the taskbar button all show the correct icon. WebView2
                // does not inherit the exe's embedded resource icon automatically,
                // so without this call Windows shows a blank placeholder.
                if let Some(icon) = app.default_window_icon() {
                    let _ = win.set_icon(icon.clone());
                }

                // Show the window on a normal (non-startup) launch.
                if !start_hidden {
                    win.show()?;
                }

                // Override window close: hide to tray instead of quitting.
                // This lets the EQ keep running in the background while the
                // user dismisses the window.
                let win_hide = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_hide.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_render_devices,
            list_audio_sessions,
            get_profiles,
            get_builtin_presets,
            create_profile,
            delete_profile,
            update_profile_bands,
            assign_app,
            unassign_app,
            set_app_volume,
            rename_profile,
            set_app_enabled,
            set_profile_crossfeed,
            export_profile,
            import_profile,
            start_engine,
            stop_engine,
            is_engine_running,
            set_active_profile,
            apply_app_profile,
            get_frequency_response,
            set_eq_bypass,
            is_eq_bypassed,
            apply_bands_live,
            get_frequency_response_live,
            get_config,
            set_device_config,
            get_output_gain,
            set_output_gain,
            open_vb_cable_download,
            is_startup_launch,
            is_startup_enabled,
            set_startup_enabled,
            get_spectrum,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
