import type { Profile, ProfileStore } from "../lib/api";

interface Props {
  store: ProfileStore | null;
  presets: Profile[];
  activeName: string;
  onSelect: (name: string) => void;
  onNew: () => void;
  onDelete: (name: string) => void;
  onLoadPreset: (preset: Profile) => void;
}

export function ProfilePanel({
  store,
  presets,
  activeName,
  onSelect,
  onNew,
  onDelete,
  onLoadPreset,
}: Props) {
  const profiles = store ? Object.values(store.profiles) : [];
  // Sort: default first, then alphabetical.
  const sorted = [...profiles].sort((a, b) => {
    if (a.name === store?.default_profile_name) return -1;
    if (b.name === store?.default_profile_name) return 1;
    return a.name.localeCompare(b.name);
  });

  return (
    <aside className="w-48 flex-shrink-0 flex flex-col bg-gray-900 border-r border-gray-800 overflow-hidden">
      {/* ------------------------------------------------------------------ */}
      {/* Profiles */}
      {/* ------------------------------------------------------------------ */}
      <div className="px-3 pt-3 pb-1">
        <span className="text-xs font-semibold text-gray-500 uppercase tracking-wider">
          Profiles
        </span>
      </div>

      <div className="flex-1 overflow-y-auto">
        {sorted.map((p) => (
          <div
            key={p.name}
            className={`group flex items-center px-3 py-1.5 cursor-pointer text-sm transition-colors ${
              p.name === activeName
                ? "bg-indigo-900/60 text-indigo-200"
                : "text-gray-300 hover:bg-gray-800"
            }`}
            onClick={() => onSelect(p.name)}
          >
            <span className="flex-1 truncate">{p.name}</span>
            {p.name === store?.default_profile_name && (
              <span className="text-xs text-gray-600 mr-1" title="Default profile">
                ★
              </span>
            )}
            {p.name !== store?.default_profile_name && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(p.name);
                }}
                title="Delete profile"
                className="opacity-0 group-hover:opacity-100 text-gray-600 hover:text-red-400 text-base leading-none px-0.5"
              >
                ×
              </button>
            )}
          </div>
        ))}
      </div>

      {/* New profile button */}
      <button
        onClick={onNew}
        className="mx-3 mb-2 mt-1 text-xs text-indigo-400 hover:text-indigo-300 text-left py-1 border border-dashed border-gray-700 rounded px-2 hover:border-indigo-600 transition-colors"
      >
        + New profile
      </button>

      {/* ------------------------------------------------------------------ */}
      {/* Built-in presets */}
      {/* ------------------------------------------------------------------ */}
      <div className="border-t border-gray-800 px-3 pt-2 pb-1">
        <span className="text-xs font-semibold text-gray-500 uppercase tracking-wider">
          Presets
        </span>
      </div>

      <div className="overflow-y-auto pb-2">
        {presets.map((p) => (
          <button
            key={p.name}
            onClick={() => onLoadPreset(p)}
            title={`Load "${p.name}" into current profile`}
            className="w-full text-left px-3 py-1.5 text-sm text-gray-400 hover:text-gray-200 hover:bg-gray-800 transition-colors truncate"
          >
            {p.name}
          </button>
        ))}
      </div>
    </aside>
  );
}
