import * as api from "./api";
import { t } from "./i18n";
import {
  ask,
  message,
  open as openDialog,
  save as saveDialog,
} from "./nativeDialog";
import { useStore } from "./store";
import type { AppData } from "./types";

// Session → Export / Import Sessions…: one `.edgeterm` file (plain JSON
// inside) holding the saved sessions with their groups and Sender tags.
// Help → Export / Import Settings…: the same container for the frontend's
// preferences alone, so a settings backup never carries hosts. Passwords and
// passphrases never leave `credentials.json`; the backend strips them on both
// sides, and the session import confirmation says so, since imported sessions
// will ask again.

/** Mirrors `APP_DATA_EXTENSION` in model.rs; the backend enforces it too. */
const DATA_FILE_EXTENSION = "edgeterm";
const FILE_FILTERS = [
  { name: "EdgeTerm data", extensions: [DATA_FILE_EXTENSION] },
];

const hasDataExtension = (path: string) =>
  path.toLowerCase().endsWith(`.${DATA_FILE_EXTENSION}`);

/**
 * Not every platform's save dialog appends the filter's extension to a typed
 * name; the export must carry it or the import side will refuse the file.
 */
const withDataExtension = (path: string) =>
  hasDataExtension(path) ? path : `${path}.${DATA_FILE_EXTENSION}`;

/** "2 sessions, 1 Sender command" — each part localized, "" dropped. */
const describeSessions = (counts: {
  profiles: number;
  senderCommands: number;
}) => {
  const count = (n: number, one: string, many: string) =>
    n === 1 ? t(one) : t(many, { count: n });
  return [
    count(counts.profiles, "1 session", "{count} sessions"),
    counts.senderCommands
      ? count(
          counts.senderCommands,
          "1 Sender command",
          "{count} Sender commands",
        )
      : null,
  ]
    .filter((part) => part !== null)
    .join(t(", "));
};

const basename = (path: string) => path.split(/[\\/]/).pop() || path;

const datestamp = (date: Date) =>
  [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");

/** The text of an `invoke` rejection, which arrives as a string from Rust. */
const describeError = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

/**
 * A failure the user has to see: an import or export that quietly did nothing
 * would leave them believing their sessions or settings moved.
 */
const reportFailure = async (title: string, text: string) => {
  await message(text, { title, kind: "error", buttons: "Ok" }).catch(
    () => undefined,
  );
};

export async function exportSessions(): Promise<void> {
  try {
    const now = new Date();
    const picked = await saveDialog({
      title: t("Export EdgeTerm Sessions"),
      defaultPath: `EdgeTerm-Sessions-${datestamp(now)}.${DATA_FILE_EXTENSION}`,
      filters: FILE_FILTERS,
    });
    if (!picked) return;
    await api.exportSessionData(withDataExtension(picked), now.toISOString());
  } catch (error) {
    await reportFailure(
      t("Export Failed"),
      t("Export failed: {error}", { error: describeError(error) }),
    );
  }
}

export async function importSessions(): Promise<void> {
  const store = useStore.getState();
  try {
    const picked = await openDialog({
      title: t("Import EdgeTerm Sessions"),
      multiple: false,
      directory: false,
      filters: FILE_FILTERS,
    });
    const path = typeof picked === "string" ? picked : null;
    if (!path) return;

    // Parse first so the confirmation can say exactly what is coming in.
    const data: AppData = await api.readAppData(path);
    if (
      data.profiles.length === 0 &&
      data.groups.length === 0 &&
      data.senderCommands.length === 0
    ) {
      throw new Error(t("This file contains no sessions."));
    }
    const summary = describeSessions({
      profiles: data.profiles.length,
      senderCommands: data.senderCommands.length,
    });
    const confirmed = await ask(
      t(
        "Import {summary} from “{path}”?\n\nEntries with the same id replace the local ones; nothing else is removed. Data files never contain session passwords, so imported sessions will ask for theirs again.",
        { summary, path: basename(path) },
      ),
      {
        title: t("Import Sessions"),
        kind: "info",
        okLabel: t("Import"),
        cancelLabel: t("Cancel"),
      },
    );
    if (!confirmed) return;

    await api.importAppData(data);
    await store.loadProfiles();
    store.bumpSenderLibrary();
  } catch (error) {
    await reportFailure(
      t("Import Failed"),
      t("Import failed: {error}", { error: describeError(error) }),
    );
  }
}

export async function exportSettings(): Promise<void> {
  const { exportSettings: collectSettings } = useStore.getState();
  try {
    const now = new Date();
    const picked = await saveDialog({
      title: t("Export EdgeTerm Settings"),
      defaultPath: `EdgeTerm-Settings-${datestamp(now)}.${DATA_FILE_EXTENSION}`,
      filters: FILE_FILTERS,
    });
    if (!picked) return;
    await api.exportSettingsData(
      withDataExtension(picked),
      collectSettings(),
      now.toISOString(),
    );
  } catch (error) {
    await reportFailure(
      t("Export Failed"),
      t("Export failed: {error}", { error: describeError(error) }),
    );
  }
}

export async function importSettings(): Promise<void> {
  const store = useStore.getState();
  try {
    const picked = await openDialog({
      title: t("Import EdgeTerm Settings"),
      multiple: false,
      directory: false,
      filters: FILE_FILTERS,
    });
    const path = typeof picked === "string" ? picked : null;
    if (!path) return;

    const data: AppData = await api.readAppData(path);
    const settings = data.settings;
    if (
      !settings ||
      typeof settings !== "object" ||
      Array.isArray(settings) ||
      Object.keys(settings).length === 0
    ) {
      throw new Error(t("This file contains no settings."));
    }
    const confirmed = await ask(
      t(
        "Import settings from “{path}”?\n\nSettings in the file replace the current ones; saved sessions are not affected.",
        { path: basename(path) },
      ),
      {
        title: t("Import Settings"),
        kind: "info",
        okLabel: t("Import"),
        cancelLabel: t("Cancel"),
      },
    );
    if (!confirmed) return;

    store.applySettings(data.settings);
  } catch (error) {
    await reportFailure(
      t("Import Failed"),
      t("Import failed: {error}", { error: describeError(error) }),
    );
  }
}

/**
 * Session → Import OpenSSH Config…: picks a client configuration (the file
 * dialog starts at ~/.ssh/config), parses it and opens the chooser; the
 * import itself happens in `SshConfigImportDialog`.
 */
export async function importSshConfig(): Promise<void> {
  const store = useStore.getState();
  try {
    const defaultPath = await api.defaultSshConfigPath();
    const picked = await openDialog({
      title: t("Import OpenSSH Config"),
      multiple: false,
      directory: false,
      defaultPath,
    });
    const path = typeof picked === "string" ? picked : null;
    if (!path) return;
    const preview = await api.readSshConfig(path);
    if (preview.entries.length > 0) store.setSshConfigImport(preview);
  } catch (error) {
    await reportFailure(
      t("Import Failed"),
      t("Import failed: {error}", { error: describeError(error) }),
    );
  }
}
