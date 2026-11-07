import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  deleteBackgroundImage,
  listSystemFonts,
  saveBackgroundImage,
  type FontFamily,
} from "../api";
import {
  fontChoices,
  fontStack,
  installedFonts,
  symbolFallbacks,
} from "../fonts";
import { useT } from "../i18n";
import { open as openDialog } from "../nativeDialog";
import {
  BACKGROUND_OPACITY,
  BUFFER_FONT_SIZE,
  PANEL_FONT_SIZE,
  TERMINAL_SCROLLBACK,
  useStore,
  type BackgroundFit,
} from "../store";
import type { CursorStyle } from "../terminal";

/** Everything the dialog hands back when Apply is pressed. */
export interface DisplaySettings {
  panelFontSize: number;
  bufferFontSize: number;
  panelFontFamily: string;
  bufferFontFamily: string;
  terminalScrollback: number;
  cursorStyle: CursorStyle;
  cursorBlink: boolean;
  /** Saved file name of the backdrop image; empty means none. */
  backgroundImage: string;
  backgroundOpacity: number;
  backgroundFit: BackgroundFit;
}

const BACKGROUND_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

interface Props extends DisplaySettings {
  onApply: (settings: DisplaySettings) => void;
  onClose: () => void;
}

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

export function FontSizeDialog({
  panelFontSize,
  bufferFontSize,
  panelFontFamily,
  bufferFontFamily,
  terminalScrollback,
  cursorStyle,
  cursorBlink,
  backgroundImage,
  backgroundOpacity,
  backgroundFit,
  onApply,
  onClose,
}: Props) {
  const t = useT();
  const [panelSize, setPanelSize] = useState(panelFontSize);
  const [bufferSize, setBufferSize] = useState(bufferFontSize);
  const [panelFamily, setPanelFamily] = useState(panelFontFamily);
  const [bufferFamily, setBufferFamily] = useState(bufferFontFamily);
  const [scrollback, setScrollback] = useState(terminalScrollback);
  const [cursor, setCursor] = useState<CursorStyle>(cursorStyle);
  const [blink, setBlink] = useState(cursorBlink);
  const [image, setImage] = useState(backgroundImage);
  const [opacity, setOpacity] = useState(backgroundOpacity);
  const [fit, setFit] = useState<BackgroundFit>(backgroundFit);
  const [imageError, setImageError] = useState<string | null>(null);
  // The copy saved by the last pick. A copy made in a dialog that is then
  // cancelled has to be removed again; one that is applied is kept.
  const pickedImage = useRef<string | null>(null);

  // Probing the machine for installed families measures text on a canvas, so
  // it happens once when the dialog opens rather than on every keystroke.
  // The saved family is kept in its list even where it is not installed, so a
  // setting brought in from another machine stays visible.
  const [probedUi] = useState(() => installedFonts("ui", panelFontFamily));
  const [probedMono] = useState(() => installedFonts("mono", bufferFontFamily));
  // The backend reads the font directories meanwhile: every family it finds
  // joins the suggestions when it answers, and the probe stands on its own if
  // it never does (see fontChoices). The terminal's icon fallback takes the
  // fresh list too, so a Nerd Font installed since start-up is picked up
  // without a restart.
  const [systemFonts, setSystemFonts] = useState<FontFamily[]>([]);
  useEffect(() => {
    let live = true;
    listSystemFonts()
      .then((fonts) => {
        useStore.getState().setSymbolFontFamilies(symbolFallbacks(fonts));
        if (live) setSystemFonts(fonts);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  // Cancelling throws away the copy picked in this dialog; the image still in
  // use is never touched.
  const close = useCallback(() => {
    const picked = pickedImage.current;
    if (picked && picked !== backgroundImage) {
      void deleteBackgroundImage(picked).catch(() => undefined);
    }
    onClose();
  }, [backgroundImage, onClose]);

  const pickImage = async () => {
    const path = await openDialog({
      multiple: false,
      directory: false,
      title: t("Choose a background image"),
      filters: [{ name: t("Images"), extensions: BACKGROUND_EXTENSIONS }],
    });
    if (typeof path !== "string") return;
    try {
      const name = await saveBackgroundImage(path, backgroundImage || null);
      pickedImage.current = name;
      setImage(name);
      setImageError(null);
      // The image would stay hidden behind an opaque background otherwise.
      if (opacity >= BACKGROUND_OPACITY.default) {
        setOpacity(BACKGROUND_OPACITY.first);
      }
    } catch (error) {
      setImageError(error instanceof Error ? error.message : String(error));
    }
  };

  const uiFonts = useMemo(
    () => fontChoices("ui", probedUi, systemFonts),
    [probedUi, systemFonts],
  );
  const monoFonts = useMemo(
    () => fontChoices("mono", probedMono, systemFonts),
    [probedMono, systemFonts],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [close]);

  const updateNumber = (
    value: number,
    setValue: (value: number) => void,
    min: number,
    max: number,
  ) => {
    if (Number.isFinite(value)) setValue(clamp(Math.round(value), min, max));
  };

  return (
    <div className="dialog-backdrop" onMouseDown={close}>
      <form
        className="dialog font-size-dialog"
        onMouseDown={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          // Cleared in this dialog: the picked copy is unreferenced once the
          // image is empty, so it can go now (the previous one is removed by
          // the caller when it applies the change).
          if (!image && pickedImage.current) {
            void deleteBackgroundImage(pickedImage.current).catch(
              () => undefined,
            );
            pickedImage.current = null;
          }
          onApply({
            panelFontSize: panelSize,
            bufferFontSize: bufferSize,
            panelFontFamily: panelFamily.trim(),
            bufferFontFamily: bufferFamily.trim(),
            terminalScrollback: scrollback,
            cursorStyle: cursor,
            cursorBlink: blink,
            backgroundImage: image,
            backgroundOpacity: opacity,
            backgroundFit: fit,
          });
        }}
      >
        <div className="dialog-header">{t("Display Settings")}</div>
        <div className="dialog-body font-size-dialog-body">
          <label className="font-size-setting">
            <span>
              <strong>{t("Panels")}</strong>
              <small>{t("Filer, Session, and Sender panels")}</small>
            </span>
            <input
              type="range"
              min={PANEL_FONT_SIZE.min}
              max={PANEL_FONT_SIZE.max}
              value={panelSize}
              onChange={(event) => setPanelSize(event.target.valueAsNumber)}
            />
            <input
              aria-label={t("Panel font size")}
              className="font-size-number"
              type="number"
              min={PANEL_FONT_SIZE.min}
              max={PANEL_FONT_SIZE.max}
              value={panelSize}
              onChange={(event) =>
                updateNumber(
                  event.target.valueAsNumber,
                  setPanelSize,
                  PANEL_FONT_SIZE.min,
                  PANEL_FONT_SIZE.max,
                )
              }
            />
            <span className="font-size-unit">px</span>
          </label>

          <FontFamilyField
            id="panel-font-family"
            label={t("Interface font")}
            hint={t("Menus, tabs, panels and dialogs")}
            role="ui"
            value={panelFamily}
            fonts={uiFonts}
            onChange={setPanelFamily}
          />

          <label className="font-size-setting">
            <span>
              <strong>{t("Buffer")}</strong>
              <small>{t("Terminal output and its gutter")}</small>
            </span>
            <input
              type="range"
              min={BUFFER_FONT_SIZE.min}
              max={BUFFER_FONT_SIZE.max}
              value={bufferSize}
              onChange={(event) => setBufferSize(event.target.valueAsNumber)}
            />
            <input
              aria-label={t("Buffer font size")}
              className="font-size-number"
              type="number"
              min={BUFFER_FONT_SIZE.min}
              max={BUFFER_FONT_SIZE.max}
              value={bufferSize}
              onChange={(event) =>
                updateNumber(
                  event.target.valueAsNumber,
                  setBufferSize,
                  BUFFER_FONT_SIZE.min,
                  BUFFER_FONT_SIZE.max,
                )
              }
            />
            <span className="font-size-unit">px</span>
          </label>

          <FontFamilyField
            id="buffer-font-family"
            label={t("Buffer font")}
            hint={t("Terminal typeface; pick a monospaced one")}
            role="mono"
            value={bufferFamily}
            fonts={monoFonts}
            onChange={setBufferFamily}
          />

          {/* A div, not a label: the row holds two controls of its own. */}
          <div className="font-size-setting cursor-setting">
            <span>
              <strong>{t("Cursor")}</strong>
              <small>{t("Shape of the terminal cursor")}</small>
            </span>
            <select
              aria-label={t("Cursor style")}
              className="cursor-style-select"
              value={cursor}
              onChange={(event) => setCursor(event.target.value as CursorStyle)}
            >
              <option value="block">{t("Block")}</option>
              <option value="underline">{t("Underline")}</option>
              <option value="bar">{t("Bar")}</option>
            </select>
            <label className="cursor-blink">
              <input
                type="checkbox"
                checked={blink}
                onChange={(event) => setBlink(event.target.checked)}
              />
              {t("Blink")}
            </label>
          </div>

          <label className="font-size-setting scrollback-setting">
            <span>
              <strong>{t("Scrollback")}</strong>
              <small>{t("Terminal history retained per session")}</small>
            </span>
            <input
              aria-label={t("Terminal scrollback lines")}
              className="scrollback-number"
              type="number"
              min={TERMINAL_SCROLLBACK.min}
              max={TERMINAL_SCROLLBACK.max}
              step={1000}
              value={scrollback}
              onChange={(event) =>
                updateNumber(
                  event.target.valueAsNumber,
                  setScrollback,
                  TERMINAL_SCROLLBACK.min,
                  TERMINAL_SCROLLBACK.max,
                )
              }
            />
            <span className="font-size-unit">{t("lines")}</span>
          </label>

          <div className="font-size-setting background-setting">
            <span>
              <strong>{t("Background image")}</strong>
              <small>{t("A picture behind the terminal")}</small>
            </span>
            <div className="background-actions">
              <span
                className="background-name"
                title={image || undefined}
                aria-label={t("Background image file")}
              >
                {image ? t("Image selected") : t("None")}
              </span>
              <button
                type="button"
                className="btn"
                onClick={() => void pickImage()}
              >
                {t("Choose Image…")}
              </button>
              {image && (
                <button
                  type="button"
                  className="btn"
                  onClick={() => setImage("")}
                >
                  {t("Remove")}
                </button>
              )}
            </div>
          </div>

          <label className="font-size-setting">
            <span>
              <strong>{t("Background opacity")}</strong>
              <small>
                {t(
                  "100% hides the image; lower values show it through the terminal color",
                )}
              </small>
            </span>
            <input
              type="range"
              aria-label={t("Background opacity")}
              min={BACKGROUND_OPACITY.min}
              max={BACKGROUND_OPACITY.max}
              value={opacity}
              disabled={!image}
              onChange={(event) => setOpacity(event.target.valueAsNumber)}
            />
            <input
              aria-label={t("Background opacity percent")}
              className="font-size-number"
              type="number"
              min={BACKGROUND_OPACITY.min}
              max={BACKGROUND_OPACITY.max}
              value={opacity}
              disabled={!image}
              onChange={(event) =>
                updateNumber(
                  event.target.valueAsNumber,
                  setOpacity,
                  BACKGROUND_OPACITY.min,
                  BACKGROUND_OPACITY.max,
                )
              }
            />
            <span className="font-size-unit">%</span>
          </label>

          <div className="font-size-setting background-setting">
            <span>
              <strong>{t("Image fit")}</strong>
              <small>{t("How the picture fills the terminal area")}</small>
            </span>
            <select
              aria-label={t("Image fit")}
              className="cursor-style-select"
              value={fit}
              disabled={!image}
              onChange={(event) => setFit(event.target.value as BackgroundFit)}
            >
              <option value="cover">{t("Fill")}</option>
              <option value="contain">{t("Fit")}</option>
              <option value="repeat">{t("Tile")}</option>
            </select>
          </div>

          {imageError && <div className="dialog-error">{imageError}</div>}
        </div>
        <div className="dialog-footer font-size-dialog-footer">
          <button
            type="button"
            className="btn font-size-reset"
            onClick={() => {
              setPanelSize(PANEL_FONT_SIZE.default);
              setBufferSize(BUFFER_FONT_SIZE.default);
              setPanelFamily("");
              setBufferFamily("");
              setScrollback(TERMINAL_SCROLLBACK.default);
              setCursor("block");
              setBlink(true);
              setImage("");
              setOpacity(BACKGROUND_OPACITY.default);
              setFit("cover");
              setImageError(null);
            }}
          >
            {t("Reset Defaults")}
          </button>
          <button type="button" className="btn" onClick={close}>
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

/**
 * A family name with the installed fonts as suggestions. Deliberately a text
 * field rather than a closed list: `fonts.ts` can only offer the families it
 * knows to probe for, and a terminal user's face of choice is as likely to be
 * a private Nerd Font build as one of them. Empty means the platform default,
 * and the field shows its own value in the font it names.
 */
function FontFamilyField({
  id,
  label,
  hint,
  role,
  value,
  fonts,
  onChange,
}: {
  id: string;
  label: string;
  hint: string;
  role: "mono" | "ui";
  value: string;
  fonts: string[];
  onChange: (family: string) => void;
}) {
  const t = useT();
  const listId = `${id}-list`;
  const systemMonoFamily = useStore((s) => s.systemMonoFamily);
  return (
    <label className="font-size-setting">
      <span>
        <strong>{label}</strong>
        <small>{hint}</small>
      </span>
      <input
        aria-label={label}
        className="font-family-input"
        type="text"
        list={listId}
        spellCheck={false}
        autoComplete="off"
        placeholder={t("System default")}
        value={value}
        style={{ fontFamily: fontStack(role, value, [], systemMonoFamily) }}
        onChange={(event) => onChange(event.target.value)}
      />
      <datalist id={listId}>
        {fonts.map((font) => (
          <option key={font} value={font} />
        ))}
      </datalist>
    </label>
  );
}
