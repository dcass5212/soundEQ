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

## Project structure
- `eq-core/` — DSP library (biquad filters, filter chain)
- `eq-audio/` — WASAPI audio capture and routing (coming soon)
- `eq-profiles/` — Profile and preset management (coming soon)
- `eq-app/` — Tauri application (coming soon)

## Development
(fill in as we build)

## Architecture
See CLAUDE.md for full technical details.