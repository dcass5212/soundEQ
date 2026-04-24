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
5. Press **Start**.

Signal path: `Apps → CABLE Input → soundEQ loopback capture → EQ → Speakers`

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
- `src/` — React frontend: EQ curve canvas, band controls, profile manager, session panel

## Architecture
See CLAUDE.md for full technical details and build decisions.