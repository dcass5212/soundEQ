// =============================================================================
// profile.rs — Profile and ProfileStore
//
// A "profile" is a named EQ configuration: a list of up to MAX_BANDS bands
// that will be loaded into a FilterChain when the user (or the system) picks
// that profile.
//
// The ProfileStore owns all profiles and knows which profile each app should
// use. The store is a pure in-memory data structure — serializing it to disk
// and loading it back are handled in the Tauri layer (Step 3).
//
// DESIGN NOTES:
// - The store always contains at least one profile: the "Default" profile.
//   Every app that has no explicit assignment falls back to Default.
// - The default profile can never be removed.
// - App assignments are keyed by process name (e.g. "spotify.exe").
//   The WASAPI session layer (Step 2c) will supply these names at runtime.
// - All HashMap operations are on the control plane — never the audio thread.
//   Heap allocation here is fine.
// =============================================================================

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::filter_chain::{FilterChain, MAX_BANDS};
use crate::filter_type::BandConfig;

// ---------------------------------------------------------------------------
// ProfileError — all the ways a profile operation can fail
// ---------------------------------------------------------------------------

/// Errors returned by ProfileStore mutation methods.
#[derive(Debug, Error, PartialEq)]
pub enum ProfileError {
    /// A lookup was done for a profile name that doesn't exist in the store.
    #[error("profile '{0}' not found")]
    NotFound(String),

    /// Attempted to add a profile whose name is already taken.
    #[error("profile '{0}' already exists")]
    AlreadyExists(String),

    /// Attempted to remove or rename the currently active default profile.
    /// The store must always have a fallback — this keeps that invariant.
    #[error("cannot remove the default profile")]
    CannotRemoveDefault,

    /// A profile was constructed with more bands than MAX_BANDS allows.
    /// This prevents unbounded heap growth and matches the DSP engine's limit.
    #[error("profile has {count} bands but the maximum is {MAX_BANDS}")]
    TooManyBands { count: usize },

    /// One of the bands inside a profile failed BandConfig::validate().
    #[error("band {index} is invalid: {reason}")]
    InvalidBand { index: usize, reason: String },

    /// Attempted to set as default a profile that doesn't exist in the store.
    #[error("cannot set default to '{0}': profile not found")]
    DefaultNotFound(String),
}

// ---------------------------------------------------------------------------
// Profile — a named EQ configuration
//
// This is the unit of storage. One profile corresponds to one EQ curve that
// the user can name, save, load, and assign to apps.
// ---------------------------------------------------------------------------

/// A named collection of EQ bands. Can hold 0..=MAX_BANDS bands.
/// Zero bands means passthrough (flat, no processing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Human-readable name shown in the UI and used as the store key.
    /// Must be unique within a ProfileStore.
    pub name: String,

    /// The EQ bands that define this profile's sound.
    /// Order matters — bands are applied in sequence by FilterChain.
    pub bands: Vec<BandConfig>,
}

impl Profile {
    /// Creates a flat (zero-band) profile with the given name.
    /// A flat profile is a perfect passthrough — it does not alter the signal.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bands: Vec::new(),
        }
    }

    /// Creates a profile pre-filled with the given bands.
    pub fn with_bands(name: impl Into<String>, bands: Vec<BandConfig>) -> Self {
        Self {
            name: name.into(),
            bands,
        }
    }

    /// Validates that the profile is safe to load into the DSP engine.
    ///
    /// Checks:
    /// - Band count does not exceed MAX_BANDS
    /// - Every band passes BandConfig::validate() (frequency, gain, Q ranges)
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.bands.len() > MAX_BANDS {
            return Err(ProfileError::TooManyBands { count: self.bands.len() });
        }
        for (i, band) in self.bands.iter().enumerate() {
            band.validate().map_err(|reason| ProfileError::InvalidBand {
                index: i,
                reason,
            })?;
        }
        Ok(())
    }

    /// Builds a FilterChain ready for audio processing from this profile's bands.
    ///
    /// `sample_rate` is in Hz (e.g. 48_000.0). The biquad coefficient formulas
    /// are all sample-rate-dependent, so it must be supplied at chain creation
    /// time and again whenever the audio device changes.
    ///
    /// The caller is responsible for replacing the active FilterChain atomically
    /// so the audio thread never sees a half-built state (done in Step 2).
    pub fn to_filter_chain(&self, sample_rate: f64) -> FilterChain {
        let mut chain = FilterChain::new(sample_rate);
        chain.set_bands(&self.bands);
        chain
    }
}

// ---------------------------------------------------------------------------
// ProfileStore — the top-level manager
//
// Owns all profiles and the per-app routing table. Serializable so the Tauri
// layer can persist it to %APPDATA%\WindowsEQ\profiles.json (Step 3).
// ---------------------------------------------------------------------------

/// The name used for the built-in fallback profile that is always present.
pub const DEFAULT_PROFILE_NAME: &str = "Default";

/// Manages all user profiles and maps app process names to profiles.
///
/// Invariants upheld by all mutating methods:
/// - At least one profile always exists (the default profile).
/// - `default_profile_name` always refers to an existing profile.
/// - App assignments always point to existing profile names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileStore {
    /// All profiles, keyed by profile name for O(1) lookup.
    profiles: HashMap<String, Profile>,

    /// Maps process executable name → profile name.
    /// Example: "spotify.exe" → "Bass Boost"
    /// Apps not present here fall back to the default profile.
    app_assignments: HashMap<String, String>,

    /// The profile used for any app that has no explicit assignment.
    /// Always points to a name present in `profiles`.
    default_profile_name: String,
}

impl ProfileStore {
    /// Creates a new store with a single flat "Default" profile.
    pub fn new() -> Self {
        let default = Profile::new(DEFAULT_PROFILE_NAME);
        let mut profiles = HashMap::new();
        profiles.insert(default.name.clone(), default);

        Self {
            profiles,
            app_assignments: HashMap::new(),
            default_profile_name: DEFAULT_PROFILE_NAME.to_string(),
        }
    }

    // -------------------------------------------------------------------------
    // Profile CRUD
    // -------------------------------------------------------------------------

    /// Adds a new profile to the store.
    ///
    /// Returns `Err(AlreadyExists)` if a profile with that name is already
    /// present. Profile names are case-sensitive.
    pub fn add_profile(&mut self, profile: Profile) -> Result<(), ProfileError> {
        if self.profiles.contains_key(&profile.name) {
            return Err(ProfileError::AlreadyExists(profile.name.clone()));
        }
        profile.validate()?;
        self.profiles.insert(profile.name.clone(), profile);
        Ok(())
    }

    /// Removes a profile from the store and returns it.
    ///
    /// Any apps that were assigned to this profile are automatically
    /// unassigned (they will fall back to the default profile).
    ///
    /// Returns `Err(CannotRemoveDefault)` if you try to remove the profile
    /// that is currently set as the default.
    pub fn remove_profile(&mut self, name: &str) -> Result<Profile, ProfileError> {
        if name == self.default_profile_name {
            return Err(ProfileError::CannotRemoveDefault);
        }
        let profile = self.profiles.remove(name)
            .ok_or_else(|| ProfileError::NotFound(name.to_string()))?;

        // Remove all app assignments that pointed to this profile.
        // Those apps will now silently fall back to the default profile,
        // which is the safest behaviour — the user still gets audio.
        self.app_assignments.retain(|_, assigned| assigned != name);

        Ok(profile)
    }

    /// Returns a reference to the named profile, or `None` if not found.
    pub fn get_profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    /// Returns a mutable reference to the named profile, or `None` if not found.
    pub fn get_profile_mut(&mut self, name: &str) -> Option<&mut Profile> {
        self.profiles.get_mut(name)
    }

    /// Returns the names of all profiles in the store (order is unspecified).
    pub fn profile_names(&self) -> Vec<&str> {
        self.profiles.keys().map(String::as_str).collect()
    }

    /// Returns the total number of profiles in the store.
    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    // -------------------------------------------------------------------------
    // Default profile management
    // -------------------------------------------------------------------------

    /// Returns the name of the current default profile.
    pub fn default_profile_name(&self) -> &str {
        &self.default_profile_name
    }

    /// Changes which profile is used as the fallback for unassigned apps.
    ///
    /// Returns `Err(DefaultNotFound)` if the named profile isn't in the store.
    pub fn set_default_profile(&mut self, name: &str) -> Result<(), ProfileError> {
        if !self.profiles.contains_key(name) {
            return Err(ProfileError::DefaultNotFound(name.to_string()));
        }
        self.default_profile_name = name.to_string();
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Per-app routing
    // -------------------------------------------------------------------------

    /// Returns the profile that should be applied to `app_exe`.
    ///
    /// Resolution order:
    /// 1. If `app_exe` has an explicit assignment → return that profile.
    /// 2. Otherwise → return the default profile.
    ///
    /// This always returns a valid profile because the store invariants
    /// guarantee the default profile is always present.
    pub fn profile_for_app(&self, app_exe: &str) -> &Profile {
        // Try the explicit assignment first
        if let Some(profile_name) = self.app_assignments.get(app_exe) {
            if let Some(profile) = self.profiles.get(profile_name) {
                return profile;
            }
            // Assignment pointed to a deleted profile — fall through to default.
            // (Should not happen under normal operation; remove_profile cleans these up.)
        }
        // Unwrap is safe: the default profile is guaranteed to exist.
        self.profiles.get(&self.default_profile_name).unwrap()
    }

    /// Assigns `app_exe` to use `profile_name` instead of the default.
    ///
    /// `app_exe` is the process executable name as reported by WASAPI,
    /// e.g. `"spotify.exe"` or `"firefox.exe"`.
    ///
    /// Returns `Err(NotFound)` if `profile_name` isn't in the store.
    pub fn assign_app(&mut self, app_exe: &str, profile_name: &str) -> Result<(), ProfileError> {
        if !self.profiles.contains_key(profile_name) {
            return Err(ProfileError::NotFound(profile_name.to_string()));
        }
        self.app_assignments.insert(app_exe.to_string(), profile_name.to_string());
        Ok(())
    }

    /// Removes the explicit profile assignment for `app_exe`.
    /// After this call the app reverts to using the default profile.
    /// No-op if the app had no assignment.
    pub fn unassign_app(&mut self, app_exe: &str) {
        self.app_assignments.remove(app_exe);
    }

    /// Returns a reference to the full app-assignment map.
    /// Useful for serialization and for the UI's "per-app" settings screen.
    pub fn app_assignments(&self) -> &HashMap<String, String> {
        &self.app_assignments
    }
}

impl Default for ProfileStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter_type::{BandConfig, FilterType};

    fn peak(freq: f64, gain_db: f64) -> BandConfig {
        let mut b = BandConfig::new(FilterType::Peak, freq);
        b.gain_db = gain_db;
        b
    }

    // --- Profile ---

    #[test]
    fn flat_profile_passes_validation() {
        let p = Profile::new("Test");
        assert!(p.validate().is_ok());
    }

    #[test]
    fn profile_with_valid_bands_passes_validation() {
        let p = Profile::with_bands("Test", vec![peak(1000.0, 6.0)]);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn profile_with_too_many_bands_fails_validation() {
        // MAX_BANDS + 1 bands — should fail
        let bands = (0..=MAX_BANDS)
            .map(|i| BandConfig::new(FilterType::Peak, 200.0 + i as f64 * 100.0))
            .collect();
        let p = Profile::with_bands("Overloaded", bands);
        assert!(matches!(p.validate(), Err(ProfileError::TooManyBands { .. })));
    }

    #[test]
    fn profile_with_invalid_band_fails_validation() {
        let mut bad_band = BandConfig::new(FilterType::Peak, 1000.0);
        bad_band.frequency = 5.0; // below 20 Hz — out of valid range
        let p = Profile::with_bands("Bad", vec![bad_band]);
        assert!(matches!(p.validate(), Err(ProfileError::InvalidBand { index: 0, .. })));
    }

    #[test]
    fn to_filter_chain_produces_correct_response() {
        // A profile with a +6 dB peak at 1 kHz should produce ~6 dB at 1 kHz
        let p = Profile::with_bands("Test", vec![peak(1000.0, 6.0)]);
        let chain = p.to_filter_chain(48_000.0);
        let db = chain.magnitude_db_at(1000.0);
        assert!((db - 6.0).abs() < 0.2, "expected ~6 dB, got {:.2}", db);
    }

    // --- ProfileStore: basic CRUD ---

    #[test]
    fn new_store_has_default_profile() {
        let store = ProfileStore::new();
        assert_eq!(store.profile_count(), 1);
        assert!(store.get_profile(DEFAULT_PROFILE_NAME).is_some());
        assert_eq!(store.default_profile_name(), DEFAULT_PROFILE_NAME);
    }

    #[test]
    fn add_and_retrieve_profile() {
        let mut store = ProfileStore::new();
        let p = Profile::with_bands("Gaming", vec![peak(3000.0, 3.0)]);
        store.add_profile(p.clone()).unwrap();
        let retrieved = store.get_profile("Gaming").unwrap();
        assert_eq!(retrieved.name, "Gaming");
        assert_eq!(retrieved.bands.len(), 1);
    }

    #[test]
    fn add_duplicate_name_returns_error() {
        let mut store = ProfileStore::new();
        let p = Profile::new("Gaming");
        store.add_profile(p.clone()).unwrap();
        let result = store.add_profile(p);
        assert!(matches!(result, Err(ProfileError::AlreadyExists(_))));
    }

    #[test]
    fn remove_profile_returns_it() {
        let mut store = ProfileStore::new();
        store.add_profile(Profile::new("Gaming")).unwrap();
        let removed = store.remove_profile("Gaming").unwrap();
        assert_eq!(removed.name, "Gaming");
        assert!(store.get_profile("Gaming").is_none());
    }

    #[test]
    fn cannot_remove_default_profile() {
        let mut store = ProfileStore::new();
        let result = store.remove_profile(DEFAULT_PROFILE_NAME);
        assert!(matches!(result, Err(ProfileError::CannotRemoveDefault)));
    }

    #[test]
    fn remove_nonexistent_profile_returns_error() {
        let mut store = ProfileStore::new();
        let result = store.remove_profile("Ghost");
        assert!(matches!(result, Err(ProfileError::NotFound(_))));
    }

    #[test]
    fn remove_profile_unassigns_apps() {
        let mut store = ProfileStore::new();
        store.add_profile(Profile::new("Gaming")).unwrap();
        store.assign_app("game.exe", "Gaming").unwrap();

        store.remove_profile("Gaming").unwrap();

        // The app assignment should be gone; the app now falls back to default
        let profile = store.profile_for_app("game.exe");
        assert_eq!(profile.name, DEFAULT_PROFILE_NAME);
    }

    // --- ProfileStore: default profile ---

    #[test]
    fn set_default_profile_changes_fallback() {
        let mut store = ProfileStore::new();
        store.add_profile(Profile::new("Gaming")).unwrap();
        store.set_default_profile("Gaming").unwrap();
        assert_eq!(store.default_profile_name(), "Gaming");
        // An unassigned app should now get Gaming
        let profile = store.profile_for_app("unknown.exe");
        assert_eq!(profile.name, "Gaming");
    }

    #[test]
    fn set_default_to_missing_profile_returns_error() {
        let mut store = ProfileStore::new();
        let result = store.set_default_profile("Nonexistent");
        assert!(matches!(result, Err(ProfileError::DefaultNotFound(_))));
    }

    // --- ProfileStore: per-app routing ---

    #[test]
    fn unassigned_app_gets_default_profile() {
        let store = ProfileStore::new();
        let profile = store.profile_for_app("firefox.exe");
        assert_eq!(profile.name, DEFAULT_PROFILE_NAME);
    }

    #[test]
    fn assigned_app_gets_its_profile() {
        let mut store = ProfileStore::new();
        store.add_profile(Profile::new("Gaming")).unwrap();
        store.assign_app("game.exe", "Gaming").unwrap();
        let profile = store.profile_for_app("game.exe");
        assert_eq!(profile.name, "Gaming");
    }

    #[test]
    fn assign_to_missing_profile_returns_error() {
        let mut store = ProfileStore::new();
        let result = store.assign_app("game.exe", "Nonexistent");
        assert!(matches!(result, Err(ProfileError::NotFound(_))));
    }

    #[test]
    fn unassign_app_reverts_to_default() {
        let mut store = ProfileStore::new();
        store.add_profile(Profile::new("Gaming")).unwrap();
        store.assign_app("game.exe", "Gaming").unwrap();
        store.unassign_app("game.exe");
        let profile = store.profile_for_app("game.exe");
        assert_eq!(profile.name, DEFAULT_PROFILE_NAME);
    }

    // --- Serialization ---

    #[test]
    fn store_serde_roundtrip() {
        let mut store = ProfileStore::new();
        store.add_profile(Profile::with_bands(
            "Gaming",
            vec![peak(3000.0, 3.0)],
        )).unwrap();
        store.assign_app("game.exe", "Gaming").unwrap();

        let json = serde_json::to_string(&store).unwrap();
        let back: ProfileStore = serde_json::from_str(&json).unwrap();

        assert_eq!(back.profile_count(), 2);
        assert_eq!(back.profile_for_app("game.exe").name, "Gaming");
        assert_eq!(back.profile_for_app("unknown.exe").name, DEFAULT_PROFILE_NAME);
    }
}
