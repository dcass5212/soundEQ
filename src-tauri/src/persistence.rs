// =============================================================================
// persistence.rs — Save and load app state to disk
//
// WHAT THIS DOES:
// Provides two independent persistence units:
//
//   ProfileStore  → <app data dir>/profiles.json
//     All user-created profiles, app assignments, and the default profile name.
//
//   AppConfig     → <app data dir>/config.json
//     Lightweight user preferences that survive profile resets (e.g. which
//     audio device was last selected).
//
// Both functions are infallible on load: if the file is missing or corrupt
// they return a sensible default and let the app start cleanly. Saves can
// fail (e.g., disk full) — callers log the error but don't panic.
//
// The app data directory is supplied by the caller (from Tauri's path API)
// so this module has no Tauri dependency and can be tested without a real app.
// =============================================================================

use std::{fs, io, path::Path};

use eq_core::{BandConfig, Profile, ProfileStore};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AppConfig — persisted user preferences
// ---------------------------------------------------------------------------

fn default_output_gain() -> f32 { 1.0 }

/// Lightweight user preferences persisted across restarts.
///
/// Kept separate from ProfileStore so config can be saved/loaded independently
/// without touching the (potentially larger) profile data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// The WASAPI render endpoint ID the user last selected (real speakers/headphones).
    ///
    /// `None` means "use the system-default device on next launch."
    /// Device IDs are stable across reboots for the same hardware.
    #[serde(default)]
    pub last_device_id: Option<String>,

    /// The WASAPI render endpoint ID to capture from via loopback.
    ///
    /// This should be the virtual cable device (e.g. "CABLE Input (VB-Audio Virtual Cable)")
    /// that apps route their audio to. `None` captures from the system-default output.
    #[serde(default)]
    pub capture_device_id: Option<String>,

    /// Linear output gain applied in the render loop (multiplied with the
    /// VolumeMonitor scalar). Default 1.0. Range [0.0, 4.0].
    ///
    /// Compensates for VB-Cable's inherent lower signal level vs. direct
    /// device output — the user sets this once and forgets it.
    #[serde(default = "default_output_gain")]
    pub output_gain: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            last_device_id: None,
            capture_device_id: None,
            output_gain: default_output_gain(),
        }
    }
}

// ---------------------------------------------------------------------------
// File names
// ---------------------------------------------------------------------------

const PROFILES_FILE: &str = "profiles.json";
const CONFIG_FILE: &str = "config.json";

// ---------------------------------------------------------------------------
// Profile persistence
// ---------------------------------------------------------------------------

/// Writes `store` as pretty-printed JSON to `<dir>/profiles.json`.
///
/// Creates `dir` and any parent directories that don't yet exist.
/// Returns an error on I/O failure (e.g. disk full).
pub fn save_profiles(dir: &Path, store: &ProfileStore) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(store)
        .map_err(io::Error::other)?;
    fs::write(dir.join(PROFILES_FILE), json)
}

/// Reads `<dir>/profiles.json` and deserializes a ProfileStore.
///
/// Returns `ProfileStore::new()` (a single flat "Default" profile) if:
/// - The file does not exist (first launch).
/// - The file is not valid JSON (e.g., corrupted).
///
/// This means soundEQ will always start with at least one usable profile.
pub fn load_profiles(dir: &Path) -> ProfileStore {
    let Ok(bytes) = fs::read(dir.join(PROFILES_FILE)) else {
        return ProfileStore::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|_| ProfileStore::new())
}

// ---------------------------------------------------------------------------
// Config persistence
// ---------------------------------------------------------------------------

/// Writes `config` as pretty-printed JSON to `<dir>/config.json`.
pub fn save_config(dir: &Path, config: &AppConfig) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(config)
        .map_err(io::Error::other)?;
    fs::write(dir.join(CONFIG_FILE), json)
}

/// Reads `<dir>/config.json` and deserializes an AppConfig.
///
/// Returns `AppConfig::default()` if the file is absent or unreadable.
pub fn load_config(dir: &Path) -> AppConfig {
    let Ok(bytes) = fs::read(dir.join(CONFIG_FILE)) else {
        return AppConfig::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// APO shared state
//
// When the active EQ profile changes in the Tauri app, we write a small JSON
// file to a machine-wide path that the APO DLL (running inside audiodg.exe)
// can read on its next Initialize or LockForProcess call.
//
// %PUBLIC% (C:\Users\Public) is chosen because:
//   - It is writable by all authenticated users (the Tauri process).
//   - It is readable by LOCAL SERVICE / SYSTEM (audiodg.exe's identity).
//   - It persists across sessions (unlike %TEMP%).
// ---------------------------------------------------------------------------

const APO_STATE_FILE: &str = "active_profile.json";

/// Returns the path where the APO shared state file is written.
/// Typically `C:\Users\Public\soundEQ\active_profile.json`.
pub fn apo_config_path() -> std::path::PathBuf {
    let public = std::env::var("PUBLIC").unwrap_or_else(|_| r"C:\Users\Public".into());
    std::path::Path::new(&public).join("soundEQ").join(APO_STATE_FILE)
}

/// JSON written to the APO shared state file.
/// The sample rate is included so the APO does not need to extract it from
/// the WAVEFORMATEX descriptor in LockForProcess — it already knows the
/// correct value from the running WASAPI session.
#[derive(Serialize)]
struct ApoStateFile<'a> {
    name: &'a str,
    bands: &'a [BandConfig],
    sample_rate: u32,
}

/// Writes `profile` and `sample_rate` to the APO shared state file.
///
/// Called whenever the active profile changes. The APO reads this on its
/// next Initialize (DLL load) or LockForProcess (stream restart) call.
/// Failures are non-fatal — the APO falls back to passthrough silently.
pub fn save_active_profile(profile: &Profile, sample_rate: u32) -> io::Result<()> {
    let path = apo_config_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = ApoStateFile { name: &profile.name, bands: &profile.bands, sample_rate };
    let json = serde_json::to_string_pretty(&file)
        .map_err(io::Error::other)?;
    fs::write(&path, json)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use eq_core::{Profile, DEFAULT_PROFILE_NAME};

    /// Creates a unique temp subdirectory for a single test.
    fn test_dir(name: &str) -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("soundeq_test_{name}"));
        let _ = fs::remove_dir_all(&d); // clear any leftover from a prior run
        d
    }

    #[test]
    fn roundtrip_config_with_device() {
        let dir = test_dir("roundtrip_config");
        let config = AppConfig {
            last_device_id: Some("WASAPI-{abc-123}".into()),
            capture_device_id: None,
        };
        save_config(&dir, &config).unwrap();
        let loaded = load_config(&dir);
        assert_eq!(loaded.last_device_id.as_deref(), Some("WASAPI-{abc-123}"));
    }

    #[test]
    fn roundtrip_config_no_device() {
        let dir = test_dir("roundtrip_config_null");
        save_config(&dir, &AppConfig::default()).unwrap();
        let loaded = load_config(&dir);
        assert!(loaded.last_device_id.is_none());
    }

    #[test]
    fn roundtrip_profiles_preserves_custom_profiles() {
        let dir = test_dir("roundtrip_profiles");
        let mut store = ProfileStore::new();
        store.add_profile(Profile::new("Gaming")).unwrap();
        store.add_profile(Profile::new("Vocal")).unwrap();
        store.assign_app("game.exe", "Gaming").unwrap();

        save_profiles(&dir, &store).unwrap();
        let loaded = load_profiles(&dir);

        assert!(loaded.get_profile("Gaming").is_some());
        assert!(loaded.get_profile("Vocal").is_some());
        assert!(loaded.get_profile(DEFAULT_PROFILE_NAME).is_some());
        assert_eq!(
            loaded.profile_for_app("game.exe").name,
            "Gaming"
        );
    }

    #[test]
    fn missing_config_file_returns_default() {
        let dir = test_dir("missing_config");
        // Directory exists but config.json does not.
        fs::create_dir_all(&dir).unwrap();
        let config = load_config(&dir);
        assert!(config.last_device_id.is_none());
    }

    #[test]
    fn missing_profiles_file_returns_fresh_store() {
        let dir = test_dir("missing_profiles");
        fs::create_dir_all(&dir).unwrap();
        let store = load_profiles(&dir);
        assert!(store.get_profile(DEFAULT_PROFILE_NAME).is_some());
        assert_eq!(store.profile_count(), 1);
    }

    #[test]
    fn corrupt_config_returns_default() {
        let dir = test_dir("corrupt_config");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), b"not valid json {{{{").unwrap();
        let config = load_config(&dir);
        assert!(config.last_device_id.is_none());
    }

    #[test]
    fn corrupt_profiles_returns_fresh_store() {
        let dir = test_dir("corrupt_profiles");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("profiles.json"), b"not valid json").unwrap();
        let store = load_profiles(&dir);
        assert!(store.get_profile(DEFAULT_PROFILE_NAME).is_some());
    }
}
