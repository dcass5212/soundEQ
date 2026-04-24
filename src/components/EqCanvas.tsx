import { useEffect, useRef } from "react";
import type { BandConfig } from "../lib/api";

// ---------------------------------------------------------------------------
// Coordinate math
// ---------------------------------------------------------------------------

const LOG_MIN = Math.log10(20);
const LOG_MAX = Math.log10(20_000);
const DB_RANGE = 24; // ±24 dB displayed

function freqToX(freq: number, w: number): number {
  const t = (Math.log10(Math.max(20, Math.min(20_000, freq))) - LOG_MIN) / (LOG_MAX - LOG_MIN);
  return t * w;
}

function dbToY(db: number, h: number): number {
  // Center at h/2; positive dB goes up (lower Y value).
  return h / 2 - (db / DB_RANGE) * (h / 2);
}

// ---------------------------------------------------------------------------
// Canvas constants
// ---------------------------------------------------------------------------

const GRID_FREQS = [20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000];
const GRID_DBS   = [-24, -18, -12, -6, 0, 6, 12, 18, 24];

const LABEL_FREQS: [number, string][] = [
  [50, "50"], [100, "100"], [200, "200"], [500, "500"],
  [1_000, "1k"], [2_000, "2k"], [5_000, "5k"], [10_000, "10k"],
];

const HAS_GAIN_TYPES = new Set(["peak", "low_shelf", "high_shelf"]);

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface Props {
  frequencyResponse: [number, number][];
  bands: BandConfig[];
}

export function EqCanvas({ frequencyResponse, bands }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const W = canvas.width;
    const H = canvas.height;

    // Background
    ctx.fillStyle = "#060d1f";
    ctx.fillRect(0, 0, W, H);

    // Vertical grid (frequency)
    for (const freq of GRID_FREQS) {
      const x = Math.round(freqToX(freq, W)) + 0.5;
      ctx.strokeStyle = "#131e36";
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, H);
      ctx.stroke();
    }

    // Horizontal grid (dB)
    for (const db of GRID_DBS) {
      const y = Math.round(dbToY(db, H)) + 0.5;
      ctx.strokeStyle = db === 0 ? "#1e3155" : "#0e172b";
      ctx.lineWidth = db === 0 ? 1.5 : 1;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
      ctx.stroke();
    }

    // Axis labels
    ctx.font = "10px ui-monospace, monospace";

    ctx.fillStyle = "#334155";
    ctx.textAlign = "center";
    ctx.textBaseline = "bottom";
    for (const [freq, label] of LABEL_FREQS) {
      ctx.fillText(label, freqToX(freq, W), H - 2);
    }

    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (const db of [-18, -12, -6, 6, 12, 18]) {
      ctx.fillText((db > 0 ? "+" : "") + db, W - 4, dbToY(db, H));
    }

    // -----------------------------------------------------------------------
    // Frequency response curve
    // -----------------------------------------------------------------------
    if (frequencyResponse.length >= 2) {
      const zeroY = dbToY(0, H);

      // Filled area
      const gradient = ctx.createLinearGradient(0, 0, 0, H);
      gradient.addColorStop(0,   "rgba(99,102,241,0.35)");
      gradient.addColorStop(0.5, "rgba(99,102,241,0.08)");
      gradient.addColorStop(1,   "rgba(99,102,241,0.0)");

      ctx.beginPath();
      ctx.moveTo(freqToX(frequencyResponse[0][0], W), zeroY);
      for (const [f, db] of frequencyResponse) {
        ctx.lineTo(freqToX(f, W), dbToY(db, H));
      }
      const last = frequencyResponse[frequencyResponse.length - 1];
      ctx.lineTo(freqToX(last[0], W), zeroY);
      ctx.closePath();
      ctx.fillStyle = gradient;
      ctx.fill();

      // Curve line
      ctx.beginPath();
      ctx.strokeStyle = "#818cf8";
      ctx.lineWidth = 1.75;
      ctx.lineJoin = "round";
      let moved = false;
      for (const [f, db] of frequencyResponse) {
        const x = freqToX(f, W);
        const y = dbToY(db, H);
        if (!moved) { ctx.moveTo(x, y); moved = true; }
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    }

    // -----------------------------------------------------------------------
    // Band markers
    // -----------------------------------------------------------------------
    for (const band of bands) {
      if (!band.enabled) continue;
      const x = freqToX(band.frequency, W);
      const markerDb = HAS_GAIN_TYPES.has(band.filter_type) ? band.gain_db : 0;
      const y = dbToY(markerDb, H);

      ctx.beginPath();
      ctx.arc(x, y, 5, 0, Math.PI * 2);
      ctx.fillStyle = "#4338ca";
      ctx.fill();
      ctx.strokeStyle = "#a5b4fc";
      ctx.lineWidth = 1.5;
      ctx.stroke();
    }
  }, [frequencyResponse, bands]);

  return (
    <canvas
      ref={canvasRef}
      width={900}
      height={220}
      className="w-full block"
    />
  );
}
