// Generates a 1024x1024 soundEQ app icon:
// Dark navy background + light teal "EQ" wordmark + amber audio wave cutting through it.
// Run with: node generate-icon.mjs
// Requires: npm install canvas

import { createCanvas, registerFont } from 'canvas';
import { writeFileSync } from 'fs';

// Bahnschrift — clean geometric condensed sans-serif, ships with all Windows 10/11 installs.
// Similar proportions to Roboto but naturally narrower, which fits "EQ" well at icon sizes.
registerFont('C:/Windows/Fonts/bahnschrift.ttf', { family: 'Bahnschrift' });

const SIZE = 1024;
const canvas = createCanvas(SIZE, SIZE);
const ctx = canvas.getContext('2d');

// ── Helpers ───────────────────────────────────────────────────────────────────

function roundedSquarePath() {
  const r = 180;
  ctx.beginPath();
  ctx.moveTo(r, 0);
  ctx.lineTo(SIZE - r, 0);
  ctx.quadraticCurveTo(SIZE, 0, SIZE, r);
  ctx.lineTo(SIZE, SIZE - r);
  ctx.quadraticCurveTo(SIZE, SIZE, SIZE - r, SIZE);
  ctx.lineTo(r, SIZE);
  ctx.quadraticCurveTo(0, SIZE, 0, SIZE - r);
  ctx.lineTo(0, r);
  ctx.quadraticCurveTo(0, 0, r, 0);
  ctx.closePath();
}

// Natural-feeling audio waveform: a fundamental + two harmonics at lower amplitude.
// Looks like a real signal — varied but not chaotic.
const WAVE_CY  = SIZE * 0.50;
const WAVE_AMP = SIZE * 0.13;

function waveAt(x) {
  const t = (x / SIZE) * Math.PI * 2 * 2;  // 2 full cycles: 2 positive + 2 negative peaks
  return WAVE_CY + Math.sin(t) * WAVE_AMP;
}

function strokeWave(lineWidth, color, alpha) {
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.strokeStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  ctx.beginPath();
  for (let x = 0; x <= SIZE; x += 2) {
    const y = waveAt(x);
    x === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  }
  ctx.stroke();
  ctx.restore();
}

// ── Background ────────────────────────────────────────────────────────────────
roundedSquarePath();
ctx.fillStyle = '#0f1628';
ctx.fill();

ctx.save();
roundedSquarePath();
ctx.clip();

// ── "EQ" — tall font scaled horizontally to fill ~80% of the icon both ways ──
// We measure the natural text width and compress it horizontally so the letters
// sit centered and balanced without clipping the sides.
const FONT_SIZE  = Math.round(SIZE * 0.82);   // drives vertical fill
const TARGET_W   = SIZE * 0.78;               // max horizontal footprint

ctx.font      = `bold ${FONT_SIZE}px "Bahnschrift"`;
ctx.textAlign = 'center';

// Use actualBoundingBox measurements to find the true visual centre of the glyphs.
// textBaseline 'middle' centres on the em-square midpoint, which sits above the
// visual centre for all-caps text. We measure then shift to the real centre.
ctx.textBaseline = 'alphabetic';
const metrics  = ctx.measureText('EQ');
const glyphH   = metrics.actualBoundingBoxAscent + metrics.actualBoundingBoxDescent;
const centreY  = SIZE / 2 + metrics.actualBoundingBoxAscent - glyphH / 2;

ctx.fillStyle = '#818cf8';
ctx.fillText('EQ', SIZE / 2, centreY);

// ── Audio wave ────────────────────────────────────────────────────────────────
// Step 1: punch a gap through the letters in the background colour so the wave
//         appears to physically cut through "EQ" rather than sit on top of it.
// Step 2: draw the teal wave inside that gap.
// The background centre colour matches the radial gradient's inner stop.
strokeWave(103, '#0f1628', 1.00);   // deadspace — erases letters along the wave path
strokeWave(42,  '#7eeedd', 0.18);   // subtle glow
strokeWave(42,  '#7eeedd', 1.00);   // solid teal line

// ── Vignette ──────────────────────────────────────────────────────────────────
const vignette = ctx.createRadialGradient(SIZE/2, SIZE/2, SIZE * 0.35, SIZE/2, SIZE/2, SIZE * 0.72);
vignette.addColorStop(0, 'rgba(0,0,0,0)');
vignette.addColorStop(1, 'rgba(0,0,0,0.40)');
roundedSquarePath();
ctx.fillStyle = vignette;
ctx.fill();

ctx.restore();

// ── Write output ──────────────────────────────────────────────────────────────
const buf = canvas.toBuffer('image/png');
writeFileSync('src-tauri/icons/app-icon-source.png', buf);
console.log('Icon written to src-tauri/icons/app-icon-source.png');
console.log('Now run: npx tauri icon src-tauri/icons/app-icon-source.png');
