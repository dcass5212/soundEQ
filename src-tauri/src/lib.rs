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
// 7. Runs a background thread that automatically applies per-app profiles
//    by matching running audio sessions against the app assignment table
//
// COMMAND GROUPS:
//   Device     — list_render_devices, list_audio_sessions
//   Profiles   — get_profiles, create_profile, delete_profile,
//                update_profile_bands, assign_app, unassign_app,
//                get_builtin_presets
//   Engine     — start_engine, stop_engine, is_engine_running,
//                set_active_profile, get_frequency_response
//   Config     — get_config, set_device_config
//   Startup    — is_startup_enabled, set_startup_enabled
// =============================================================================

use std::{path::PathBuf, sync::Mutex, time::Duration};

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State,
};

mod engine;
mod persistence;
mod startup;

use engine::AudioEngine;
use eq_audio::{
    list_audio_sessions as audio_sessions, list_render_devices as audio_devices,
    AudioDeviceInfo, AudioSessionInfo, StreamFormat,
};
use eq_core::{builtin_presets, BandConfig, Profile, ProfileStore};
use persistence::{AppConfig, load_config, load_profiles, save_config, save_profiles};

// ---------------------------------------------------------------------------
// App state
//
// Everything the app needs across command invocations lives here. Tauri
// injects `State<'_, AppState>` into any command that declares it.
//
// `data_dir` is read-only after setup so it does not need a Mutex.
// ---------------------------------------------------------------------------

struct AppState {
    store: Mutex<ProfileStore>,
    engine: Mutex<AudioEngine>,
    config: Mutex<AppConfig>,
    /// Path to the directory where profiles.json and config.json are stored.
    /// Typically %APPDATA%\com.soundeq.app\ on Windows.
    data_dir: PathBuf,
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
}

// ---------------------------------------------------------------------------
// Engine commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn start_engine(
    capture_device_id: Option<String>,
    device_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<StreamFormat, String> {
    state
        .engine
        .lock()
        .unwrap()
        .start(capture_device_id, device_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_engine(state: State<'_, AppState>) {
    state.engine.lock().unwrap().stop();
}

#[tauri::command]
fn is_engine_running(state: State<'_, AppState>) -> bool {
    state.engine.lock().unwrap().is_running()
}

#[tauri::command]
fn set_active_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    // Clone the profile out of the store before locking the engine,
    // so we never hold both locks at once (would risk deadlock).
    let profile = {
        let store = state.store.lock().unwrap();
        store
            .get_profile(&name)
            .ok_or_else(|| format!("profile '{name}' not found"))?
            .clone()
    };
    state.engine.lock().unwrap().apply_profile(&profile);
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

// ---------------------------------------------------------------------------
// Startup commands
// ---------------------------------------------------------------------------

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
// Per-app profile routing
//
// A background thread (started in setup()) calls routing_tick() every 3 s.
// It lists running audio sessions, finds the first one that has an explicit
// profile assignment in the store, and applies that profile to the engine.
//
// If no assigned app is currently running the active profile is left unchanged.
// This means the user's manual profile selection is respected whenever no
// assigned app is detected.
// ---------------------------------------------------------------------------

/// Checks running sessions and applies the appropriate profile to the engine.
///
/// Called from the background routing thread; never from a command.
fn routing_tick(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();

    // Short-circuit if the engine isn't running — nothing to route.
    if !state.engine.lock().unwrap().is_running() {
        return;
    }

    // List running audio sessions. Failure is silently ignored — the routing
    // thread will try again on its next 3-second tick.
    let sessions = match audio_sessions() {
        Ok(s) => s,
        Err(_) => return,
    };

    // Determine which profile should be active based on running sessions.
    // Clone the profile out of the store before releasing the store lock.
    let target = {
        let store = state.store.lock().unwrap();
        select_profile_for_sessions(&store, &sessions)
        // store lock released here
    };

    if let Some(profile) = target {
        let mut engine = state.engine.lock().unwrap();
        // Only re-apply if the profile actually changed — avoids rebuilding
        // biquad coefficients every 3 s when nothing has changed.
        if engine.active_profile_name != profile.name {
            engine.apply_profile(&profile);
        }
    }
}

/// Returns the profile that should be applied given the currently-running sessions.
///
/// Scans `sessions` in order and returns the profile for the first session whose
/// process name has an explicit entry in the app-assignment table.
/// Returns `None` if no running session has an assignment, meaning the caller
/// should not change the currently active profile.
fn select_profile_for_sessions(
    store: &ProfileStore,
    sessions: &[AudioSessionInfo],
) -> Option<Profile> {
    let assignments = store.app_assignments();
    for session in sessions {
        if session.process_name.is_empty() {
            continue;
        }
        if assignments.contains_key(&session.process_name) {
            return Some(store.profile_for_app(&session.process_name).clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// System tray
//
// Menu items:
//   Open soundEQ           — shows and focuses the main window
//   ──────────
//   [✓] Start with Windows — toggles the HKCU Run registry entry
//   ──────────
//   Quit                   — exits the process
// ---------------------------------------------------------------------------

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
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
            &open_item,
            &PredefinedMenuItem::separator(app)?,
            &startup_item,
            &PredefinedMenuItem::separator(app)?,
            &quit_item,
        ],
    )?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("soundEQ")
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

    Ok(())
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

            // Register shared state. manage() must be called before any
            // commands run, so setup() is the right place.
            app.manage(AppState {
                store: Mutex::new(store),
                engine: Mutex::new(AudioEngine::new()),
                config: Mutex::new(config),
                data_dir,
            });

            // Build the system tray icon and menu.
            setup_tray(app)?;

            // Spawn the background per-app routing thread.
            // AppHandle is Send + Clone, so it can be moved into the thread.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(3));
                routing_tick(&handle);
            });

            if let Some(win) = app.get_webview_window("main") {
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
            start_engine,
            stop_engine,
            is_engine_running,
            set_active_profile,
            get_frequency_response,
            get_config,
            set_device_config,
            is_startup_enabled,
            set_startup_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
