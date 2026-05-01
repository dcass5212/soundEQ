import { useEffect, useRef, useState } from "react";
import type { BandConfig } from "../lib/api";
import { bandColor, hexToRgba } from "../lib/colors";

// ---------------------------------------------------------------------------
// Coordinate math — forward and inverse transforms
// ---------------------------------------------------------------------------

const LOG_MIN = Math.log10(20);
const LOG_MAX = Math.log10(20_000);
const DB_RANGE = 24; // ±24 dB displayed

function freqToX(freq: number, w: number): number {
  const t = (Math.log10(Math.max(20, Math.min(20_000, freq))) - LOG_MIN) / (LOG_MAX - LOG_MIN);
  return t * w;
}

function dbToY(db: number, h: number): number {
  return h / 2 - (db / DB_RANGE) * (h / 2);
}

// Inverse of freqToX — maps a CSS x pixel back to Hz.
function xToFreq(x: number, w: number): number {
  const t = Math.max(0, Math.min(1, x / w));
  return Math.pow(10, LOG_MIN + t * (LOG_MAX - LOG_MIN));
}

// Inverse of dbToY — maps a CSS y pixel back to dB.
function yToDb(y: number, h: number): number {
  return ((h / 2 - y) / (h / 2)) * DB_RANGE;
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

// Radius (CSS px) within which a click counts as hitting a band dot.
const HIT_RADIUS = 15;

// ---------------------------------------------------------------------------
// Gradient builder
//
// Creates a horizontal CanvasGradient that places each enabled band's color
// at the x-position corresponding to its frequency. The leftmost band's color
// extends to the canvas left edge, and the rightmost extends to the right edge,
// so the gradient always covers the full width.
// ---------------------------------------------------------------------------

function buildBandGradient(
  ctx: CanvasRenderingContext2D,
  W: number,
  checkBands: BandConfig[],
  alpha: number,
): CanvasGradient {
  const grad = ctx.createLinearGradient(0, 0, W, 0);

  // Collect enabled bands sorted by frequency (left to right on the canvas).
  // Preserve the original band index so the palette fallback (bandColor(i, …))
  // stays consistent with the dot colors drawn below.
  const resolvedStops = checkBands
    .map((b, origIndex) => ({ b, origIndex }))
    .filter(({ b }) => b.enabled)
    .map(({ b, origIndex }) => ({
      // t is the normalised [0,1] position on the log-frequency axis.
      t: (Math.log10(Math.max(20, Math.min(20_000, b.frequency))) - LOG_MIN) / (LOG_MAX - LOG_MIN),
      rgba: hexToRgba(bandColor(origIndex, b.color), alpha),
    }))
    .sort((a, z) => a.t - z.t);

  if (resolvedStops.length === 0) {
    // No enabled bands — fall back to the first palette color.
    const fallback = hexToRgba(bandColor(0), alpha);
    grad.addColorStop(0, fallback);
    grad.addColorStop(1, fallback);
    return grad;
  }

  grad.addColorStop(0, resolvedStops[0].rgba);
  for (const s of resolvedStops) {
    grad.addColorStop(Math.max(0, Math.min(1, s.t)), s.rgba);
  }
  grad.addColorStop(1, resolvedStops[resolvedStops.length - 1].rgba);
  return grad;
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

function hitTestBand(
  x: number,
  y: number,
  checkBands: BandConfig[],
  W: number,
  H: number,
): number {
  // Iterate in reverse so the last-drawn (topmost) dot wins on overlap.
  for (let i = checkBands.length - 1; i >= 0; i--) {
    const band = checkBands[i];
    if (!band.enabled) continue;
    const bx = freqToX(band.frequency, W);
    const markerDb = HAS_GAIN_TYPES.has(band.filter_type) ? band.gain_db : 0;
    const by = dbToY(markerDb, H);
    if (Math.hypot(x - bx, y - by) <= HIT_RADIUS) return i;
  }
  return -1;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface Props {
  frequencyResponse: [number, number][];
  /** Bands used for the gradient fill (reflects solo/mute enabled state). */
  bands: BandConfig[];
  /** Full stored bands used for dot positions, colors, and hit testing.
   *  Defaults to `bands` when not provided (no solo/mute active). */
  allBands?: BandConfig[];
  bypassed: boolean;
  spectrumData: number[];
  /** Indices of bands silenced by solo/mute — their dots are drawn dimmed. */
  silenced?: ReadonlySet<number>;
  /** Fired once when a drag begins — used to push a single undo snapshot. */
  onBandDragStart: (index: number) => void;
  /** Called while dragging a band dot to propagate the updated freq/gain. */
  onBandChange: (index: number, updated: BandConfig) => void;
}

export function EqCanvas({ frequencyResponse, bands, allBands, bypassed, spectrumData, silenced, onBandDragStart, onBandChange }: Props) {
  // `dotBands` = what we draw dots for and hit-test against.
  // When solo/mute is active, `bands` has silenced bands disabled (for the
  // gradient) but `allBands` preserves them so their dots remain interactive.
  const dotBands = allBands ?? bands;
  const canvasRef = useRef<HTMLCanvasElement>(null);

  // Resizable canvas height — drag the bottom handle to adjust.
  const [canvasHeight, setCanvasHeight] = useState(260);

  function startHeightDrag(e: React.MouseEvent) {
    e.preventDefault();
    const startY      = e.clientY;
    const startHeight = canvasHeight;
    function onMove(ev: MouseEvent) {
      setCanvasHeight(Math.max(120, Math.min(600, startHeight + ev.clientY - startY)));
    }
    function onUp() {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    }
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  // During a band drag, this holds the optimistically-updated band list so the
  // dots follow the cursor immediately without waiting for the IPC round-trip.
  const dragBandsRef = useRef<BandConfig[] | null>(null);

  // Ref to the latest draw() so mouse handlers can trigger a repaint.
  const drawRef = useRef<(() => void) | null>(null);

  // Active band-drag state — index of the band being dragged.
  const dragRef = useRef<{ index: number } | null>(null);

  // -------------------------------------------------------------------------
  // Draw
  // -------------------------------------------------------------------------

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    function draw() {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const dpr  = window.devicePixelRatio || 1;
      const cssW = canvas.clientWidth;
      const cssH = canvas.clientHeight;

      if (canvas.width !== Math.round(cssW * dpr) || canvas.height !== Math.round(cssH * dpr)) {
        canvas.width  = Math.round(cssW * dpr);
        canvas.height = Math.round(cssH * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const W = cssW;
      const H = cssH;

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

      ctx.textAlign = "left";
      ctx.textBaseline = "middle";
      for (const db of [-18, -12, -6, 12, 18]) {
        ctx.fillText((db > 0 ? "+" : "") + db, 4, dbToY(db, H));
      }

      // -----------------------------------------------------------------------
      // Spectrum analyzer bars (behind the EQ curve)
      // -----------------------------------------------------------------------
      if (spectrumData.length > 0) {
        const numBands = spectrumData.length;
        const FLOOR_DB = -90;

        const specGrad = ctx.createLinearGradient(0, 0, 0, H);
        specGrad.addColorStop(0,   bypassed ? "rgba(45,212,191,0.18)" : "rgba(45,212,191,0.50)");
        specGrad.addColorStop(0.5, bypassed ? "rgba(45,212,191,0.08)" : "rgba(45,212,191,0.20)");
        specGrad.addColorStop(1,   "rgba(45,212,191,0.00)");
        ctx.fillStyle = specGrad;

        const logMin = Math.log10(20);
        const logMax = Math.log10(20_000);

        for (let b = 0; b < numBands; b++) {
          const tLo = b / numBands;
          const tHi = (b + 1) / numBands;
          const fLo = Math.pow(10, logMin + tLo * (logMax - logMin));
          const fHi = Math.pow(10, logMin + tHi * (logMax - logMin));
          const xLo = freqToX(fLo, W);
          const xHi = freqToX(fHi, W);
          const barW = Math.max(1, xHi - xLo - 1);

          const db = spectrumData[b];
          const barH = Math.max(0, Math.min(H, ((db - FLOOR_DB) / -FLOOR_DB) * H));
          ctx.fillRect(xLo, H - barH, barW, barH);
        }
      }

      // -----------------------------------------------------------------------
      // Frequency response curve
      //
      // Fill: clip to the curve area, paint a horizontal band-color gradient,
      // then overlay a vertical background-color fade so the bottom fades out.
      // This simulates a 2D gradient (horizontal hue + vertical transparency)
      // using only the 1D gradient that the Canvas API natively supports.
      //
      // Line: same horizontal gradient at full (or dimmed-bypass) opacity.
      // -----------------------------------------------------------------------
      if (frequencyResponse.length >= 2) {
        const zeroY = dbToY(0, H);
        const last  = frequencyResponse[frequencyResponse.length - 1];

        // Build the closed fill path (curve + return to 0 dB baseline).
        function buildCurvePath() {
          ctx!.beginPath();
          ctx!.moveTo(freqToX(frequencyResponse[0][0], W), zeroY);
          for (const [f, db] of frequencyResponse) {
            ctx!.lineTo(freqToX(f, W), dbToY(db, H));
          }
          ctx!.lineTo(freqToX(last[0], W), zeroY);
          ctx!.closePath();
        }

        const fillAlpha = bypassed ? 0.12 : 0.55;

        // --- Fill ----------------------------------------------------------
        ctx.save();
        buildCurvePath();
        ctx.clip(); // Restrict both fills to the curve area.

        // 1. Horizontal band-color gradient (gives each region its hue).
        ctx.fillStyle = buildBandGradient(ctx, W, bands, fillAlpha);
        ctx.fillRect(0, 0, W, H);

        // 2. Vertical fade: overlay the background color, opaque at the bottom,
        //    transparent at the top. The result looks like the fill fades away.
        const vertFade = ctx.createLinearGradient(0, 0, 0, H);
        vertFade.addColorStop(0,   "rgba(6,13,31,0)");
        vertFade.addColorStop(0.55, "rgba(6,13,31,0.45)");
        vertFade.addColorStop(1,   "rgba(6,13,31,0.88)");
        ctx.fillStyle = vertFade;
        ctx.fillRect(0, 0, W, H);

        ctx.restore();

        // --- Line stroke ---------------------------------------------------
        const lineAlpha = bypassed ? 0.3 : 1.0;
        ctx.beginPath();
        ctx.strokeStyle = buildBandGradient(ctx, W, bands, lineAlpha);
        ctx.lineWidth = 2.25;
        ctx.lineJoin = "round";
        let moved = false;
        for (const [f, db] of frequencyResponse) {
          const x = freqToX(f, W);
          const y = dbToY(db, H);
          if (!moved) { ctx.moveTo(x, y); moved = true; }
          else ctx.lineTo(x, y);
        }
        ctx.stroke();

        if (bypassed) {
          ctx.beginPath();
          ctx.strokeStyle = "rgba(251,191,36,0.8)";
          ctx.lineWidth = 1.5;
          ctx.setLineDash([6, 4]);
          ctx.moveTo(0, zeroY);
          ctx.lineTo(W, zeroY);
          ctx.stroke();
          ctx.setLineDash([]);

          ctx.font = "bold 11px ui-monospace, monospace";
          ctx.fillStyle = "rgba(251,191,36,0.75)";
          ctx.textAlign = "right";
          ctx.textBaseline = "top";
          ctx.fillText("BYPASSED", W - 8, 8);
        }
      }

      // -----------------------------------------------------------------------
      // Band marker dots
      //
      // Uses `dotBands` (the stored profile) so dots stay visible and
      // interactive even when a band is silenced by solo/mute.
      //
      // Opacity tiers (highest to lowest priority):
      //   bypassed              → 30% fill, 15% ring
      //   muted/silenced        → 20% fill, 12% ring (noticeably dimmed)
      //   active (being dragged) → 100% fill, 90% ring
      //   normal                → 85% fill, 45% ring
      //
      // dragBandsRef is used during a drag so the dot follows the cursor
      // immediately — no waiting for the IPC round-trip.
      // -----------------------------------------------------------------------
      const renderBands = dragBandsRef.current ?? dotBands;
      for (let i = 0; i < renderBands.length; i++) {
        const band = renderBands[i];
        if (!band.enabled) continue; // band is toggled off in the stored profile
        const x = freqToX(band.frequency, W);
        const markerDb = HAS_GAIN_TYPES.has(band.filter_type) ? band.gain_db : 0;
        const y = dbToY(markerDb, H);

        const isActive    = dragRef.current?.index === i;
        const isSilenced  = silenced?.has(i) ?? false;
        const color       = bandColor(i, band.color);

        const fillAlpha = bypassed   ? 0.30
                        : isSilenced ? 0.20
                        : isActive   ? 1.00 : 0.85;
        const ringAlpha = bypassed   ? 0.15
                        : isSilenced ? 0.12
                        : isActive   ? 0.90 : 0.45;

        ctx.beginPath();
        ctx.arc(x, y, isActive ? 9 : 7, 0, Math.PI * 2);
        ctx.fillStyle   = hexToRgba(color, fillAlpha);
        ctx.strokeStyle = `rgba(255,255,255,${ringAlpha})`;
        ctx.lineWidth   = isActive ? 2.5 : 2;
        ctx.fill();
        ctx.stroke();
      }
    }

    drawRef.current = draw;
    draw();

    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => {
      drawRef.current = null;
      observer.disconnect();
    };
  }, [frequencyResponse, bands, allBands, bypassed, spectrumData, silenced]);

  // -------------------------------------------------------------------------
  // Drag interaction
  // -------------------------------------------------------------------------

  function handleMouseDown(e: React.MouseEvent<HTMLCanvasElement>) {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const W = canvas.clientWidth;
    const H = canvas.clientHeight;

    const idx = hitTestBand(x, y, dragBandsRef.current ?? dotBands, W, H);
    if (idx === -1) return;

    e.preventDefault();
    // Snapshot the current bands for undo BEFORE any frame updates fire.
    onBandDragStart(idx);
    dragRef.current  = { index: idx };
    dragBandsRef.current = [...dotBands];
    canvas.style.cursor = "grabbing";

    // requestAnimationFrame throttle — fire onBandChange at most once per frame
    // so we don't flood the IPC channel with dozens of calls per second.
    let rafId: number | null = null;
    let pending: { index: number; band: BandConfig } | null = null;

    function flushPending() {
      rafId = null;
      if (pending) {
        onBandChange(pending.index, pending.band);
        pending = null;
      }
    }

    function onMove(ev: MouseEvent) {
      const canvas = canvasRef.current;
      if (!canvas || !dragRef.current) return;

      const rect = canvas.getBoundingClientRect();
      const mx = ev.clientX - rect.left;
      const my = ev.clientY - rect.top;
      const W = canvas.clientWidth;
      const H = canvas.clientHeight;

      const dBands = dragBandsRef.current!;
      const band   = dBands[dragRef.current.index];

      // Horizontal → frequency, clamped to [20, 20000] Hz, rounded to integer.
      const newFreq = Math.max(20, Math.min(20_000, Math.round(xToFreq(mx, W))));

      // Vertical → gain, only for filter types that have a gain parameter.
      // Clamped to ±DB_RANGE, rounded to 0.1 dB.
      let newDb = band.gain_db;
      if (HAS_GAIN_TYPES.has(band.filter_type)) {
        newDb = Math.max(-DB_RANGE, Math.min(DB_RANGE,
          Math.round(yToDb(my, H) * 10) / 10));
      }

      const updated = { ...band, frequency: newFreq, gain_db: newDb };

      // Update local optimistic copy and redraw immediately.
      dragBandsRef.current = dBands.map((b, i) =>
        i === dragRef.current!.index ? updated : b,
      );
      drawRef.current?.();

      // Schedule the backend call for the next animation frame.
      pending = { index: dragRef.current.index, band: updated };
      if (!rafId) rafId = requestAnimationFrame(flushPending);
    }

    function onUp() {
      // Cancel any pending frame and fire the final position immediately.
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        flushPending();
      }
      dragRef.current = null;
      dragBandsRef.current = null;
      if (canvasRef.current) canvasRef.current.style.cursor = "default";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    }

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  function handleMouseMove(e: React.MouseEvent<HTMLCanvasElement>) {
    // During a drag the cursor is already set to "grabbing" — don't override it.
    if (dragRef.current) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const idx = hitTestBand(
      x, y, dragBandsRef.current ?? dotBands, canvas.clientWidth, canvas.clientHeight,
    );
    canvas.style.cursor = idx !== -1 ? "grab" : "default";
  }

  function handleMouseLeave() {
    if (!dragRef.current) {
      const canvas = canvasRef.current;
      if (canvas) canvas.style.cursor = "default";
    }
  }

  return (
    <div className="relative">
      <canvas
        ref={canvasRef}
        className="w-full block"
        style={{ height: canvasHeight }}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
      />
      {/* Bottom drag handle — drag down to make the canvas taller */}
      <div
        onMouseDown={startHeightDrag}
        className="w-full h-2 cursor-row-resize hover:bg-indigo-500/30 transition-colors"
      />
    </div>
  );
}
