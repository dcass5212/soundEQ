# soundEQ

A system-wide parametric equalizer for Windows. Built with Tauri v2 + React
frontend and Rust backend. Captures all system audio via WASAPI loopback,
applies real-time DSP filtering, and routes processed audio back to your speakers.

## Prerequisites
- Windows 10/11
- Rust (via rustup.rs)
- Node.js 18+
- MSVC Build Tools (Desktop development with C++)
- Windows 11 SDK
- **VB-Audio Virtual Cable** — required for audio routing (see Setup below)

## Setup (one-time)

soundEQ cannot register itself as a Windows audio device from user-mode code.
Instead it uses a free virtual audio cable driver to intercept system audio:

1. Search for **VB-Audio Virtual Cable** and install the free driver.
2. Open **Windows Sound settings** and set **CABLE Input** as your default
   output device. This redirects all app audio into the virtual cable.
3. Launch soundEQ. The **Capture** dropdown will auto-select `CABLE Input`.
4. In the **Output** dropdown, select your real speakers or headphones.
5. Press **Start** (the engine starts automatically if devices are detected).

Signal path: `Apps → CABLE Input → soundEQ loopback capture → EQ → Speakers`

---

## Features

### EQ Bands

soundEQ supports up to **16 parametric EQ bands** per profile. Each band has:

- **Filter type** — Peak, Low Shelf, High Shelf, Low Pass, High Pass, Notch, or Bandpass
- **Frequency** — 20 Hz to 20 kHz
- **Gain** — ±24 dB (Peak and Shelf types only)
- **Q factor** — 0.1 to 20 (controls bandwidth / resonance)
- **Enable/disable pill toggle** — removes the band from the signal path without deleting it
- **Per-band color** — click the color swatch to open the OS color picker; color persists with the profile

**Adding and removing bands:**
- Click **+ Add Band** at the bottom of the list to add a new Peak band at 1 kHz.
- Hover any band row and click the **×** that appears on the right to delete it.
- Bands are processed in top-to-bottom order (cascaded biquad filters).

**Reordering bands:**
- Drag the **⠿ grip handle** on the left of any row to change its position.
- Use **Shift+↑ / Shift+↓** on a focused row to move it one position with the keyboard.

**Mute and Solo:**
- **M** (amber) — mutes the band in the live engine without touching the stored profile.
  Useful for A/B comparing a single band's effect.
- **S** (yellow) — solos this band, silencing all others. Click the active solo band again
  to unsolo. Soloed and muted bands fade out in the list and on the canvas.

Both are non-destructive — the stored profile is unchanged and state resets on profile switch.

---

### EQ Canvas and Spectrum Analyzer

The visualization area at the top shows the combined frequency response curve and the
live spectrum of the audio passing through.

- **Frequency response curve** — displays the combined gain-vs-frequency shape of all active
  bands, drawn as a gradient that flows between each band's color at its center frequency.
- **Spectrum analyzer** — real-time FFT display rendered behind the EQ curve. Teal bars show
  the magnitude of each frequency band in the audio currently playing.
  Updates at ~20 Hz; clears to silence when the engine stops.
- **Draggable band dots** — each band appears as a dot on the canvas at its (frequency, gain)
  position. Click and drag horizontally to adjust frequency, vertically to adjust gain.
  Filter types without gain (Low Pass, High Pass, Notch, Bandpass) only move horizontally.
- **Resizable canvas** — drag the handle at the bottom edge of the canvas to resize it
  vertically (120 px – 600 px).
- **Bypass indicator** — when EQ bypass is active the curve dims, a dashed 0 dB line appears,
  and a "BYPASSED" label is shown on the canvas.

---

### Undo / Redo

All band edits are undoable, up to 50 steps:

| Action | Shortcut |
|---|---|
| Undo | Ctrl+Z  or  ↩ button in header |
| Redo | Ctrl+Y  or  Ctrl+Shift+Z  or  ↪ button in header |

Undoable actions include: field edits (freq, gain, Q), filter-type changes, enable/disable
toggles, color changes, add, delete, reorder, and preset loads. Canvas dot drags count as
one undo step regardless of how many frames fire during the drag. History is cleared when
you switch or delete a profile.

---

### Keyboard Navigation for Band Rows

Click any non-interactive part of a band row (the index number, the left color bar, or
the row background) to give that row keyboard focus — a subtle indigo ring appears.

| Key | Action |
|---|---|
| ↑ / ↓ | Move focus to the row above or below |
| Shift+↑ / Shift+↓ | Reorder this band one position up or down |
| Enter | Enter edit mode — focuses the filter-type select |
| Delete or Backspace | Delete this band; focus moves to the next row |
| Escape | Exit edit mode — blurs the focused field, returns focus to the row |

The Tab key still moves through individual fields (filter type, frequency, gain, Q) as
normal and is not affected by row-level navigation.

---

### Profiles

**Profiles** are named EQ configurations saved to disk in `%APPDATA%\com.soundeq.app\`.
Every profile holds up to 16 band configurations plus any per-app routing assignments.
The sidebar on the left lists all profiles; the **Default** profile (marked **★**) is
always present and is used when no app-specific assignment matches.

| Action | How |
|---|---|
| Switch profile | Click its name in the sidebar — applies immediately if the engine is running |
| Create | Click **+ New** at the bottom of the sidebar |
| Rename | Hover a row → click **✎** → edit inline → Enter to confirm, Escape to cancel |
| Export | Hover a row → click **↓** — downloads the profile as `<name>.json` |
| Import | Click **↑ Import** at the bottom of the sidebar → pick a `.json` file exported from soundEQ. If the name is already taken a numeric suffix is added automatically (`Name (2)`, etc.). |
| Delete | Hover a row → click **×** (not available on the Default profile) |

The sidebar is resizable — drag its right edge. Text size stays constant until the panel
is dragged quite narrow, then scales down proportionally.

### Presets

**Presets** are factory-supplied, read-only EQ configurations built into the app.
They appear in the lower half of the sidebar and are always available — no files to
manage. Loading a preset copies its bands into the currently active profile (soundEQ
asks for confirmation if the profile already has bands).

| Preset | Character |
|---|---|
| **Flat** | No EQ — all bands at 0 dB. Useful for assigning to apps you want unprocessed. |
| **Bass Boost** | Warm low-end lift; suits music on speakers that lack bass extension. |
| **Treble Boost** | Adds air and presence above 6 kHz; brightens dull recordings. |
| **Vocal Clarity** | Presence bump around 3–5 kHz; slight low-mid cut for cleaner speech. |
| **Gaming** | Wide soundstage shaping; lifts highs for footstep detail, adds sub-bass punch. |
| **Classical** | Gentle high-shelf lift with a neutral low end; suits orchestral content. |
| **Lo-Fi** | Bandpass-style roll-off at both ends for a warm, vintage character. |

Presets are a starting point — load one, then tweak the bands to taste. The original
preset is never modified.

---

### Per-App Profile Routing

The **Apps** panel (bottom bar) lets you assign a different EQ profile to each application.
soundEQ watches which window has keyboard focus and automatically switches to the assigned
profile when you tab between apps — no manual action needed.

**Adding an app:**
1. Click **Manage** to expand the Apps panel.
2. Click **+** to open the add form. Currently playing apps appear as quick-add chips.
3. Type or select the process name, choose a profile, and confirm.

**Removing an app** reverts it to the global default profile.

**Enabling/disabling an app** — in the collapsed bar, click an app chip to toggle its
automatic profile switching on or off without removing the assignment. Disabled apps appear
dimmed. The currently focused app is highlighted in emerald with an animated dot.

**What per-app routing does and does not do:**

It switches the *active* EQ profile when a different assigned app gains focus. It does
**not** apply different EQ to multiple apps simultaneously — by the time audio reaches
soundEQ's loopback capture, Windows has already mixed all app streams into a single stereo
buffer. Applying separate EQ curves to Spotify and a browser at the same time is not
possible with this architecture. (That would require pre-mix interception via Windows Audio
Processing Objects — a fundamentally different approach.)

---

### System Tray

soundEQ minimizes to the system tray when you close the window. The engine keeps running
in the background.

**Tray context menu:**
- **Status line** (top, non-clickable) — shows current state:
  - `○  Stopped` — engine is not running
  - `⊘  Bypassed` — engine running, EQ bypassed
  - `●  Active — Profile Name` — engine running and EQ is applied
- **Open soundEQ** — brings the window to the foreground
- **Start with Windows** — toggles the Windows startup registry entry
- **Quit** — exits the process

The tooltip (hover the tray icon) also reflects the current state and active profile name.

Left-clicking the tray icon opens the window (same as Open soundEQ).

---

### Headphone Crossfeed

The **Crossfeed** control bar sits between the EQ canvas and the band list. It is a
per-profile setting — you can have crossfeed on for a "Headphones" profile and off for
a "Speakers" profile, and it switches automatically with per-app routing.

**What crossfeed does:** when listening through headphones, each ear hears only one
channel. Real speakers produce natural crosstalk — the left ear also hears the right
speaker with a slight delay and some high-frequency attenuation (the head blocks HF from
the opposite side). Crossfeed recreates this for headphones, making extreme left/right
panning feel less fatiguing and more natural.

| Setting | Blend | Character |
|---|---|---|
| **Off** | 0% | Pure headphone stereo — no crossfeed |
| **Mild** | 25% | Subtle, wide image — good starting point |
| **Moderate** | 40% | Balanced speaker-like sound |
| **Strong** | 55% | Pronounced speaker simulation, narrower stage |

The algorithm (Linkwitz-style) low-pass filters each channel at 700 Hz before blending it
into the opposite ear, and adds a ~0.3 ms inter-aural delay. Only audible through
headphones — has no meaningful effect on speakers.

---

### EQ Bypass

Click **⊘ Bypass** in the header to pass audio through completely unmodified while keeping
the engine running. The EQ curve dims on the canvas and the tray indicator updates to
`⊘ Bypassed`. Click again to re-enable the active profile. Bypass resets automatically
when the engine is stopped.

---

### Device Auto-Restart

If the audio device is unplugged or the WASAPI session is invalidated (e.g. switching
default output device while running), soundEQ detects the failure within 2 seconds and
attempts an automatic restart:

1. First tries the same device IDs as before.
2. If the render device is gone, falls back to the Windows system default output.

A teal notice appears on success. If auto-restart fails, the engine stops and an error
message explains what happened so you can restart manually.

---

## Development

```bash
npm install          # install frontend dependencies (first run only)
npm run tauri dev    # start Vite dev server + Tauri app together
npm run tauri build  # produce a release installer
```

The first `tauri dev` compile takes several minutes (cold Rust build).
Subsequent runs are fast due to incremental compilation.

## Project structure
- `eq-core/` — DSP library: biquad filters, filter chain, profiles, presets
- `eq-audio/` — WASAPI audio I/O: loopback capture, render, device enumeration, session detection
- `src-tauri/` — Tauri backend: audio engine, IPC commands, persistence, system tray, startup
- `src/` — React frontend: EQ curve canvas, band controls, profile manager, app panel

## Architecture
See CLAUDE.md for full technical details and build decisions.
