import { useEffect, useState } from "react";

import * as api from "../api";
import { useT } from "../i18n";
import { open as openDialog } from "../nativeDialog";
import { useStore } from "../store";
import { RECORDING_KINDS, type RecordingKind } from "../types";

const KIND_LABELS: Record<RecordingKind, string> = {
  local: "Shell",
  ssh: "SSH",
  telnet: "Telnet",
  serial: "Serial",
};

/**
 * One recording folder per session type. The Session menu opens this dialog
 * because the menu cannot show four long paths; recording itself stays on the
 * Session panel's button. An empty field means that type's default folder,
 * which the field shows as its placeholder.
 */
export function RecordingDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const recordingDirs = useStore((s) => s.recordingDirs);
  const setRecordingDir = useStore((s) => s.setRecordingDir);
  const [dirs, setDirs] = useState(() => ({ ...recordingDirs }));
  const [defaults, setDefaults] = useState<
    Partial<Record<RecordingKind, string>>
  >({});

  useEffect(() => {
    let live = true;
    void Promise.all(
      RECORDING_KINDS.map((kind) =>
        api
          .defaultRecordingDir(kind)
          .then((dir) => [kind, dir] as const)
          .catch(() => null),
      ),
    ).then((entries) => {
      if (!live) return;
      const resolved: Partial<Record<RecordingKind, string>> = {};
      for (const entry of entries) {
        if (entry) resolved[entry[0]] = entry[1];
      }
      setDefaults(resolved);
    });
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onClose]);

  const choose = async (kind: RecordingKind) => {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: t("Choose a folder"),
      defaultPath: dirs[kind].trim() || defaults[kind] || undefined,
    });
    if (typeof picked !== "string") return;
    setDirs((current) => ({ ...current, [kind]: picked }));
  };

  const apply = () => {
    for (const kind of RECORDING_KINDS) {
      if (dirs[kind] !== recordingDirs[kind]) {
        setRecordingDir(kind, dirs[kind]);
      }
    }
    onClose();
  };

  return (
    <div className="dialog-backdrop" onMouseDown={onClose}>
      <form
        className="dialog recording-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="recording-dialog-title"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          apply();
        }}
      >
        <div className="dialog-header" id="recording-dialog-title">
          {t("Recording folders")}
        </div>
        <div className="dialog-body">
          <p className="recording-dialog-hint">
            {t(
              "Each session type keeps its recordings in its own folder. An empty field uses the default shown inside it.",
            )}
          </p>
          {RECORDING_KINDS.map((kind) => (
            <div className="recording-folder-row" key={kind}>
              <span className="recording-folder-kind">
                {t(KIND_LABELS[kind])}
              </span>
              <input
                aria-label={`${t("Recording folder")}: ${t(KIND_LABELS[kind])}`}
                className="recording-folder-path"
                type="text"
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                value={dirs[kind]}
                placeholder={defaults[kind] ?? ""}
                title={dirs[kind] || defaults[kind] || undefined}
                onChange={(event) =>
                  setDirs((current) => ({
                    ...current,
                    [kind]: event.target.value,
                  }))
                }
              />
              <button
                type="button"
                className="btn"
                onClick={() => void choose(kind)}
              >
                {t("Choose a folder")}
              </button>
            </div>
          ))}
        </div>
        <div className="dialog-footer">
          <button type="button" className="btn" onClick={onClose}>
            {t("Cancel")}
          </button>
          <button type="submit" className="btn is-primary">
            {t("Apply")}
          </button>
        </div>
      </form>
    </div>
  );
}
