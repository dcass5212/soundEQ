import { useState } from "react";
import type { ProfileStore, SessionInfo } from "../lib/api";

// ---------------------------------------------------------------------------
// AppPanel — per-app EQ profile assignments
//
// Collapsed state: a thin bar showing the app count and the currently-focused
// app name. A "Manage" button expands the full panel.
//
// Expanded state: a scrollable list of all assigned apps, each with:
//   - A pill toggle to enable/disable auto-switching without losing the assignment
//   - A green focus dot when that app currently has keyboard focus
//   - A profile dropdown
//   - A × remove button
// Below the list, an inline add-app form with running-app suggestions.
// ---------------------------------------------------------------------------

interface Props {
  store: ProfileStore | null;
  activeSessions: SessionInfo[];
  focusedApp: string | null;
  appVolumes: Record<string, number>;
  onAssign: (processName: string, profileName: string) => void;
  onUnassign: (processName: string) => void;
  onSetEnabled: (processName: string, enabled: boolean) => void;
  onSetVolume: (processName: string, volume: number) => void;
}

export function AppPanel({
  store,
  activeSessions,
  focusedApp,
  appVolumes,
  onAssign,
  onUnassign,
  onSetEnabled,
  onSetVolume,
}: Props) {
  const [expanded, setExpanded] = useState(false);
  const [inputName, setInputName] = useState("");
  const [inputProfile, setInputProfile] = useState("");

  // Resizable collapsed bar height — drag the top edge to adjust.
  const [barHeight, setBarHeight] = useState(90);
  // Font size scales with bar height so text fills the space naturally.
  const barFontSize = Math.max(11, Math.min(24, Math.round(barHeight * 0.167)));
  const barBtnSize = Math.max(12, Math.min(28, Math.round(barHeight * 0.2)));
  const dotSize = Math.max(6, Math.min(14, Math.round(barHeight * 0.1)));

  function startDragHeight(e: React.MouseEvent) {
    e.preventDefault();
    const startY = e.clientY;
    const startHeight = barHeight;

    function onMove(ev: MouseEvent) {
      // Dragging up (clientY decreasing) increases bar height.
      setBarHeight(Math.max(60, Math.min(180, startHeight + startY - ev.clientY)));
    }
    function onUp() {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    }
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  const profiles = store ? Object.values(store.profiles) : [];
  const defaultProfileName = store?.default_profile_name ?? "Default";

  const disabledApps = new Set(store?.disabled_apps ?? []);

  const assignments = store
    ? Object.entries(store.app_assignments).sort(([a], [b]) => a.localeCompare(b))
    : [];

  const enabledCount = assignments.filter(([name]) => !disabledApps.has(name)).length;

  const suggestions = activeSessions
    .filter(
      (s, i, arr) =>
        s.process_name !== "" &&
        arr.findIndex((s2) => s2.process_name === s.process_name) === i,
    )
    .filter((s) => !store?.app_assignments[s.process_name]);

  function handleAdd() {
    const name = inputName.trim();
    if (!name) return;
    onAssign(name, inputProfile || defaultProfileName);
    setInputName("");
    setInputProfile("");
  }

  const trackedFocusedApp =
    focusedApp &&
    store?.app_assignments[focusedApp] !== undefined &&
    !disabledApps.has(focusedApp)
      ? focusedApp
      : null;

  return (
    <div className="flex-shrink-0 border-t border-gray-800 relative">

      {/* ------------------------------------------------------------------ */}
      {/* Expanded panel — floats above the band list (absolute) so it never  */}
      {/* pushes the layout when the window is small.                         */}
      {/* ------------------------------------------------------------------ */}
      {expanded && (
        <div className="absolute bottom-full left-0 right-0 z-20 bg-gray-900 border-t border-x border-gray-800 shadow-xl">

          {/* App list ---------------------------------------------------- */}
          <div className="max-h-[220px] overflow-y-auto">
            {assignments.length === 0 ? (
              <p className="text-xs text-gray-600 px-4 py-3">
                No apps yet — use the form below to add one.
              </p>
            ) : (
              <div className="divide-y divide-gray-800/50">
                {assignments.map(([processName, profileName]) => {
                  const isFocused = processName === focusedApp;
                  const isEnabled = !disabledApps.has(processName);

                  return (
                    <div
                      key={processName}
                      className={`flex items-center gap-2.5 px-4 py-2 transition-all duration-200 ${
                        isEnabled ? "" : "opacity-40"
                      } ${isFocused ? "bg-emerald-950/40" : ""}`}
                    >
                      {/* Enable / disable pill toggle */}
                      <button
                        onClick={() => onSetEnabled(processName, !isEnabled)}
                        title={
                          isEnabled
                            ? "Disable auto-switching for this app"
                            : "Enable auto-switching for this app"
                        }
                        className={`relative flex-shrink-0 w-8 h-4 rounded-full transition-colors ${
                          isEnabled ? "bg-indigo-600" : "bg-gray-700"
                        }`}
                      >
                        <span
                          className={`absolute top-0.5 w-3 h-3 rounded-full bg-white shadow transition-all ${
                            isEnabled ? "left-[18px]" : "left-0.5"
                          }`}
                        />
                      </button>

                      {/* Focus indicator dot — always reserves space to prevent layout shift */}
                      <span
                        className={`w-1.5 h-1.5 rounded-full flex-shrink-0 transition-colors duration-200 ${
                          isFocused && isEnabled
                            ? "bg-emerald-400 animate-pulse"
                            : "bg-transparent"
                        }`}
                        title={isFocused && isEnabled ? "Currently focused" : undefined}
                      />

                      <span
                        className={`text-xs flex-1 min-w-0 truncate transition-colors duration-200 ${
                          isFocused && isEnabled ? "text-emerald-300 font-medium" : "text-gray-300"
                        }`}
                      >
                        {processName}
                      </span>

                      <select
                        value={profileName}
                        onChange={(e) => onAssign(processName, e.target.value)}
                        disabled={!isEnabled}
                        className="bg-gray-800 text-xs text-indigo-400 border border-gray-700 rounded px-1.5 py-0.5 focus:outline-none focus:border-indigo-500 disabled:cursor-not-allowed max-w-[130px]"
                      >
                        {profiles.map((p) => (
                          <option key={p.name} value={p.name}>
                            {p.name}
                          </option>
                        ))}
                      </select>

                      {/* Per-app volume slider — adjusts WASAPI session volume
                          independently of the master Windows volume. Range is
                          0–100% (linear). The value is stored in ProfileStore
                          and applied live to the WASAPI session on change. */}
                      <div className="flex items-center gap-1 flex-shrink-0" title={`Session volume: ${Math.round((appVolumes[processName] ?? 1.0) * 100)}%`}>
                        <input
                          type="range"
                          min={0}
                          max={100}
                          value={Math.round((appVolumes[processName] ?? 1.0) * 100)}
                          onChange={(e) => onSetVolume(processName, Number(e.target.value) / 100)}
                          disabled={!isEnabled}
                          className="w-16 h-1 accent-indigo-500 disabled:opacity-40 cursor-pointer disabled:cursor-not-allowed"
                        />
                        <span className="text-[10px] text-gray-600 w-7 tabular-nums text-right">
                          {Math.round((appVolumes[processName] ?? 1.0) * 100)}%
                        </span>
                      </div>

                      <button
                        onClick={() => onUnassign(processName)}
                        className="text-gray-600 hover:text-red-400 text-base leading-none flex-shrink-0 transition-colors"
                        title={`Remove ${processName}`}
                      >
                        ×
                      </button>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Add form ----------------------------------------------------- */}
          <div className="border-t border-gray-800 px-4 py-3">
            <p className="text-[10px] font-semibold tracking-widest text-gray-600 uppercase mb-2">
              Add App
            </p>
            <p className="text-[10px] text-gray-600 mb-2">
              Profile switches automatically when that app has focus.
              Multiple apps can't be EQ'd differently at the same time.
            </p>

            <div className="flex items-center gap-2">
              <input
                value={inputName}
                onChange={(e) => setInputName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleAdd();
                }}
                placeholder="e.g. spotify.exe"
                className="flex-1 bg-gray-800 text-gray-200 text-xs rounded px-2 py-1 border border-gray-700 focus:outline-none focus:border-indigo-500 placeholder-gray-600"
              />
              <select
                value={inputProfile || defaultProfileName}
                onChange={(e) => setInputProfile(e.target.value)}
                className="bg-gray-800 text-gray-200 text-xs rounded px-2 py-1 border border-gray-700 focus:outline-none"
              >
                {profiles.map((p) => (
                  <option key={p.name} value={p.name}>
                    {p.name}
                    {p.name === defaultProfileName ? " (default)" : ""}
                  </option>
                ))}
              </select>
              <button
                onClick={handleAdd}
                disabled={!inputName.trim()}
                className="px-3 py-1 text-xs bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed text-white rounded transition-colors"
              >
                Add
              </button>
            </div>

            {suggestions.length > 0 && (
              <div className="mt-2 flex items-center gap-2 flex-wrap">
                <span className="text-[10px] text-gray-600">Running:</span>
                {suggestions.map((s) => (
                  <button
                    key={s.process_name}
                    onClick={() => setInputName(s.process_name)}
                    className="text-[10px] text-indigo-400 hover:text-indigo-300 bg-indigo-950/40 border border-indigo-900/60 hover:border-indigo-700 px-1.5 py-0.5 rounded transition-colors"
                  >
                    {s.process_name}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      )}

      {/* ------------------------------------------------------------------ */}
      {/* Collapsed bar                                                        */}
      {/* ------------------------------------------------------------------ */}
      <div
        style={{ height: barHeight }}
        className="relative flex items-center gap-3 px-4 bg-gray-900"
      >
        {/* Top-edge drag handle — drag up to make the bar taller */}
        <div
          onMouseDown={startDragHeight}
          className="absolute top-0 left-0 right-0 h-1.5 cursor-row-resize hover:bg-indigo-500/30 transition-colors z-10"
        />

        <span
          style={{ fontSize: barFontSize }}
          className="font-semibold tracking-widest text-gray-600 uppercase flex-shrink-0 cursor-default"
          title="Per-app profiles switch the active EQ automatically when that app has focus. Multiple apps cannot be EQ'd differently at the same time — all audio is mixed by Windows before soundEQ processes it."
        >
          Apps ⓘ
        </span>

        {/* App chips — flex-shrink-0 on each chip so they don't compress;
            overflow-hidden on the container cuts off chips that don't fit.
            Clicking a chip toggles its enabled/disabled state. */}
        <div className="flex-1 min-w-0 flex items-center gap-3 overflow-hidden">
          {assignments.length === 0 ? (
            <span style={{ fontSize: Math.max(10, barFontSize - 2) }} className="text-gray-700 italic">
              No apps — click Manage to add one
            </span>
          ) : (
            assignments.map(([processName]) => {
              const isFocused = processName === trackedFocusedApp;
              const isEnabled = !disabledApps.has(processName);
              return (
                <button
                  key={processName}
                  onClick={() => onSetEnabled(processName, !isEnabled)}
                  title={isEnabled ? `Disable ${processName}` : `Enable ${processName}`}
                  style={{
                    fontSize: Math.max(10, barFontSize - 1),
                    // Glow ring — only shown when this app is actively focused.
                    boxShadow: isFocused
                      ? "0 0 0 1px #34d399, 0 0 12px rgba(52,211,153,0.35)"
                      : undefined,
                  }}
                  className={`flex-shrink-0 inline-flex items-center justify-center gap-1.5 px-2.5 py-1 rounded border transition-all duration-200 ${
                    isFocused
                      ? "bg-emerald-500/25 text-emerald-200 border-emerald-400/70"
                      : isEnabled
                      ? "bg-gray-800 text-gray-400 border-gray-700 hover:bg-gray-700 hover:text-gray-200"
                      : "bg-gray-900 text-gray-600 border-gray-800 opacity-50 hover:opacity-80"
                  }`}
                >
                  {isFocused && (
                    <span
                      style={{ width: dotSize, height: dotSize }}
                      className="rounded-full flex-shrink-0 bg-emerald-400 animate-pulse"
                    />
                  )}
                  {processName}
                </button>
              );
            })
          )}
        </div>

        <button
          onClick={() => setExpanded((v) => !v)}
          title={expanded ? "Close apps panel" : "Manage per-app profiles"}
          style={{ fontSize: barBtnSize }}
          className={`flex-shrink-0 px-4 py-1.5 rounded border transition-colors ${
            expanded
              ? "text-indigo-400 border-indigo-700 bg-indigo-950/50"
              : "text-gray-600 hover:text-indigo-400 border-gray-700 hover:border-indigo-600"
          }`}
        >
          {expanded ? "Close" : "Manage"}
        </button>
      </div>
    </div>
  );
}
