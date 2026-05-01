// =============================================================================
// BandRow.tsx — A single EQ band control row
//
// Each row shows the band's color accent, drag grip, filter type, frequency,
// gain, Q, enable toggle, color picker, and delete button. The drag handle
// works via mouse events (not HTML5 DnD, which is unreliable in Tauri).
// =============================================================================

import { useEffect, useRef, useState } from "react";
import type { BandConfig, FilterType } from "../lib/api";
import { bandColor, hexToRgba } from "../lib/colors";

const FILTER_OPTIONS: { value: FilterType; label: string }[] = [
  { value: "peak",       label: "Peak"     },
  { value: "low_shelf",  label: "Lo Shelf" },
  { value: "high_shelf", label: "Hi Shelf" },
  { value: "low_pass",   label: "Lo Pass"  },
  { value: "high_pass",  label: "Hi Pass"  },
  { value: "notch",      label: "Notch"    },
  { value: "bandpass",   label: "Bandpass" },
];

const HAS_GAIN = new Set<FilterType>(["peak", "low_shelf", "high_shelf"]);

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface Props {
  band: BandConfig;
  index: number;
  /** True when this band is the one currently being dragged — dims the row. */
  isDragging: boolean;
  /** True when the dragged band will land at this position — shows a ring. */
  isDragTarget: boolean;
  /** Color of the band being dragged, used for the drop-target ring. */
  dragTargetColor: string;
  /** True when this band is muted (bypassed without disabling). */
  isMuted: boolean;
  /** True when this band is the soloed band (all others are silenced). */
  isSoloed: boolean;
  /** True when ANY band is currently soloed — dims all non-soloed rows. */
  anySoloed: boolean;
  /** Called when the user presses the drag grip — App.tsx starts the drag. */
  onGripMouseDown: (e: React.MouseEvent) => void;
  onChange: (index: number, updated: BandConfig) => void;
  onColorChange: (index: number, color: string) => void;
  onDelete: (index: number) => void;
  onMute: (index: number) => void;
  onSolo: (index: number) => void;
  onUndo: (index: number) => void;
  onRedo: (index: number) => void;
  canUndo: boolean;
  canRedo: boolean;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function BandRow({
  band, index,
  isDragging, isDragTarget, dragTargetColor,
  isMuted, isSoloed, anySoloed,
  onGripMouseDown, onChange, onColorChange, onDelete, onMute, onSolo,
  onUndo, onRedo, canUndo, canRedo,
}: Props) {
  // Local string state lets the user type freely without every keystroke
  // being clamped and propagated to the backend.
  const [freq, setFreq] = useState(String(band.frequency));
  const [gain, setGain] = useState(String(band.gain_db));
  const [q,    setQ]    = useState(String(band.q));
  const colorInputRef   = useRef<HTMLInputElement>(null);

  // Palette color for this index, overridden by the user's stored choice.
  const color = bandColor(index, band.color);

  // Sync displayed values if the parent replaces the band (e.g. preset load).
  useEffect(() => { setFreq(String(band.frequency)); }, [band.frequency]);
  useEffect(() => { setGain(String(band.gain_db));   }, [band.gain_db]);
  useEffect(() => { setQ(String(band.q));            }, [band.q]);

  // Validates, clamps, and commits all three numeric fields at once.
  function commit() {
    onChange(index, {
      ...band,
      frequency: clamp(parseFloat(freq) || band.frequency, 20, 20_000),
      gain_db:   clamp(parseFloat(gain) || band.gain_db,   -24, 24),
      q:         clamp(parseFloat(q)    || band.q,          0.1, 20),
    });
  }

  // Shared class string for every number input.
  // Spinner arrows are hidden — the values are adjusted by typing or by
  // dragging the dot on the EQ canvas.
  const numCls =
    "w-full bg-slate-800/80 text-slate-100 text-xs rounded-lg px-2 py-1.5 " +
    "border border-white/5 text-right focus:outline-none focus:border-white/20 " +
    "transition-colors [appearance:textfield] " +
    "[&::-webkit-inner-spin-button]:appearance-none " +
    "[&::-webkit-outer-spin-button]:appearance-none";

  // A band is "silenced" when it is muted, or when another band is soloed.
  // Silenced bands stay in the list (can still be edited) but are visually
  // dimmed to make clear they're not contributing to the audio output.
  const silenced = isMuted || (anySoloed && !isSoloed);
  const dimmed   = !band.enabled;

  return (
    <div
      className={[
        "group relative flex items-center h-12 rounded-xl border",
        "overflow-hidden transition-all duration-150 select-none",
        dimmed && !silenced
          ? "bg-slate-900/40 border-white/[0.04] opacity-50"
          : silenced
          ? "bg-slate-900/30 border-white/[0.03] opacity-35"
          : "bg-slate-900  border-white/[0.07]",
        isDragging ? "opacity-20 scale-[0.98] pointer-events-none" : "",
      ].join(" ")}
      style={isDragTarget ? {
        outline: `2px solid ${dragTargetColor}`,
        outlineOffset: "2px",
      } : undefined}
    >
      {/* ── Left color accent bar ──────────────────────────────────────────── */}
      {/* 3 px inset-left shadow acts as a colored left border without
          breaking border-radius or overflow:hidden */}
      <div
        className="absolute inset-y-0 left-0 w-[3px]"
        style={{ backgroundColor: dimmed ? hexToRgba(color, 0.35) : color }}
      />

      {/* ── Drag grip ──────────────────────────────────────────────────────── */}
      {/* Mouse events only — HTML5 DnD is unreliable inside Tauri's WebView. */}
      <button
        onMouseDown={onGripMouseDown}
        className="pl-3.5 pr-2 self-stretch flex items-center flex-shrink-0 text-slate-600 hover:text-slate-400 cursor-grab active:cursor-grabbing transition-colors"
        tabIndex={-1}
        title="Drag to reorder"
      >
        <svg width="8" height="13" viewBox="0 0 8 13" fill="currentColor">
          <circle cx="1.5" cy="1.5"  r="1.15" />
          <circle cx="6.5" cy="1.5"  r="1.15" />
          <circle cx="1.5" cy="6.5"  r="1.15" />
          <circle cx="6.5" cy="6.5"  r="1.15" />
          <circle cx="1.5" cy="11.5" r="1.15" />
          <circle cx="6.5" cy="11.5" r="1.15" />
        </svg>
      </button>

      {/* ── Band index ─────────────────────────────────────────────────────── */}
      <span className="w-5 flex-shrink-0 text-center text-[10px] text-slate-600 font-mono">
        {index + 1}
      </span>

      {/* ── Filter type ────────────────────────────────────────────────────── */}
      <select
        value={band.filter_type}
        onChange={(e) =>
          onChange(index, { ...band, filter_type: e.target.value as FilterType })
        }
        className="mx-2 w-[90px] flex-shrink-0 bg-slate-800/80 text-slate-200 text-xs rounded-lg px-2 py-1.5 border border-white/5 focus:outline-none focus:border-white/20 cursor-pointer transition-colors"
      >
        {FILTER_OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>{o.label}</option>
        ))}
      </select>

      {/* ── Frequency ──────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-1 mx-1 flex-shrink-0" style={{ width: 96 }}>
        <input
          type="number"
          value={freq}
          min={20} max={20_000} step={1}
          onChange={(e) => setFreq(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
          className={numCls}
        />
        <span className="text-[10px] text-slate-500 flex-shrink-0 w-5">Hz</span>
      </div>

      {/* ── Gain (peak / shelves only) — spacer keeps columns aligned ─────── */}
      <div className="flex items-center gap-1 mx-1 flex-shrink-0" style={{ width: 80 }}>
        {HAS_GAIN.has(band.filter_type) ? (
          <>
            <input
              type="number"
              value={gain}
              min={-24} max={24} step={0.5}
              onChange={(e) => setGain(e.target.value)}
              onBlur={commit}
              onKeyDown={(e) => e.key === "Enter" && commit()}
              className={numCls}
            />
            <span className="text-[10px] text-slate-500 flex-shrink-0 w-5">dB</span>
          </>
        ) : null}
      </div>

      {/* ── Q factor ───────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-1 mx-1 flex-shrink-0" style={{ width: 72 }}>
        <span className="text-[10px] text-slate-500 flex-shrink-0">Q</span>
        <input
          type="number"
          value={q}
          min={0.1} max={20} step={0.1}
          onChange={(e) => setQ(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
          className={numCls}
        />
      </div>

      {/* ── Enable/disable pill toggle ─────────────────────────────────────── */}
      {/* Pill switches are more legible than circles for an on/off state. */}
      <button
        onClick={() => onChange(index, { ...band, enabled: !band.enabled })}
        title={band.enabled ? "Disable band" : "Enable band"}
        className="relative mx-2 flex-shrink-0 w-8 h-[18px] rounded-full transition-colors duration-200"
        style={{ backgroundColor: band.enabled ? hexToRgba(color, 0.85) : "#1e293b" }}
      >
        <span
          className="absolute top-[2px] w-[14px] h-[14px] rounded-full bg-white shadow-sm transition-all duration-200"
          style={{ left: band.enabled ? "calc(100% - 16px)" : "2px" }}
        />
      </button>

      {/* ── Mute button (M) ────────────────────────────────────────────────── */}
      {/* Mutes this band in the live engine without touching the stored profile.
          Amber background when muted; outlined when off. */}
      <button
        onClick={() => onMute(index)}
        title={isMuted ? "Unmute band" : "Mute band"}
        className={[
          "mx-1 flex-shrink-0 w-[22px] h-[22px] rounded text-[10px] font-bold",
          "transition-colors",
          isMuted
            ? "bg-amber-500/90 text-white"
            : "bg-slate-800/80 text-slate-500 hover:text-amber-400 border border-white/5",
        ].join(" ")}
      >
        M
      </button>

      {/* ── Solo button (S) ─────────────────────────────────────────────────── */}
      {/* Solos this band — silences every other band in the live engine.
          Clicking the active solo band un-solos it. */}
      <button
        onClick={() => onSolo(index)}
        title={isSoloed ? "Unsolo band" : "Solo this band only"}
        className={[
          "mx-1 flex-shrink-0 w-[22px] h-[22px] rounded text-[10px] font-bold",
          "transition-colors",
          isSoloed
            ? "bg-yellow-400/90 text-gray-900"
            : "bg-slate-800/80 text-slate-500 hover:text-yellow-400 border border-white/5",
        ].join(" ")}
      >
        S
      </button>

      {/* ── Per-band undo / redo ───────────────────────────────────────────── */}
      <button
        onClick={() => onUndo(index)}
        disabled={!canUndo}
        title="Undo last change to this band"
        className="mx-0.5 flex-shrink-0 w-[22px] h-[22px] rounded text-[11px] bg-slate-800/80 border border-white/5 text-slate-500 hover:text-slate-200 disabled:opacity-20 disabled:cursor-not-allowed transition-colors"
      >↩</button>
      <button
        onClick={() => onRedo(index)}
        disabled={!canRedo}
        title="Redo last undone change to this band"
        className="mx-0.5 flex-shrink-0 w-[22px] h-[22px] rounded text-[11px] bg-slate-800/80 border border-white/5 text-slate-500 hover:text-slate-200 disabled:opacity-20 disabled:cursor-not-allowed transition-colors"
      >↪</button>

      {/* ── Color swatch ───────────────────────────────────────────────────── */}
      <div className="relative flex-shrink-0">
        <button
          title="Change band color"
          onClick={() => colorInputRef.current?.click()}
          className="w-[18px] h-[18px] rounded-full border-2 border-white/10 hover:border-white/40 transition-colors"
          style={{ backgroundColor: color }}
        />
        <input
          ref={colorInputRef}
          type="color"
          value={color}
          onChange={(e) => onColorChange(index, e.target.value)}
          className="absolute top-0 left-0 opacity-0 pointer-events-none w-0 h-0"
          tabIndex={-1}
        />
      </div>

      {/* ── Delete ─────────────────────────────────────────────────────────── */}
      <button
        onClick={() => onDelete(index)}
        title="Delete band"
        className="ml-auto mr-2 flex-shrink-0 opacity-0 group-hover:opacity-100 text-slate-600 hover:text-red-400 transition-all"
      >
        <svg viewBox="0 0 14 14" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round">
          <line x1="2" y1="2" x2="12" y2="12" />
          <line x1="12" y1="2" x2="2" y2="12" />
        </svg>
      </button>
    </div>
  );
}

function clamp(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, v));
}
