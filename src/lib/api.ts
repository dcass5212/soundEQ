// =============================================================================
// api.ts — Typed wrappers around Tauri IPC commands
//
// Every function here is a thin invoke() call with full TypeScript types.
// Import from this module instead of calling invoke() directly so the
// command names and argument shapes are defined in one place.
// =============================================================================

import { invoke } from "@tauri-apps/api/core";

// ---------------------------------------------------------------------------
// Shared types (must match the Rust structs in src-tauri/src/lib.rs)
// ---------------------------------------------------------------------------

export interface DeviceInfo {
  id: string;
  name: string;
  is_default: boolean;
}

export interface SessionInfo {
  pid: number;
  process_name: string;
  session_id: string;
}

export interface StreamInfo {
  sample_rate: number;
  channels: number;
}

export type FilterType =
  | "peak"
  | "low_shelf"
  | "high_shelf"
  | "low_pass"
  | "high_pass"
  | "notch"
  | "bandpass";

export interface BandConfig {
  filter_type: FilterType;
  frequency: number;
  gain_db: number;
  q: number;
  enabled: boolean;
  /** Hex color string (e.g. "#6366f1") for the canvas dot and curve gradient. Cosmetic only. */
  color?: string;
}

export type CrossfeedLevel = "Mild" | "Moderate" | "Strong";

export interface CrossfeedConfig {
  enabled: boolean;
  level: CrossfeedLevel;
}

export interface Profile {
  name: string;
  bands: BandConfig[];
  /** Headphone crossfeed settings. Absent on old saves — treat missing as disabled. */
  crossfeed?: CrossfeedConfig;
}

export interface ProfileStore {
  profiles: Record<string, Profile>;
  app_assignments: Record<string, string>;
  default_profile_name: string;
  /** Apps in this list have auto-switching disabled (assignment is kept). */
  disabled_apps: string[];
  /** Per-app WASAPI session volume overrides (linear 0.0–1.0; 1.0 = full volume). */
  app_volumes: Record<string, number>;
}

export interface AppConfig {
  last_device_id: string | null;
  capture_device_id: string | null;
}

// ---------------------------------------------------------------------------
// Device commands
// ---------------------------------------------------------------------------

export const listRenderDevices = (): Promise<DeviceInfo[]> =>
  invoke("list_render_devices");

export const listAudioSessions = (): Promise<SessionInfo[]> =>
  invoke("list_audio_sessions");

// ---------------------------------------------------------------------------
// Profile commands
// ---------------------------------------------------------------------------

export const getProfiles = (): Promise<ProfileStore> =>
  invoke("get_profiles");

export const getBuiltinPresets = (): Promise<Profile[]> =>
  invoke("get_builtin_presets");

export const createProfile = (name: string): Promise<void> =>
  invoke("create_profile", { name });

export const deleteProfile = (name: string): Promise<void> =>
  invoke("delete_profile", { name });

export const renameProfile = (oldName: string, newName: string): Promise<void> =>
  invoke("rename_profile", { oldName, newName });

export const updateProfileBands = (
  name: string,
  bands: BandConfig[]
): Promise<void> => invoke("update_profile_bands", { name, bands });

export const assignApp = (
  processName: string,
  profileName: string
): Promise<void> => invoke("assign_app", { processName, profileName });

export const unassignApp = (processName: string): Promise<void> =>
  invoke("unassign_app", { processName });

export const setAppEnabled = (processName: string, enabled: boolean): Promise<void> =>
  invoke("set_app_enabled", { processName, enabled });

/** Serializes a profile to a pretty-printed JSON string for user export.
 *  The caller is responsible for triggering the file download. */
export const exportProfile = (name: string): Promise<string> =>
  invoke("export_profile", { name });

/** Parses a profile JSON string, resolves name collisions, adds it to the
 *  store, and returns the actual name used (may have a numeric suffix). */
export const importProfile = (json: string): Promise<string> =>
  invoke("import_profile", { json });

/** Updates the crossfeed config for a profile and applies it live if active. */
export const setProfileCrossfeed = (
  profileName: string,
  config: CrossfeedConfig,
): Promise<void> => invoke("set_profile_crossfeed", { profileName, config });

/** Sets the WASAPI session volume for `processName` and persists it.
 *  `volume` is linear [0.0, 1.0]; 1.0 = full session volume, 0.0 = silent. */
export const setAppVolume = (processName: string, volume: number): Promise<void> =>
  invoke("set_app_volume", { processName, volume });

// ---------------------------------------------------------------------------
// Engine commands
// ---------------------------------------------------------------------------

export const startEngine = (captureDeviceId?: string, deviceId?: string): Promise<StreamInfo> =>
  invoke("start_engine", { captureDeviceId: captureDeviceId ?? null, deviceId: deviceId ?? null });

export const stopEngine = (): Promise<void> =>
  invoke("stop_engine");

export const isEngineRunning = (): Promise<boolean> =>
  invoke("is_engine_running");

export const setActiveProfile = (name: string): Promise<void> =>
  invoke("set_active_profile", { name });

// ---------------------------------------------------------------------------
// Visualization commands
// ---------------------------------------------------------------------------

/** 256 (frequency_hz, gain_db) log-spaced pairs for the EQ curve canvas. */
export const getFrequencyResponse = (name: string): Promise<[number, number][]> =>
  invoke("get_frequency_response", { name });

/** 80 dB magnitude values, log-spaced 20 Hz–20 kHz. −100 = silence.
 *  Poll at ~20 Hz while the engine is running to drive the spectrum display. */
export const getSpectrum = (): Promise<number[]> =>
  invoke("get_spectrum");

// ---------------------------------------------------------------------------
// Bypass commands
// ---------------------------------------------------------------------------

/** Immediately applies the profile assigned to `processName` and pins it as
 *  the routing priority, so subsequent routing ticks don't override it. */
export const applyAppProfile = (processName: string): Promise<void> =>
  invoke("apply_app_profile", { processName });

/** Enables or disables the global EQ bypass (audio passes through unmodified). */
export const setEqBypass = (enabled: boolean): Promise<void> =>
  invoke("set_eq_bypass", { enabled });

/** Returns true if the global EQ bypass is currently active. */
export const isEqBypassed = (): Promise<boolean> =>
  invoke("is_eq_bypassed");

// ---------------------------------------------------------------------------
// Live preview commands (solo / mute)
// ---------------------------------------------------------------------------

/** Immediately applies `bands` to the live engine without saving to the store.
 *  Used by solo/mute: the frontend masks silenced bands' enabled flag and calls
 *  this so the audio thread hears only the un-silenced bands in real time.
 *  No-op when the engine is not running. */
export const applyBandsLive = (bands: BandConfig[]): Promise<void> =>
  invoke("apply_bands_live", { bands });

/** Computes the frequency-response curve for the given bands without a profile
 *  lookup. Used when solo/mute is active so the canvas shows the effective curve
 *  rather than the stored profile's curve. */
export const getFrequencyResponseLive = (
  bands: BandConfig[],
): Promise<[number, number][]> =>
  invoke("get_frequency_response_live", { bands });

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

/** Returns the persisted app config (e.g. last selected device). */
export const getConfig = (): Promise<AppConfig> =>
  invoke("get_config");

/** Returns the current output gain (linear, 1.0 = unity). */
export const getOutputGain = (): Promise<number> =>
  invoke("get_output_gain");

/** Sets and persists the output gain. 1.0 = unity, 1.5 ≈ +3.5 dB, 2.0 = +6 dB. */
export const setOutputGain = (gain: number): Promise<void> =>
  invoke("set_output_gain", { gain });

/** Saves both device selections so they are restored on next launch. */
export const setDeviceConfig = (
  deviceId: string | null,
  captureDeviceId: string | null,
): Promise<void> => invoke("set_device_config", { deviceId, captureDeviceId });

// ---------------------------------------------------------------------------
// Startup commands
// ---------------------------------------------------------------------------

/** Opens the VB-Audio Virtual Cable download page in the default system browser. */
export const openVbCableDownload = (): Promise<void> =>
  invoke("open_vb_cable_download");

/** Returns true if this launch was triggered by the Windows startup entry
 *  (i.e. the process was invoked with --minimized). The frontend uses this
 *  to insert a 10-second delay before auto-starting the audio engine. */
export const isStartupLaunch = (): Promise<boolean> =>
  invoke("is_startup_launch");

/** Returns true if soundEQ is registered in HKCU\...\Run. */
export const isStartupEnabled = (): Promise<boolean> =>
  invoke("is_startup_enabled");

/** Writes or removes the Windows startup registry entry. */
export const setStartupEnabled = (enabled: boolean): Promise<void> =>
  invoke("set_startup_enabled", { enabled });
