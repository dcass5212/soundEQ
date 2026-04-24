import { useCallback, useEffect, useRef, useState } from "react";
import { EqCanvas } from "./components/EqCanvas";
import { BandRow } from "./components/BandRow";
import { ProfilePanel } from "./components/ProfilePanel";
import { SessionPanel } from "./components/SessionPanel";
import { SetupBanner } from "./components/SetupBanner";
import {
  type AppConfig,
  type BandConfig,
  type DeviceInfo,
  type Profile,
  type ProfileStore,
  type SessionInfo,
  assignApp,
  createProfile,
  deleteProfile,
  getBuiltinPresets,
  getConfig,
  getFrequencyResponse,
  getProfiles,
  isEngineRunning,
  isStartupEnabled,
  listAudioSessions,
  listRenderDevices,
  setActiveProfile,
  setDeviceConfig,
  setStartupEnabled,
  startEngine,
  stopEngine,
  unassignApp,
  updateProfileBands,
} from "./lib/api";

const DEFAULT_BAND: BandConfig = {
  filter_type: "peak",
  frequency: 1_000,
  gain_db: 0,
  q: 1.0,
  enabled: true,
};

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

export default function App() {
  const [store, setStore]                   = useState<ProfileStore | null>(null);
  const [activeName, setActiveName]         = useState("Default");
  const [running, setRunning]               = useState(false);
  const [devices, setDevices]               = useState<DeviceInfo[]>([]);
  const [deviceId, setDeviceId]             = useState<string | null>(null);
  const [captureDeviceId, setCaptureDeviceId] = useState<string | null>(null);
  const [showSetupBanner, setShowSetupBanner] = useState(false);
  const [sessions, setSessions]             = useState<SessionInfo[]>([]);
  const [curve, setCurve]                   = useState<[number, number][]>([]);
  const [presets, setPresets]               = useState<Profile[]>([]);
  const [startupOn, setStartupOn]           = useState(false);
  const [error, setError]                   = useState<string | null>(null);

  const errorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  function showError(msg: string) {
    setError(msg);
    if (errorTimer.current) clearTimeout(errorTimer.current);
    errorTimer.current = setTimeout(() => setError(null), 4_000);
  }

  // -----------------------------------------------------------------------
  // Data loading helpers
  // -----------------------------------------------------------------------

  const refreshStore = useCallback(async () => {
    try {
      const s = await getProfiles();
      setStore(s);
      return s;
    } catch (e) {
      showError(String(e));
      return null;
    }
  }, []);

  const refreshCurve = useCallback(async (name: string) => {
    try {
      const pts = await getFrequencyResponse(name);
      setCurve(pts);
    } catch {
      setCurve([]);
    }
  }, []);

  const refreshSessions = useCallback(async () => {
    try {
      const s = await listAudioSessions();
      setSessions(s);
    } catch {
      // Non-fatal: sessions panel shows empty state.
    }
  }, []);

  // -----------------------------------------------------------------------
  // Initial load
  // -----------------------------------------------------------------------

  useEffect(() => {
    (async () => {
      const [s, devs, running, savedConfig] = await Promise.all([
        getProfiles().catch(() => null),
        listRenderDevices().catch((err) => { console.error("list_render_devices:", err); return [] as DeviceInfo[]; }),
        isEngineRunning().catch(() => false),
        getConfig().catch((): AppConfig => ({ last_device_id: null, capture_device_id: null })),
        getBuiltinPresets().then(setPresets).catch(() => {}),
        isStartupEnabled().then(setStartupOn).catch(() => {}),
      ]);
      if (s) setStore(s);
      setDevices(devs);

      // Output device: prefer the user's saved choice, fall back to system default.
      const savedDev = devs.find((d) => d.id === savedConfig.last_device_id);
      const defDev   = devs.find((d) => d.is_default);
      const chosen   = savedDev ?? defDev;
      if (chosen) setDeviceId(chosen.id);

      // Capture device: prefer saved, then auto-detect VB-Cable by name.
      // VB-Cable's render endpoint is named "CABLE Input (VB-Audio Virtual Cable)".
      const cableDev    = devs.find((d) => d.name.toLowerCase().includes("cable"));
      const savedCapture = devs.find((d) => d.id === savedConfig.capture_device_id);
      const chosenCapture = savedCapture ?? cableDev ?? null;
      if (chosenCapture) setCaptureDeviceId(chosenCapture.id);

      // Show the setup banner if no cable device is present at all.
      setShowSetupBanner(!cableDev);

      setRunning(running);
    })();
  }, []);

  // Refresh curve whenever the active profile or store changes.
  useEffect(() => {
    if (!store) return;
    const profile = store.profiles[activeName];
    if (!profile || profile.bands.length === 0) {
      setCurve([]);
      return;
    }
    refreshCurve(activeName);
  }, [store, activeName, refreshCurve]);

  // Poll sessions every 3 s.
  useEffect(() => {
    refreshSessions();
    const id = setInterval(refreshSessions, 3_000);
    return () => clearInterval(id);
  }, [refreshSessions]);

  // -----------------------------------------------------------------------
  // Derived state
  // -----------------------------------------------------------------------

  const activeProfile = store?.profiles[activeName] ?? null;
  const bands = activeProfile?.bands ?? [];

  // -----------------------------------------------------------------------
  // Engine handlers
  // -----------------------------------------------------------------------

  async function handleToggleEngine() {
    try {
      if (running) {
        await stopEngine();
        setRunning(false);
      } else {
        await startEngine(captureDeviceId ?? undefined, deviceId ?? undefined);
        setRunning(true);
        // Apply the currently-selected profile immediately.
        await setActiveProfile(activeName).catch(() => {});
      }
    } catch (e) {
      showError(String(e));
    }
  }

  // -----------------------------------------------------------------------
  // Profile handlers
  // -----------------------------------------------------------------------

  async function handleSelectProfile(name: string) {
    setActiveName(name);
    if (running) {
      await setActiveProfile(name).catch((e) => showError(String(e)));
    }
  }

  async function handleNewProfile() {
    const name = window.prompt("Profile name:");
    if (!name?.trim()) return;
    try {
      await createProfile(name.trim());
      const s = await refreshStore();
      if (s) setActiveName(name.trim());
    } catch (e) {
      showError(String(e));
    }
  }

  async function handleDeleteProfile(name: string) {
    if (!window.confirm(`Delete profile "${name}"?`)) return;
    try {
      await deleteProfile(name);
      const s = await refreshStore();
      if (s && activeName === name) {
        setActiveName(s.default_profile_name);
      }
    } catch (e) {
      showError(String(e));
    }
  }

  async function handleLoadPreset(preset: Profile) {
    if (bands.length > 0) {
      if (!window.confirm(`Load preset "${preset.name}" into "${activeName}"?`)) return;
    }
    await handleBandsChange(preset.bands);
  }

  // -----------------------------------------------------------------------
  // Band handlers
  // -----------------------------------------------------------------------

  async function handleBandsChange(newBands: BandConfig[]) {
    try {
      await updateProfileBands(activeName, newBands);
      await refreshStore();
      if (running) await setActiveProfile(activeName).catch(() => {});
    } catch (e) {
      showError(String(e));
    }
  }

  function handleBandChange(index: number, updated: BandConfig) {
    const next = bands.map((b, i) => (i === index ? updated : b));
    handleBandsChange(next);
  }

  function handleBandDelete(index: number) {
    handleBandsChange(bands.filter((_, i) => i !== index));
  }

  function handleAddBand() {
    handleBandsChange([...bands, { ...DEFAULT_BAND }]);
  }

  // -----------------------------------------------------------------------
  // Session handlers
  // -----------------------------------------------------------------------

  async function handleAssign(processName: string, profileName: string) {
    try {
      await assignApp(processName, profileName);
      await refreshStore();
    } catch (e) {
      showError(String(e));
    }
  }

  async function handleToggleStartup() {
    const next = !startupOn;
    try {
      await setStartupEnabled(next);
      setStartupOn(next);
    } catch (e) {
      showError(String(e));
    }
  }

  async function handleUnassign(processName: string) {
    try {
      await unassignApp(processName);
      await refreshStore();
    } catch (e) {
      showError(String(e));
    }
  }

  // -----------------------------------------------------------------------
  // Render
  // -----------------------------------------------------------------------

  return (
    <div className="h-screen flex flex-col bg-gray-950 text-gray-100 overflow-hidden select-none">

      {/* ------------------------------------------------------------------ */}
      {/* Top bar */}
      {/* ------------------------------------------------------------------ */}
      <header className="flex-shrink-0 h-12 flex items-center gap-3 px-4 bg-gray-900 border-b border-gray-800">
        <span className="text-sm font-bold tracking-widest text-indigo-400 mr-2">
          soundEQ
        </span>

        {/* Capture source selector */}
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] text-gray-500 uppercase tracking-wider shrink-0">Capture</span>
          <select
            value={captureDeviceId ?? ""}
            onChange={(e) => {
              const id = e.target.value || null;
              setCaptureDeviceId(id);
              setDeviceConfig(deviceId, id).catch(() => {});
            }}
            disabled={running}
            title="Audio source to capture and EQ (set this to your virtual cable device)"
            className="max-w-[190px] bg-gray-800 text-gray-200 text-xs rounded px-2 py-1 border border-gray-700 disabled:opacity-50"
          >
            {devices.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
            {devices.length === 0 && <option value="">No devices found</option>}
          </select>
        </div>

        <span className="text-gray-600 text-xs shrink-0">→</span>

        {/* Output device selector */}
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] text-gray-500 uppercase tracking-wider shrink-0">Output</span>
          <select
            value={deviceId ?? ""}
            onChange={(e) => {
              const id = e.target.value || null;
              setDeviceId(id);
              setDeviceConfig(id, captureDeviceId).catch(() => {});
            }}
            disabled={running}
            title="Audio output device (your real speakers or headphones)"
            className="max-w-[190px] bg-gray-800 text-gray-200 text-xs rounded px-2 py-1 border border-gray-700 disabled:opacity-50"
          >
            {devices.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
            {devices.length === 0 && <option value="">No devices found</option>}
          </select>
        </div>

        <div className="flex-1" />

        {/* Startup toggle */}
        <button
          onClick={handleToggleStartup}
          title={startupOn ? "Disable launch at Windows startup" : "Enable launch at Windows startup"}
          className={`flex items-center gap-1.5 px-2.5 py-1 rounded text-xs transition-colors ${
            startupOn
              ? "bg-indigo-900/60 text-indigo-300 border border-indigo-700"
              : "text-gray-600 hover:text-gray-400 border border-gray-800 hover:border-gray-600"
          }`}
        >
          <span className="text-[10px]">⚡</span>
          Startup
        </button>

        {/* Running indicator */}
        {running && (
          <span className="flex items-center gap-1.5 text-xs text-emerald-400">
            <span className="inline-block w-2 h-2 rounded-full bg-emerald-400 animate-pulse" />
            Running
          </span>
        )}

        {/* Start / Stop button */}
        <button
          onClick={handleToggleEngine}
          className={`px-4 py-1.5 rounded text-xs font-medium transition-colors ${
            running
              ? "bg-red-700 hover:bg-red-600 text-white"
              : "bg-indigo-600 hover:bg-indigo-500 text-white"
          }`}
        >
          {running ? "Stop" : "Start"}
        </button>
      </header>

      {/* ------------------------------------------------------------------ */}
      {/* VB-Cable setup guidance (shown only when no cable device found) */}
      {/* ------------------------------------------------------------------ */}
      {showSetupBanner && (
        <SetupBanner onDismiss={() => setShowSetupBanner(false)} />
      )}

      {/* ------------------------------------------------------------------ */}
      {/* Body */}
      {/* ------------------------------------------------------------------ */}
      <div className="flex flex-1 min-h-0">

        {/* Profile / preset sidebar */}
        <ProfilePanel
          store={store}
          presets={presets}
          activeName={activeName}
          onSelect={handleSelectProfile}
          onNew={handleNewProfile}
          onDelete={handleDeleteProfile}
          onLoadPreset={handleLoadPreset}
        />

        {/* Main content */}
        <div className="flex flex-col flex-1 min-h-0">

          {/* EQ curve */}
          <div className="flex-shrink-0 bg-gray-950 border-b border-gray-800">
            <EqCanvas frequencyResponse={curve} bands={bands} />
          </div>

          {/* Band list */}
          <div className="flex-1 min-h-0 overflow-y-auto p-3">
            {bands.length === 0 ? (
              <p className="text-xs text-gray-600 py-2 px-1">
                No bands — add one below or load a preset.
              </p>
            ) : (
              <div className="space-y-1">
                {bands.map((band, i) => (
                  <BandRow
                    key={i}
                    band={band}
                    index={i}
                    onChange={handleBandChange}
                    onDelete={handleBandDelete}
                  />
                ))}
              </div>
            )}

            {bands.length < 16 && (
              <button
                onClick={handleAddBand}
                className="mt-2 text-xs text-gray-500 hover:text-indigo-400 border border-dashed border-gray-700 hover:border-indigo-600 rounded px-3 py-1 transition-colors"
              >
                + Add band
              </button>
            )}
          </div>

          {/* Session bar */}
          <SessionPanel
            sessions={sessions}
            store={store}
            onAssign={handleAssign}
            onUnassign={handleUnassign}
          />
        </div>
      </div>

      {/* ------------------------------------------------------------------ */}
      {/* Error toast */}
      {/* ------------------------------------------------------------------ */}
      {error && (
        <div className="fixed bottom-4 left-1/2 -translate-x-1/2 bg-red-900 text-red-200 text-xs px-4 py-2 rounded shadow-lg z-50">
          {error}
        </div>
      )}
    </div>
  );
}
