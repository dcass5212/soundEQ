// =============================================================================
// useBandHistory — per-band undo/redo for EQ band edits
//
// Each band has its own independent past/future stack. Editing band 2 does not
// affect band 1's history, and undoing band 2 does not touch band 1.
//
// Structural changes (add, delete, reorder, preset load) clear all stacks
// because array indices shift and the stored snapshots would be wrong.
//
// Usage pattern:
//   const hist = useBandHistory(bands);
//
//   // Before a single-band edit (field commit, canvas drag start):
//   hist.pushBand(index);
//
//   // Before a structural change (add, delete, reorder, preset load):
//   hist.clearAll();
//
//   // Per-band undo/redo (for the row buttons):
//   const prev = hist.undoBand(index, bands);   // null if nothing to undo
//   const next = hist.redoBand(index, bands);   // null if nothing to redo
//   if (prev) await applyBands(prev);
//
//   // Global Ctrl+Z — undoes the most recently changed band:
//   const prev = hist.undoLast(bands);
//   if (prev) await applyBands(prev);
//
//   // On profile switch (history is per-profile):
//   hist.clearAll();
// =============================================================================

import { useCallback, useRef, useState } from "react";
import type { BandConfig } from "./api";

const MAX_DEPTH = 50;

interface BandStack {
  past:   BandConfig[];
  future: BandConfig[];
}

type Stacks = Record<number, BandStack>;

function getStack(stacks: Stacks, index: number): BandStack {
  return stacks[index] ?? { past: [], future: [] };
}

export function useBandHistory(currentBands: BandConfig[]) {
  const latestRef = useRef<BandConfig[]>(currentBands);
  latestRef.current = currentBands;

  const [stacks, setStacks] = useState<Stacks>({});

  // Tracks the order bands were edited so Ctrl+Z knows which band to undo last.
  // Most recently edited band is at the end of the array.
  const editOrderRef = useRef<number[]>([]);

  // ── Write helpers ──────────────────────────────────────────────────────────

  // Snapshot bands[index] before an edit. Call immediately before applying the
  // change so the stored value is the state the user can return to.
  const pushBand = useCallback((index: number) => {
    const snapshot = latestRef.current[index];
    // Track recency for Ctrl+Z — move this index to the end of the order list.
    editOrderRef.current = [
      ...editOrderRef.current.filter(i => i !== index),
      index,
    ];
    setStacks(prev => {
      const s = getStack(prev, index);
      return {
        ...prev,
        [index]: {
          past:   [...s.past, snapshot].slice(-MAX_DEPTH),
          future: [],
        },
      };
    });
  }, []);

  // Wipe all stacks. Call before structural changes (add/delete/reorder/preset)
  // and on profile switch so stale per-band snapshots don't survive index shifts.
  const clearAll = useCallback(() => {
    editOrderRef.current = [];
    setStacks({});
  }, []);

  // ── Per-band undo/redo (for the row ↩↪ buttons) ───────────────────────────

  // Undoes the last change to band[index]. Returns the new full bands array to
  // apply, or null if there is nothing to undo for this band.
  const undoBand = useCallback((index: number): BandConfig[] | null => {
    const s = getStack(stacks, index);
    if (s.past.length === 0) return null;
    const snapshot = s.past[s.past.length - 1];
    const current  = latestRef.current;
    setStacks(prev => {
      const ps = getStack(prev, index);
      return {
        ...prev,
        [index]: {
          past:   ps.past.slice(0, -1),
          future: [...ps.future, current[index]],
        },
      };
    });
    return current.map((b, i) => (i === index ? snapshot : b));
  }, [stacks]);

  // Redoes the last undone change to band[index]. Returns the new full bands
  // array, or null if there is nothing to redo.
  const redoBand = useCallback((index: number): BandConfig[] | null => {
    const s = getStack(stacks, index);
    if (s.future.length === 0) return null;
    const snapshot = s.future[s.future.length - 1];
    const current  = latestRef.current;
    setStacks(prev => {
      const ps = getStack(prev, index);
      return {
        ...prev,
        [index]: {
          past:   [...ps.past, current[index]],
          future: ps.future.slice(0, -1),
        },
      };
    });
    return current.map((b, i) => (i === index ? snapshot : b));
  }, [stacks]);

  // ── Global Ctrl+Z / Ctrl+Y ─────────────────────────────────────────────────

  // Undoes the most recently changed band. Ctrl+Z entry point.
  const undoLast = useCallback((): BandConfig[] | null => {
    const order = editOrderRef.current;
    // Walk from most recent to oldest, find first band with undo history.
    for (let i = order.length - 1; i >= 0; i--) {
      const idx = order[i];
      const s = getStack(stacks, idx);
      if (s.past.length > 0) {
        return undoBand(idx);
      }
    }
    return null;
  }, [stacks, undoBand]);

  // Redoes the most recently undone change (the band that was last undone).
  const redoLast = useCallback((): BandConfig[] | null => {
    const order = editOrderRef.current;
    for (let i = order.length - 1; i >= 0; i--) {
      const idx = order[i];
      const s = getStack(stacks, idx);
      if (s.future.length > 0) {
        return redoBand(idx);
      }
    }
    return null;
  }, [stacks, redoBand]);

  // ── Derived flags for button disabled states ───────────────────────────────

  const canUndoBand  = useCallback((index: number) =>
    getStack(stacks, index).past.length > 0,
  [stacks]);

  const canRedoBand  = useCallback((index: number) =>
    getStack(stacks, index).future.length > 0,
  [stacks]);

  const canUndoAny = Object.values(stacks).some(s => s.past.length   > 0);
  const canRedoAny = Object.values(stacks).some(s => s.future.length > 0);

  return {
    pushBand,
    clearAll,
    undoBand,
    redoBand,
    undoLast,
    redoLast,
    canUndoBand,
    canRedoBand,
    canUndo: canUndoAny,
    canRedo: canRedoAny,
  };
}
