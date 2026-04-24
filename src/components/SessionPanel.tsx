import type { ProfileStore, SessionInfo } from "../lib/api";

interface Props {
  sessions: SessionInfo[];
  store: ProfileStore | null;
  onAssign: (processName: string, profileName: string) => void;
  onUnassign: (processName: string) => void;
}

export function SessionPanel({ sessions, store, onAssign, onUnassign }: Props) {
  // Filter to only sessions that have a process name (skip system sessions).
  const appSessions = sessions.filter((s) => s.process_name !== "");

  const profileNames = store ? Object.keys(store.profiles) : [];

  if (appSessions.length === 0) {
    return (
      <div className="h-10 flex-shrink-0 flex items-center px-4 bg-gray-900 border-t border-gray-800">
        <span className="text-xs text-gray-600">No active audio sessions</span>
      </div>
    );
  }

  return (
    <div className="flex-shrink-0 flex items-center gap-4 px-4 bg-gray-900 border-t border-gray-800 overflow-x-auto h-12 min-h-[3rem]">
      <span className="text-xs text-gray-500 flex-shrink-0">Sessions:</span>
      {appSessions.map((s) => {
        const assigned = store?.app_assignments[s.process_name];
        return (
          <div
            key={s.session_id || s.pid}
            className="flex items-center gap-1.5 flex-shrink-0"
          >
            <span className="text-xs text-gray-300">{s.process_name}</span>
            <select
              value={assigned ?? ""}
              onChange={(e) => {
                const val = e.target.value;
                if (val === "") onUnassign(s.process_name);
                else onAssign(s.process_name, val);
              }}
              className="bg-gray-700 text-gray-200 text-xs rounded px-1 py-0.5 border border-gray-600 cursor-pointer"
            >
              <option value="">Default</option>
              {profileNames.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </div>
        );
      })}
    </div>
  );
}
