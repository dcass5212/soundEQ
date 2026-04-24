import { useEffect, useState } from "react";
import type { BandConfig, FilterType } from "../lib/api";

// ---------------------------------------------------------------------------
// Filter type metadata
// ---------------------------------------------------------------------------

const FILTER_OPTIONS: { value: FilterType; label: string }[] = [
  { value: "peak",       label: "Peak"      },
  { value: "low_shelf",  label: "Lo Shelf"  },
  { value: "high_shelf", label: "Hi Shelf"  },
  { value: "low_pass",   label: "Lo Pass"   },
  { value: "high_pass",  label: "Hi Pass"   },
  { value: "notch",      label: "Notch"     },
  { value: "bandpass",   label: "Bandpass"  },
];

const HAS_GAIN = new Set<FilterType>(["peak", "low_shelf", "high_shelf"]);

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface Props {
  band: BandConfig;
  index: number;
  onChange: (index: number, updated: BandConfig) => void;
  onDelete: (index: number) => void;
}

export function BandRow({ band, index, onChange, onDelete }: Props) {
  // Local string state so inputs are editable mid-type.
  const [freq, setFreq] = useState(String(band.frequency));
  const [gain, setGain] = useState(String(band.gain_db));
  const [q,    setQ]    = useState(String(band.q));

  // Sync if parent replaces the band (e.g. preset load).
  useEffect(() => { setFreq(String(band.frequency)); }, [band.frequency]);
  useEffect(() => { setGain(String(band.gain_db));   }, [band.gain_db]);
  useEffect(() => { setQ(String(band.q));            }, [band.q]);

  function commit() {
    onChange(index, {
      ...band,
      frequency: clamp(parseFloat(freq) || band.frequency, 20, 20_000),
      gain_db:   clamp(parseFloat(gain) || band.gain_db, -24, 24),
      q:         clamp(parseFloat(q)    || band.q, 0.1, 20),
    });
  }

  return (
    <div
      className={`flex items-center gap-2 px-2 py-1.5 rounded text-sm ${
        band.enabled ? "bg-gray-800" : "bg-gray-900 opacity-50"
      }`}
    >
      {/* Band number */}
      <span className="w-4 text-xs text-gray-500 text-right flex-shrink-0">
        {index + 1}
      </span>

      {/* Filter type */}
      <select
        value={band.filter_type}
        onChange={(e) =>
          onChange(index, { ...band, filter_type: e.target.value as FilterType })
        }
        className="bg-gray-700 text-gray-200 rounded px-1.5 py-0.5 text-xs border border-gray-600 cursor-pointer w-24 flex-shrink-0"
      >
        {FILTER_OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>

      {/* Frequency */}
      <label className="flex items-center gap-1 flex-shrink-0">
        <input
          type="number"
          value={freq}
          min={20}
          max={20_000}
          step={1}
          onChange={(e) => setFreq(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
          className="w-20 bg-gray-700 text-gray-100 rounded px-1.5 py-0.5 text-xs border border-gray-600 text-right"
        />
        <span className="text-gray-500 text-xs">Hz</span>
      </label>

      {/* Gain (only for peak/shelf types) */}
      {HAS_GAIN.has(band.filter_type) ? (
        <label className="flex items-center gap-1 flex-shrink-0">
          <input
            type="number"
            value={gain}
            min={-24}
            max={24}
            step={0.5}
            onChange={(e) => setGain(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => e.key === "Enter" && commit()}
            className="w-16 bg-gray-700 text-gray-100 rounded px-1.5 py-0.5 text-xs border border-gray-600 text-right"
          />
          <span className="text-gray-500 text-xs">dB</span>
        </label>
      ) : (
        /* spacer so columns align */
        <div className="w-[5.5rem] flex-shrink-0" />
      )}

      {/* Q */}
      <label className="flex items-center gap-1 flex-shrink-0">
        <input
          type="number"
          value={q}
          min={0.1}
          max={20}
          step={0.1}
          onChange={(e) => setQ(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
          className="w-14 bg-gray-700 text-gray-100 rounded px-1.5 py-0.5 text-xs border border-gray-600 text-right"
        />
        <span className="text-gray-500 text-xs">Q</span>
      </label>

      {/* Enable toggle */}
      <button
        onClick={() => onChange(index, { ...band, enabled: !band.enabled })}
        title={band.enabled ? "Disable band" : "Enable band"}
        className={`w-5 h-5 rounded-full border flex-shrink-0 transition-colors ${
          band.enabled
            ? "bg-indigo-500 border-indigo-400"
            : "bg-gray-700 border-gray-600"
        }`}
      />

      {/* Delete */}
      <button
        onClick={() => onDelete(index)}
        title="Delete band"
        className="ml-auto text-gray-600 hover:text-red-400 text-base leading-none flex-shrink-0 px-1"
      >
        ×
      </button>
    </div>
  );
}

function clamp(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, v));
}
