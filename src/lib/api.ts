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
}

export interface Profile {
  name: string;
  bands: BandConfig[];
}

export interface ProfileStore {
  profiles: Record<string, Profile>;
  app_assignments: Record<string, string>;
  default_profile_name: string;
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

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

/** Returns the persisted app config (e.g. last selected device). */
export const getConfig = (): Promise<AppConfig> =>
  invoke("get_config");

/** Saves both device selections so they are restored on next launch. */
export const setDeviceConfig = (
  deviceId: string | null,
  captureDeviceId: string | null,
): Promise<void> => invoke("set_device_config", { deviceId, captureDeviceId });

// ---------------------------------------------------------------------------
// Startup commands
// ---------------------------------------------------------------------------

/** Returns true if soundEQ is registered in HKCU\...\Run. */
export const isStartupEnabled = (): Promise<boolean> =>
  invoke("is_startup_enabled");

/** Writes or removes the Windows startup registry entry. */
export const setStartupEnabled = (enabled: boolean): Promise<void> =>
  invoke("set_startup_enabled", { enabled });
