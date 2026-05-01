import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useBandHistory } from "./lib/useBandHistory";
import { listen } from "@tauri-apps/api/event";
import { EqCanvas } from "./components/EqCanvas";
import { BandRow } from "./components/BandRow";
import { ProfilePanel } from "./components/ProfilePanel";
import { AppPanel } from "./components/AppPanel";
import { SetupBanner } from "./components/SetupBanner";
import { DEFAULT_BAND_COLOR, bandColor } from "./lib/colors";
import {
  type AppConfig,
  type BandConfig,
  type CrossfeedConfig,
  type CrossfeedLevel,
  type DeviceInfo,
  type Profile,
  type ProfileStore,
  type SessionInfo,
  applyAppProfile,
  applyBandsLive,
  assignApp,
  setAppEnabled,
  createProfile,
  deleteProfile,
  getBuiltinPresets,
  getConfig,
  getFrequencyResponse,
  getFrequencyResponseLive,
  getProfiles,
  getSpectrum,
  renameProfile,
  isEngineRunning,
  isStartupEnabled,
  isStartupLaunch,
  listAudioSessions,
  listRenderDevices,
  setActiveProfile,
  setDeviceConfig,
  setEqBypass,
  isEqBypassed,
  setStartupEnabled,
  startEngine,
  stopEngine,
  unassignApp,
  updateProfileBands,
  getOutputGain,
  setOutputGain,
  setAppVolume,
  setProfileCrossfeed,
  exportProfile,
  importProfile,
} from "./lib/api";

const DEFAULT_BAND: BandConfig = {
  filter_type: "peak",
  frequency: 1_000,
  gain_db: 0,
  q: 1.0,
  enabled: true,
};

// ---------------------------------------------------------------------------
// Solo / mute helpers
// ---------------------------------------------------------------------------

// Pure function — computes the bands the audio engine should hear given the
// current mute/solo state. Leaves the stored profile bands untouched.
//
// Solo takes priority over mute: if any band is soloed, every other band is
// silenced regardless of its muted/enabled status.
function computeEffective(
  allBands: BandConfig[],
  muted: Set<number>,
  solo: number | null,
): BandConfig[] {
  if (muted.size === 0 && solo === null) return allBands;
  return allBands.map((b, i) => {
    if (solo !== null) {
      // Solo mode: only the soloed band stays enabled (if it was already on).
      return { ...b, enabled: i === solo ? b.enabled : false };
    }
    // Mute mode: disabled bands stay disabled; additionally muted bands are silenced.
    return { ...b, enabled: b.enabled && !muted.has(i) };
  });
}

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
  const [bypassed, setBypassed]             = useState(false);
  const [focusedApp, setFocusedApp]         = useState<string | null>(null);
  const [spectrumData, setSpectrumData]     = useState<number[]>([]);
  const [error, setError]                   = useState<string | null>(null);
  const [notice, setNotice]                 = useState<string | null>(null);
  const [mutedBands, setMutedBands]         = useState<Set<number>>(new Set());
  const [soloedBand, setSoloedBand]         = useState<number | null>(null);
  const [outputGain, setOutputGainState]    = useState<number>(1.0);
  // Roving tabindex: tracks which band row is in the Tab order (tabIndex=0).
  // Only one row is reachable via Tab at a time; arrow keys move the rover.
  const [rovingBandIdx, setRovingBandIdx]   = useState(0);

  const gainHoldTimer    = useRef<ReturnType<typeof setTimeout> | null>(null);
  const gainHoldInterval = useRef<ReturnType<typeof setInterval> | null>(null);

  const errorTimer  = useRef<ReturnType<typeof setTimeout> | null>(null);
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Band drag-to-reorder — uses mouse events instead of HTML5 DnD because
  // the HTML5 DnD API is unreliable inside Tauri's WebView.
  const rowRefs       = useRef<(HTMLDivElement | null)[]>([]);
  const [draggingIndex, setDraggingIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);

  // After an async reorder or delete the row list re-renders before we can
  // call .focus() — store the target index here and a useEffect picks it up.
  const pendingFocusRef = useRef<number | null>(null);

  function showError(msg: string) {
    setError(msg);
    if (errorTimer.current) clearTimeout(errorTimer.current);
    errorTimer.current = setTimeout(() => setError(null), 4_000);
  }

  function showNotice(msg: string) {
    setNotice(msg);
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setNotice(null), 3_000);
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
        getOutputGain().then(setOutputGainState).catch(() => {}),
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

      // Auto-start the engine on launch if devices are ready.
      // Skip when already running (single-instance re-focus) or when the
      // cable device isn't installed (setup banner guides the user instead).
      if (!running && chosenCapture && chosen) {
        try {
          // When launched at Windows startup (--minimized flag), wait 10 seconds
          // before opening WASAPI. The audio subsystem can take several seconds
          // to finish initializing at login, and an early open attempt silently
          // fails on some systems. The app is fully usable during this wait —
          // only the engine start is deferred.
          const startupLaunch = await isStartupLaunch().catch(() => false);
          if (startupLaunch) {
            await new Promise<void>((resolve) => setTimeout(resolve, 10_000));
          }
          await startEngine(chosenCapture.id, chosen.id);
          setRunning(true);
          if (s) await setActiveProfile(s.default_profile_name).catch(() => {});
        } catch (e) {
          showError(String(e));
        }
      }
    })();
  }, []);

  // Refresh curve whenever the active profile, store, or solo/mute state changes.
  // When solo/mute is active we ask the engine for the effective curve directly
  // rather than using the stored profile's curve, so the canvas always shows
  // what the user is actually hearing.
  useEffect(() => {
    if (!store) return;
    const profile = store.profiles[activeName];
    if (!profile || profile.bands.length === 0) {
      setCurve([]);
      return;
    }
    const anySoloOrMute = soloedBand !== null || mutedBands.size > 0;
    if (anySoloOrMute) {
      const effective = computeEffective(profile.bands, mutedBands, soloedBand);
      getFrequencyResponseLive(effective).then(setCurve).catch(() => setCurve([]));
    } else {
      refreshCurve(activeName);
    }
  }, [store, activeName, refreshCurve, mutedBands, soloedBand]);

  // Poll sessions every 3 s (used only for add-app suggestions in AppPanel).
  useEffect(() => {
    refreshSessions();
    const id = setInterval(refreshSessions, 3_000);
    return () => clearInterval(id);
  }, [refreshSessions]);

  // Poll spectrum data at ~20 Hz while the engine is running.
  // Clears immediately when the engine stops so the canvas shows silence.
  useEffect(() => {
    if (!running) {
      setSpectrumData([]);
      return;
    }
    const id = setInterval(async () => {
      const data = await getSpectrum().catch((): number[] => []);
      setSpectrumData(data);
    }, 50);
    return () => clearInterval(id);
  }, [running]);

  // Listen for device change events from the Rust device_watch_tick thread.
  // "device-restarted": the engine auto-restarted after a WASAPI thread crash.
  //   → Refresh the device list so dropdowns reflect the new active device.
  // "device-error": auto-restart failed (device gone) — engine is now stopped.
  //   → Clear running state and show an error so the user can act.
  useEffect(() => {
    let unlistenBypass:    (() => void) | undefined;
    let unlistenRestarted: (() => void) | undefined;
    let unlistenError:     (() => void) | undefined;

    listen<boolean>("bypass-changed", (evt) => {
      setBypassed(evt.payload);
    }).then((fn) => { unlistenBypass = fn; });

    // Poll bypass state every 250 ms. The global shortcut (Ctrl+Shift+B) is
    // registered with RegisterHotKey which consumes the key before it reaches
    // the webview, so keydown listeners and Tauri events both fail to fire
    // reliably from that OS callback thread. Polling is the simplest fix.
    const bypassPoll = setInterval(() => {
      isEqBypassed().then(setBypassed).catch(() => {});
    }, 250);

    listen<void>("device-restarted", async () => {
      showNotice("Audio device restarted automatically.");
      try {
        const devs = await listRenderDevices();
        setDevices(devs);
      } catch { /* non-fatal */ }
    }).then((fn) => { unlistenRestarted = fn; });

    listen<string>("device-error", (_evt) => {
      setRunning(false);
      setBypassed(false);
      setSpectrumData([]);
      showError("Audio device disconnected. Click Start to reconnect.");
    }).then((fn) => { unlistenError = fn; });

    return () => {
      unlistenBypass?.();
      unlistenRestarted?.();
      unlistenError?.();
      clearInterval(bypassPoll);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Ctrl+Z → undo, Ctrl+Y / Ctrl+Shift+Z → redo.
  // Registered once; always calls through refs so it never captures stale state.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (!e.ctrlKey) return;
      if (e.key === "z" && !e.shiftKey) { e.preventDefault(); undoRef.current(); }
      if (e.key === "y" || (e.key === "z" && e.shiftKey)) { e.preventDefault(); redoRef.current(); }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Listen for focus changes emitted by the Rust focus_tick background thread.
  // Updates which app chip is highlighted and keeps activeName in sync.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ process_name: string; profile_name: string | null }>(
      "active-app-changed",
      (evt) => {
        setFocusedApp(evt.payload.process_name || null);
        // Always sync the viewer to the engine's current active profile.
        // The Rust side now always sends the active profile name when running,
        // not only when the focused app has an explicit assignment.
        if (evt.payload.profile_name) {
          setActiveName(evt.payload.profile_name);
        }
      },
    ).then((fn) => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, []);

  // -----------------------------------------------------------------------
  // Derived state
  // -----------------------------------------------------------------------

  const activeProfile = store?.profiles[activeName] ?? null;
  const bands = activeProfile?.bands ?? [];

  // Derive the currently selected crossfeed option from the active profile.
  // Falls back to "Off" for old profiles that predate the crossfeed field.
  const crossfeedOption: "Off" | CrossfeedLevel = (() => {
    const cf = activeProfile?.crossfeed;
    if (!cf?.enabled) return "Off";
    return cf.level;
  })();

  // Bands with solo/mute applied — what the audio engine actually hears.
  // When no solo/mute is active this is the same array reference as `bands`
  // (computeEffective short-circuits) so downstream memos don't re-compute.
  const effectiveBands = useMemo(
    () => computeEffective(bands, mutedBands, soloedBand),
    [bands, mutedBands, soloedBand],
  );

  // Set of band indices that are silenced by solo/mute — used to dim dots on
  // the canvas and rows in the band list without touching the stored profile.
  const silencedIndices = useMemo<ReadonlySet<number>>(() => {
    const s = new Set<number>();
    if (soloedBand !== null) {
      for (let i = 0; i < bands.length; i++) {
        if (i !== soloedBand) s.add(i);
      }
    } else {
      mutedBands.forEach((i) => s.add(i));
    }
    return s;
  }, [bands.length, mutedBands, soloedBand]);

  // After a keyboard-driven reorder or delete the band array changes
  // Clamp the rover to a valid index when the band count changes (add / delete).
  useEffect(() => {
    if (bands.length === 0) return;
    setRovingBandIdx((prev) => Math.min(prev, bands.length - 1));
  }, [bands.length]);

  // asynchronously (IPC round-trip → refreshStore → setStore). We can't call
  // .focus() immediately because the DOM hasn't re-rendered yet. Instead, we
  // stash the target index in pendingFocusRef and focus it here once bands
  // (derived from store) reflects the new state.
  useEffect(() => {
    if (pendingFocusRef.current !== null) {
      const idx = pendingFocusRef.current;
      rowRefs.current[idx]?.focus();
      setRovingBandIdx(idx);
      pendingFocusRef.current = null;
    }
  }, [bands]);

  // -----------------------------------------------------------------------
  // Undo / redo history (band edits only)
  // -----------------------------------------------------------------------

  const hist = useBandHistory(bands);

  // Refs so the keyboard handler never goes stale between renders.
  const undoRef = useRef<() => void>(() => {});
  const redoRef = useRef<() => void>(() => {});

  // -----------------------------------------------------------------------
  // Engine handlers
  // -----------------------------------------------------------------------

  async function handleToggleEngine() {
    try {
      if (running) {
        await stopEngine();
        setRunning(false);
        // Reset bypass state — the engine is stopped so bypass is meaningless.
        setBypassed(false);
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

  async function handleToggleBypass() {
    const next = !bypassed;
    try {
      await setEqBypass(next);
      setBypassed(next);
    } catch (e) {
      showError(String(e));
    }
  }

  // Use a ref so the hold-repeat closure always reads the latest gain without
  // needing to be recreated on every render.
  const outputGainRef = useRef(outputGain);
  useEffect(() => { outputGainRef.current = outputGain; }, [outputGain]);

  function applyGainDelta(delta: number) {
    const next = Math.max(0.5, Math.min(2.0, Math.round((outputGainRef.current + delta) * 20) / 20));
    setOutputGainState(next);
    setOutputGain(next).catch((e) => showError(String(e)));
  }

  function startGainHold(delta: number) {
    applyGainDelta(delta);
    gainHoldTimer.current = setTimeout(() => {
      gainHoldInterval.current = setInterval(() => applyGainDelta(delta), 80);
    }, 400);
  }

  function stopGainHold() {
    if (gainHoldTimer.current)    { clearTimeout(gainHoldTimer.current);    gainHoldTimer.current    = null; }
    if (gainHoldInterval.current) { clearInterval(gainHoldInterval.current); gainHoldInterval.current = null; }
  }

  // -----------------------------------------------------------------------
  // Profile handlers
  // -----------------------------------------------------------------------

  async function handleSelectProfile(name: string) {
    hist.clearAll(); // history is per-profile; don't let edits from one bleed into another
    setMutedBands(new Set()); // solo/mute is per-session, not per-profile
    setSoloedBand(null);
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

  async function handleRenameProfile(oldName: string, newName: string) {
    try {
      await renameProfile(oldName, newName);
      await refreshStore();
      // If the active profile was the one renamed, follow it to the new name.
      if (activeName === oldName) setActiveName(newName);
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
        hist.clearAll();
        setMutedBands(new Set());
        setSoloedBand(null);
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

  // Exports a profile as a .json file download in the browser.
  async function handleExportProfile(name: string) {
    try {
      const json = await exportProfile(name);
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${name}.json`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch (e) {
      showError(String(e));
    }
  }

  // Parses a dropped/selected profile JSON and adds it to the store.
  async function handleImportProfile(json: string) {
    try {
      const finalName = await importProfile(json);
      const s = await refreshStore();
      if (s) setActiveName(finalName);
      showNotice(`Imported "${finalName}"`);
    } catch (e) {
      showError(String(e));
    }
  }

  // -----------------------------------------------------------------------
  // Band handlers
  // -----------------------------------------------------------------------

  // Core apply — persists bands to the store and updates the audio engine.
  // Does NOT touch the undo history; callers decide whether to push first.
  //
  // When solo/mute is active we skip set_active_profile (which would restore
  // the full profile) and apply only the effective (un-silenced) bands instead,
  // so the muted/soloed state is preserved across band edits.
  async function applyBands(newBands: BandConfig[]) {
    try {
      await updateProfileBands(activeName, newBands);
      await refreshStore();
      if (running) {
        if (soloedBand !== null || mutedBands.size > 0) {
          const effective = computeEffective(newBands, mutedBands, soloedBand);
          await applyBandsLive(effective).catch(() => {});
        } else {
          await setActiveProfile(activeName).catch(() => {});
        }
      }
    } catch (e) {
      showError(String(e));
    }
  }

  // Multi-band user change (add, delete, reorder, preset load).
  // Clears all per-band history since indices may shift after the change.
  async function handleBandsChange(newBands: BandConfig[]) {
    hist.clearAll();
    await applyBands(newBands);
  }

  // BandRow field commit — snapshots only the one band that changed,
  // so undo reverts just that band without touching others.
  function handleBandChange(index: number, updated: BandConfig) {
    hist.pushBand(index);
    const next = bands.map((b, i) => (i === index ? updated : b));
    applyBands(next);
  }

  // Color-only change: persists the new color but never touches the audio engine.
  // Color is cosmetic; routing it through applyBands would rebuild the FilterChain
  // and reset biquad delay lines, causing an audible click/static.
  async function handleBandColorChange(index: number, color: string) {
    const next = bands.map((b, i) => (i === index ? { ...b, color } : b));
    try {
      await updateProfileBands(activeName, next);
      await refreshStore();
    } catch (e) {
      showError(String(e));
    }
  }

  // Canvas dot drag start — snapshot just this band once for the whole drag.
  // handleBandDragStart already does this; canvas drag frames call applyBands directly.
  function handleBandDragStart(index: number) {
    hist.pushBand(index);
  }

  async function handleBandUndo(index: number) {
    const prev = hist.undoBand(index);
    if (prev) await applyBands(prev);
  }

  async function handleBandRedo(index: number) {
    const next = hist.redoBand(index);
    if (next) await applyBands(next);
  }

  // Canvas dot drag frame — applies the position WITHOUT pushing to history
  // (handleBandDragStart already did that for this drag operation).
  function handleCanvasBandChange(index: number, updated: BandConfig) {
    const next = bands.map((b, i) => (i === index ? updated : b));
    applyBands(next);
  }

  async function handleUndo() {
    const prev = hist.undoLast();
    if (prev) await applyBands(prev);
  }

  async function handleRedo() {
    const next = hist.redoLast();
    if (next) await applyBands(next);
  }

  // Keep refs current so the keyboard effect never captures stale closures.
  undoRef.current = handleUndo;
  redoRef.current = handleRedo;

  function handleBandDelete(index: number) {
    handleBandsChange(bands.filter((_, i) => i !== index));
  }

  function handleAddBand() {
    // Store the default color explicitly so the band's color never changes
    // if it is later reordered (index-based fallbacks would shift it).
    handleBandsChange([...bands, { ...DEFAULT_BAND, color: DEFAULT_BAND_COLOR }]);
  }

  // Toggles mute for a single band.
  // The stored profile is never changed — mute only affects the live engine.
  // Solo takes priority: if a band is soloed, muting another band has no extra
  // audible effect until solo is cleared, but the muted state is remembered.
  function handleToggleMute(index: number) {
    const next = new Set(mutedBands);
    if (next.has(index)) next.delete(index);
    else next.add(index);
    setMutedBands(next);

    const effective = computeEffective(bands, next, soloedBand);
    if (next.size > 0 || soloedBand !== null) {
      applyBandsLive(effective).catch(() => {});
    } else {
      // All mutes cleared and no solo — restore the stored profile.
      if (running) setActiveProfile(activeName).catch(() => {});
    }
  }

  // Toggles solo for a single band (clicking the active solo band un-solos it).
  function handleToggleSolo(index: number) {
    const newSolo = soloedBand === index ? null : index;
    setSoloedBand(newSolo);

    const effective = computeEffective(bands, mutedBands, newSolo);
    if (newSolo !== null || mutedBands.size > 0) {
      applyBandsLive(effective).catch(() => {});
    } else {
      // Solo cleared and no mutes — restore the stored profile.
      if (running) setActiveProfile(activeName).catch(() => {});
    }
  }

  function handleReorder(from: number, to: number) {
    if (from === to) return;
    const next = [...bands];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    handleBandsChange(next);
  }

  // Starts a mouse-driven drag-to-reorder for band rows.
  // We attach mousemove/mouseup to the window so the drag keeps working even
  // if the cursor leaves the list container.
  function startDrag(e: React.MouseEvent, fromIndex: number) {
    e.preventDefault();
    setDraggingIndex(fromIndex);
    let toIndex = fromIndex;

    // Walk the row refs to find which row the cursor is closest to.
    function getTargetIndex(clientY: number): number {
      for (let i = 0; i < rowRefs.current.length; i++) {
        const el = rowRefs.current[i];
        if (!el) continue;
        const rect = el.getBoundingClientRect();
        if (clientY < rect.top + rect.height / 2) return i;
      }
      return rowRefs.current.length - 1;
    }

    function onMove(ev: MouseEvent) {
      const next = getTargetIndex(ev.clientY);
      if (next !== toIndex) {
        toIndex = next;
        setDragOverIndex(toIndex);
      }
    }

    function onUp() {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      setDraggingIndex(null);
      setDragOverIndex(null);
      if (toIndex !== fromIndex) handleReorder(fromIndex, toIndex);
    }

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  // -----------------------------------------------------------------------
  // Session handlers
  // -----------------------------------------------------------------------

  async function handleAssign(processName: string, profileName: string) {
    try {
      await assignApp(processName, profileName);
      await refreshStore();
      // Apply immediately if this app is currently focused — no need to wait
      // for the next 500 ms focus tick to pick up the assignment change.
      if (running && focusedApp === processName) {
        await applyAppProfile(processName).catch((e) => showError(String(e)));
        setActiveName(profileName);
      }
    } catch (e) {
      showError(String(e));
    }
  }

  // -----------------------------------------------------------------------
  // Crossfeed handler
  // -----------------------------------------------------------------------

  // The four options presented in the UI: "Off" or one of the three preset levels.
  type CrossfeedOption = "Off" | CrossfeedLevel;

  async function handleCrossfeedChange(option: CrossfeedOption) {
    const config: CrossfeedConfig =
      option === "Off"
        ? { enabled: false, level: "Mild" }
        : { enabled: true, level: option };

    // Optimistic update so the button highlights immediately.
    setStore((prev) => {
      if (!prev) return prev;
      const profile = prev.profiles[activeName];
      if (!profile) return prev;
      return {
        ...prev,
        profiles: { ...prev.profiles, [activeName]: { ...profile, crossfeed: config } },
      };
    });

    try {
      await setProfileCrossfeed(activeName, config);
    } catch (e) {
      showError(String(e));
      await refreshStore(); // roll back optimistic update on error
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

  async function handleSetAppEnabled(processName: string, enabled: boolean) {
    try {
      await setAppEnabled(processName, enabled);
      await refreshStore();
    } catch (e) {
      showError(String(e));
    }
  }

  async function handleSetVolume(processName: string, volume: number) {
    // Optimistically update the store so the slider position reflects the
    // change immediately without waiting for the IPC round-trip to complete.
    setStore((prev) =>
      prev
        ? { ...prev, app_volumes: { ...prev.app_volumes, [processName]: volume } }
        : prev,
    );
    try {
      await setAppVolume(processName, volume);
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

        {/* Capture source — always VB-Cable; not user-configurable */}
        <div className="flex items-center gap-1.5">
          <span className="text-[10px] text-gray-500 uppercase tracking-wider shrink-0">Capture</span>
          {captureDeviceId
            ? <span className="text-xs text-gray-400 max-w-[190px] truncate" title={devices.find(d => d.id === captureDeviceId)?.name}>
                {devices.find(d => d.id === captureDeviceId)?.name ?? "CABLE Input"}
              </span>
            : <span className="text-xs text-amber-400">VB-Cable not found</span>
          }
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



        {/* Output gain — compensates for VB-Cable's lower signal level vs. direct output.
            Hold the button for continuous scrolling (400 ms delay, 80 ms repeat). */}
        <div className="flex items-center gap-1" title="Output gain — hold to scroll. Boost if soundEQ is quieter than direct headset output.">
          <span className="text-[10px] text-gray-500 uppercase tracking-wider">Vol</span>
          <button
            onMouseDown={() => startGainHold(-0.05)}
            onMouseUp={stopGainHold}
            onMouseLeave={stopGainHold}
            disabled={outputGain <= 0.5}
            className="w-5 h-5 flex items-center justify-center rounded text-gray-400 hover:text-white hover:bg-gray-700 disabled:opacity-30 disabled:cursor-not-allowed text-xs leading-none select-none"
          >−</button>
          <span className="text-xs text-gray-300 w-9 text-center tabular-nums">
            {Math.round(outputGain * 100)}%
          </span>
          <button
            onMouseDown={() => startGainHold(0.05)}
            onMouseUp={stopGainHold}
            onMouseLeave={stopGainHold}
            disabled={outputGain >= 2.0}
            className="w-5 h-5 flex items-center justify-center rounded text-gray-400 hover:text-white hover:bg-gray-700 disabled:opacity-30 disabled:cursor-not-allowed text-xs leading-none select-none"
          >+</button>
        </div>

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

        {/* EQ bypass toggle — only meaningful while the engine is running */}
        <button
          onClick={handleToggleBypass}
          disabled={!running}
          title={bypassed ? "Re-enable EQ processing (Ctrl+Shift+B)" : "Bypass EQ — pass audio through unmodified (Ctrl+Shift+B)"}
          className={`flex items-center gap-1.5 px-2.5 py-1 rounded text-xs transition-colors ${
            bypassed
              ? "bg-amber-900/60 text-amber-300 border border-amber-700"
              : "text-gray-600 hover:text-gray-400 border border-gray-800 hover:border-gray-600 disabled:opacity-30 disabled:cursor-not-allowed"
          }`}
        >
          <span className="text-[10px]">⊘</span>
          Bypass
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
          onRename={handleRenameProfile}
          onLoadPreset={handleLoadPreset}
          onExport={handleExportProfile}
          onImportFile={handleImportProfile}
        />

        {/* Main content */}
        <div className="flex flex-col flex-1 min-h-0">

          {/* EQ curve + spectrum analyzer */}
          <div className="flex-shrink-0 bg-gray-950 border-b border-gray-800">
            <EqCanvas
              frequencyResponse={curve}
              bands={effectiveBands}
              allBands={bands}
              bypassed={bypassed}
              spectrumData={spectrumData}
              silenced={silencedIndices}
              onBandDragStart={handleBandDragStart}
              onBandChange={handleCanvasBandChange}
            />
          </div>

          {/* ── Crossfeed control bar ────────────────────────────────────── */}
          {/* Per-profile crossfeed toggle. Shown as a segmented button group:
              "Off" disables crossfeed; "Mild / Moderate / Strong" sets the blend level.
              Crossfeed is only audible through headphones — no effect on speakers. */}
          <div className="flex-shrink-0 flex items-center gap-2 px-4 py-1.5 bg-gray-900 border-b border-gray-800">
            <span className="text-[10px] text-gray-500 uppercase tracking-wider mr-1">
              Crossfeed
            </span>
            {(["Off", "Mild", "Moderate", "Strong"] as const).map((opt) => (
              <button
                key={opt}
                onClick={() => handleCrossfeedChange(opt)}
                title={
                  opt === "Off"
                    ? "Disable crossfeed — normal headphone stereo"
                    : opt === "Mild"
                    ? "Mild crossfeed — subtle, wide stereo image (25% blend)"
                    : opt === "Moderate"
                    ? "Moderate crossfeed — balanced speaker-like sound (40% blend)"
                    : "Strong crossfeed — pronounced speaker simulation (55% blend)"
                }
                className={`text-xs px-2.5 py-0.5 rounded transition-colors ${
                  crossfeedOption === opt
                    ? "bg-indigo-700 text-white"
                    : "text-gray-500 hover:text-gray-300 hover:bg-gray-800"
                }`}
              >
                {opt}
              </button>
            ))}
          </div>

          {/* ── Band list ────────────────────────────────────────────────── */}
          <div className="flex-1 min-h-0 overflow-y-auto p-3 flex flex-col gap-1.5">

            {bands.length === 0 && (
              <div className="flex flex-col items-center justify-center h-24 text-center">
                <p className="text-xs text-slate-600">No EQ bands</p>
                <p className="text-[10px] text-slate-700 mt-1">
                  Add a band below or load a preset from the sidebar.
                </p>
              </div>
            )}

            {bands.map((band, i) => (
              // ---------------------------------------------------------------------------
              // Keyboard navigation for band rows  (roving tabindex pattern)
              //
              // One row at a time has tabIndex=0 (the "rover") so Tab from elsewhere
              // in the UI lands on the last-focused band row. Arrow keys move the rover.
              // Any focus within a row (clicking an input, etc.) also claims the rover.
              //
              // Shortcuts when the row container itself has focus (row-level mode):
              //   ↑ / ↓             — move focus to adjacent row
              //   Shift+↑ / Shift+↓  — reorder this band up or down one position
              //   Enter / F2         — enter edit mode (focus the first field in the row)
              //   Delete / Backspace — delete this band
              //   Space              — toggle band enabled
              //   m                  — toggle mute
              //   s                  — toggle solo
              //
              // Shortcut available from anywhere inside the row:
              //   Escape — exit edit mode (blur the focused field, return to row)
              // ---------------------------------------------------------------------------
              <div
                key={i}
                ref={(el) => { rowRefs.current[i] = el; }}
                tabIndex={rovingBandIdx === i ? 0 : -1}
                className="rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500/60"
                onFocus={() => setRovingBandIdx(i)}
                onKeyDown={(e) => {
                  // Escape from any child exits back to row-level navigation mode.
                  if (e.key === "Escape") {
                    e.preventDefault();
                    (e.target as HTMLElement).blur();
                    rowRefs.current[i]?.focus();
                    return;
                  }
                  // All other shortcuts only fire when the row container itself has focus,
                  // not when a child input, select, or button is focused. This prevents
                  // arrow keys from hijacking text cursor or select-dropdown navigation.
                  if (e.target !== e.currentTarget) return;

                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    if (e.shiftKey) {
                      if (i < bands.length - 1) {
                        handleReorder(i, i + 1);
                        pendingFocusRef.current = i + 1;
                      }
                    } else if (i < bands.length - 1) {
                      rowRefs.current[i + 1]?.focus();
                    }
                  } else if (e.key === "ArrowUp") {
                    e.preventDefault();
                    if (e.shiftKey) {
                      if (i > 0) {
                        handleReorder(i, i - 1);
                        pendingFocusRef.current = i - 1;
                      }
                    } else if (i > 0) {
                      rowRefs.current[i - 1]?.focus();
                    }
                  } else if (e.key === "Enter" || e.key === "F2") {
                    // Enter edit mode — focus the filter-type select.
                    e.preventDefault();
                    rowRefs.current[i]
                      ?.querySelector<HTMLElement>('select, input[type="number"]')
                      ?.focus();
                  } else if (e.key === "Delete" || e.key === "Backspace") {
                    e.preventDefault();
                    handleBandDelete(i);
                    // After deletion, focus the band that slides into this position,
                    // or the one before it if we just deleted the last band.
                    pendingFocusRef.current = Math.max(0, i >= bands.length - 1 ? i - 1 : i);
                  } else if (e.key === " ") {
                    e.preventDefault();
                    handleBandChange(i, { ...bands[i], enabled: !bands[i].enabled });
                  } else if (e.key === "m") {
                    e.preventDefault();
                    handleToggleMute(i);
                  } else if (e.key === "s") {
                    e.preventDefault();
                    handleToggleSolo(i);
                  }
                }}
              >
                <BandRow
                  band={band}
                  index={i}
                  isDragging={draggingIndex === i}
                  isDragTarget={
                    draggingIndex !== null &&
                    draggingIndex !== i &&
                    dragOverIndex === i
                  }
                  dragTargetColor={
                    draggingIndex !== null
                      ? bandColor(draggingIndex, bands[draggingIndex]?.color)
                      : "#6366f1"
                  }
                  isMuted={mutedBands.has(i)}
                  isSoloed={soloedBand === i}
                  anySoloed={soloedBand !== null}
                  onGripMouseDown={(e) => startDrag(e, i)}
                  onChange={handleBandChange}
                  onColorChange={handleBandColorChange}
                  onDelete={handleBandDelete}
                  onMute={handleToggleMute}
                  onSolo={handleToggleSolo}
                  onUndo={handleBandUndo}
                  onRedo={handleBandRedo}
                  canUndo={hist.canUndoBand(i)}
                  canRedo={hist.canRedoBand(i)}
                />
              </div>
            ))}

            {bands.length < 16 && (
              <button
                onClick={handleAddBand}
                className="h-10 w-full rounded-xl border border-dashed border-slate-800 hover:border-slate-700 hover:bg-slate-900/40 text-slate-600 hover:text-slate-400 text-xs transition-all"
              >
                + Add Band
              </button>
            )}
          </div>

          {/* App assignment bar */}
          <AppPanel
            store={store}
            activeSessions={sessions}
            focusedApp={focusedApp}
            appVolumes={store?.app_volumes ?? {}}
            onAssign={handleAssign}
            onUnassign={handleUnassign}
            onSetEnabled={handleSetAppEnabled}
            onSetVolume={handleSetVolume}
          />
        </div>
      </div>

      {/* ------------------------------------------------------------------ */}
      {/* Notice toast (device auto-restart, etc.) */}
      {/* ------------------------------------------------------------------ */}
      {notice && (
        <div className="fixed bottom-4 left-1/2 -translate-x-1/2 bg-teal-900 text-teal-200 text-xs px-4 py-2 rounded shadow-lg z-50">
          {notice}
        </div>
      )}

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
