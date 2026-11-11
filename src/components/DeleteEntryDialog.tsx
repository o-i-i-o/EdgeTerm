import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";

import { useT } from "../i18n";
import type { FileEntry } from "../types";

interface Props {
  /** Every entry to delete; folders go with everything inside them. */
  entries: FileEntry[];
  /** Where the entries live, shown next to their paths: "local" or the session name. */
  location: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/** A long selection is summarised rather than scrolled: the decision is the count. */
const LISTED_ENTRIES = 5;

const kindOf = (entry: FileEntry) =>
  entry.isSymlink ? "link" : entry.isDir ? "folder" : "file";

/**
 * Confirmation shown before the Filer deletes files or folders, whether the
 * request came from the toolbar, a row's context menu, or a multi-entry
 * selection. Enter confirms and Esc cancels, so a stray click can no longer
 * remove something on a remote host without a second look.
 */
export function DeleteEntryDialog({
  entries,
  location,
  onConfirm,
  onCancel,
}: Props) {
  const t = useT();
  const confirmRef = useRef<HTMLButtonElement>(null);
  const single = entries.length === 1 ? entries[0] : null;
  const kind = single ? kindOf(single) : null;
  const hasFolder = entries.some((entry) => entry.isDir && !entry.isSymlink);

  useEffect(() => {
    // Pull focus out of whatever was active so keystrokes reach the dialog.
    confirmRef.current?.focus();
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Enter") {
        // A focused button activates natively: the confirm button on open, or
        // Cancel after tabbing, which Enter must not override.
        if (document.activeElement instanceof HTMLButtonElement) return;
        event.preventDefault();
        event.stopPropagation();
        onConfirm();
      } else if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onCancel();
      }
    };
    // Capture phase so the dialog answers before any global shortcut handler.
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onCancel, onConfirm]);

  if (entries.length === 0) return null;

  const title = single
    ? kind === "folder"
      ? t("Delete Folder")
      : kind === "link"
        ? t("Delete Link")
        : t("Delete File")
    : t("Delete {count} Items", { count: entries.length });
  // The backends delete a folder with everything inside it, so the hint says
  // so before the click rather than leaving the error to explain afterwards.
  const hint = single
    ? kind === "folder"
      ? t(
          "The folder and everything inside it will be removed. This cannot be undone.",
        )
      : kind === "link"
        ? t(
            "Only the link itself is removed; what it points to is left alone. This cannot be undone.",
          )
        : t("The file will be removed. This cannot be undone.")
    : hasFolder
      ? t(
          "The selected items will be removed, including everything inside their folders. This cannot be undone.",
        )
      : t("The selected files will be removed. This cannot be undone.");
  const listed = entries.slice(0, LISTED_ENTRIES);

  // The Filer lives inside a dock, and a dock with a terminal background uses
  // `backdrop-filter`, which makes it the containing block for fixed children.
  // Rendered in place, `position: fixed` would centre the dialog in the panel
  // instead of the window, clipping its buttons behind the panel's overflow.
  return createPortal(
    <div className="dialog-backdrop" onMouseDown={onCancel}>
      <div
        className="dialog confirm-dialog"
        role="alertdialog"
        aria-labelledby="delete-entry-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="dialog-header" id="delete-entry-title">
          {title}
        </div>
        <div className="dialog-body confirm-dialog-body">
          {single ? (
            <>
              <span>
                {t("Delete")} {t(kindOf(single))} <strong>{single.name}</strong>
                ?
              </span>
              <span className="confirm-dialog-target">
                {location} · {single.path}
              </span>
            </>
          ) : (
            <>
              <span>
                {t("Delete {count} items?", { count: entries.length })}
              </span>
              {listed.map((entry) => (
                <span className="confirm-dialog-target" key={entry.path}>
                  {entry.name}
                </span>
              ))}
              {entries.length > listed.length && (
                <span className="confirm-dialog-target">
                  {t("… and {count} more", {
                    count: entries.length - listed.length,
                  })}
                </span>
              )}
            </>
          )}
          <span className="confirm-dialog-hint">{hint}</span>
        </div>
        <div className="dialog-footer confirm-dialog-footer">
          <span className="confirm-dialog-keys">
            <kbd>Enter</kbd> {t("delete")} · <kbd>Esc</kbd> {t("cancel")}
          </span>
          <button type="button" className="btn" onClick={onCancel}>
            {t("Cancel")}
          </button>
          <button
            ref={confirmRef}
            type="button"
            className="btn is-danger"
            onClick={onConfirm}
          >
            {t("Delete")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
