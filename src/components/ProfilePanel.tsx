import { useRef, useState } from "react";
import type { Profile, ProfileStore } from "../lib/api";

interface Props {
  store: ProfileStore | null;
  presets: Profile[];
  activeName: string;
  onSelect: (name: string) => void;
  onNew: () => void;
  onDelete: (name: string) => void;
  onRename: (oldName: string, newName: string) => void;
  onLoadPreset: (preset: Profile) => void;
  onExport: (name: string) => void;
  onImportFile: (json: string) => void;
}

export function ProfilePanel({
  store,
  presets,
  activeName,
  onSelect,
  onNew,
  onDelete,
  onRename,
  onLoadPreset,
  onExport,
  onImportFile,
}: Props) {
  const [editingName, setEditingName] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const importInputRef = useRef<HTMLInputElement>(null);

  function handleImportClick() {
    importInputRef.current?.click();
  }

  function handleFileChange(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (evt) => {
      const json = evt.target?.result;
      if (typeof json === "string") onImportFile(json);
    };
    reader.readAsText(file);
    // Reset so the same file can be re-imported if needed.
    e.target.value = "";
  }

  // Resizable width — drag the right edge to adjust.
  const [width, setWidth] = useState(192);
  // Text stays at a comfortable fixed size within the normal width range and
  // only starts shrinking when the panel is dragged narrower than 160 px.
  // Above 160 px it scales upward normally; below 160 px it linearly tapers
  // from 13 px (at 160 px wide) down to 10 px (at the 140 px minimum).
  const fontSize = (() => {
    if (width >= 160) return Math.min(20, Math.round(width * 0.073));
    return Math.max(10, Math.round(10 + (width - 140) * 3 / 20));
  })();

  function startDragWidth(e: React.MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = width;

    function onMove(ev: MouseEvent) {
      setWidth(Math.max(140, Math.min(450, startWidth + ev.clientX - startX)));
    }
    function onUp() {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    }
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }

  const profiles = store ? Object.values(store.profiles) : [];
  const sorted = [...profiles].sort((a, b) => {
    if (a.name === store?.default_profile_name) return -1;
    if (b.name === store?.default_profile_name) return 1;
    return a.name.localeCompare(b.name);
  });

  function startEditing(name: string) {
    setEditingName(name);
    setEditValue(name);
    setTimeout(() => inputRef.current?.select(), 0);
  }

  function commitRename() {
    if (!editingName) return;
    const trimmed = editValue.trim();
    if (trimmed && trimmed !== editingName) {
      onRename(editingName, trimmed);
    }
    setEditingName(null);
  }

  function cancelRename() {
    setEditingName(null);
  }

  return (
    // position:relative lets the drag handle sit on the right edge absolutely.
    <aside
      style={{ width }}
      className="relative flex-shrink-0 flex flex-col bg-gray-900 border-r border-gray-800 overflow-hidden"
    >
      {/* Right-edge drag handle */}
      <div
        onMouseDown={startDragWidth}
        className="absolute right-0 top-0 bottom-0 w-1.5 cursor-col-resize hover:bg-indigo-500/30 transition-colors z-10"
      />

      {/* ------------------------------------------------------------------ */}
      {/* Profiles                                                             */}
      {/* ------------------------------------------------------------------ */}
      <div className="px-3 pt-3 pb-1">
        <span
          style={{ fontSize }}
          className="font-semibold text-gray-500 uppercase tracking-wider"
        >
          Profiles
        </span>
      </div>

      <div className="flex-1 overflow-y-auto">
        {sorted.map((p) => (
          <div
            key={p.name}
            style={{ fontSize }}
            className={`group flex items-center px-3 py-1.5 cursor-pointer transition-colors ${
              p.name === activeName
                ? "bg-indigo-900/60 text-indigo-200"
                : "text-gray-300 hover:bg-gray-800"
            }`}
            onClick={() => { if (editingName !== p.name) onSelect(p.name); }}
          >
            {editingName === p.name ? (
              <input
                ref={inputRef}
                value={editValue}
                onChange={(e) => setEditValue(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename();
                  if (e.key === "Escape") cancelRename();
                }}
                onBlur={commitRename}
                onClick={(e) => e.stopPropagation()}
                style={{ fontSize }}
                className="flex-1 min-w-0 bg-gray-700 text-gray-100 rounded px-1 py-0 border border-indigo-500 outline-none"
              />
            ) : (
              <>
                <span className="flex-1 truncate">{p.name}</span>

                {p.name === store?.default_profile_name && (
                  <span
                    style={{ fontSize: Math.max(9, fontSize - 2) }}
                    className="text-gray-600 mr-1"
                    title="Default profile"
                  >
                    ★
                  </span>
                )}

                {/* Pencil — rename, visible on hover */}
                <button
                  onClick={(e) => { e.stopPropagation(); startEditing(p.name); }}
                  title="Rename profile"
                  style={{ fontSize: Math.max(12, fontSize) }}
                  className="opacity-0 group-hover:opacity-100 text-gray-600 hover:text-indigo-400 leading-none px-1 transition-opacity"
                >
                  ✎
                </button>

                {/* ↓ — export this profile as a JSON file */}
                <button
                  onClick={(e) => { e.stopPropagation(); onExport(p.name); }}
                  title="Export profile to file"
                  style={{ fontSize: Math.max(12, fontSize) }}
                  className="opacity-0 group-hover:opacity-100 text-gray-600 hover:text-teal-400 leading-none px-1 transition-opacity"
                >
                  ↓
                </button>

                {/* × — delete, only for non-default profiles */}
                {p.name !== store?.default_profile_name && (
                  <button
                    onClick={(e) => { e.stopPropagation(); onDelete(p.name); }}
                    title="Delete profile"
                    style={{ fontSize: Math.max(16, fontSize + 2) }}
                    className="opacity-0 group-hover:opacity-100 text-gray-600 hover:text-red-400 leading-none px-1 transition-opacity"
                  >
                    ×
                  </button>
                )}
              </>
            )}
          </div>
        ))}
      </div>

      {/* New / Import profile buttons */}
      <div className="mx-3 mb-2 mt-1 flex gap-1">
        <button
          onClick={onNew}
          style={{ fontSize: Math.max(9, fontSize - 2) }}
          className="flex-1 text-indigo-400 hover:text-indigo-300 text-left py-1 border border-dashed border-gray-700 rounded px-2 hover:border-indigo-600 transition-colors"
        >
          + New
        </button>
        <button
          onClick={handleImportClick}
          style={{ fontSize: Math.max(9, fontSize - 2) }}
          title="Import a profile from a .json file"
          className="flex-1 text-teal-500 hover:text-teal-300 text-left py-1 border border-dashed border-gray-700 rounded px-2 hover:border-teal-700 transition-colors"
        >
          ↑ Import
        </button>
      </div>

      {/* Hidden file input — triggered by the Import button above */}
      <input
        ref={importInputRef}
        type="file"
        accept=".json"
        className="hidden"
        onChange={handleFileChange}
      />

      {/* ------------------------------------------------------------------ */}
      {/* Built-in presets                                                     */}
      {/* ------------------------------------------------------------------ */}
      <div className="border-t border-gray-800 px-3 pt-2 pb-1">
        <span
          style={{ fontSize }}
          className="font-semibold text-gray-500 uppercase tracking-wider"
        >
          Presets
        </span>
      </div>

      <div className="overflow-y-auto pb-2">
        {presets.map((p) => (
          <button
            key={p.name}
            onClick={() => onLoadPreset(p)}
            title={`Load "${p.name}" into current profile`}
            style={{ fontSize }}
            className="w-full text-left px-3 py-1.5 text-gray-400 hover:text-gray-200 hover:bg-gray-800 transition-colors truncate"
          >
            {p.name}
          </button>
        ))}
      </div>
    </aside>
  );
}
