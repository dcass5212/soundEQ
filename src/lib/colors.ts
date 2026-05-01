// =============================================================================
// colors.ts — Band color utilities
// =============================================================================

// The indigo that was used for the EQ curve before per-band colors existed.
// All new bands default to this color and it is used as the fallback for any
// band that has no stored color (e.g. profiles created before this feature).
export const DEFAULT_BAND_COLOR = "#6366f1";

// Palette available in the color picker's suggestions — not used as automatic
// defaults because cycling through colors on add causes the reorder bug:
// index-based fallbacks change when a band moves to a new position.
export const BAND_COLORS: readonly string[] = [
  "#6366f1", // indigo  ← same as DEFAULT_BAND_COLOR
  "#f59e0b", // amber
  "#10b981", // emerald
  "#ef4444", // red
  "#3b82f6", // blue
  "#a855f7", // purple
  "#f97316", // orange
  "#14b8a6", // teal
  "#ec4899", // pink
  "#84cc16", // lime
  "#06b6d4", // cyan
  "#8b5cf6", // violet
  "#22c55e", // green
  "#f43f5e", // rose
  "#eab308", // yellow
  "#0ea5e9", // sky
];

// Returns the band's stored color, or the default indigo when none is set.
// Deliberately NOT index-based so that reordering bands never changes their
// apparent color — only an explicit user action (the color picker) can do that.
export function bandColor(_i: number, override?: string): string {
  return override ?? DEFAULT_BAND_COLOR;
}

// Converts a hex color string ("#rrggbb") to "rgba(r,g,b,alpha)".
export function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r},${g},${b},${alpha})`;
}
