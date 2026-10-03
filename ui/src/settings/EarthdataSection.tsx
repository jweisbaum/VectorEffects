/**
 * Settings → NASA Earthdata (spec 4.10, M97): the token CMC's downloads are
 * made with.
 *
 * The backend never sends the token back — not to this dialog, not to an
 * agent — so this shows only whether one is set, and the field is for
 * pasting a new one. It is kept in a file of its own and sent to NASA's
 * PO.DAAC archive and nowhere else (`ve_app::earthdata`).
 */
import { useEffect, useRef, useState } from "react";

import { api } from "../ipc";
import { useT } from "../i18n";

export default function EarthdataSection({ onError }: { onError: (err: unknown) => void }) {
  const t = useT();
  const [isSet, setIsSet] = useState<boolean | null>(null);
  const [draft, setDraft] = useState("");

  // Asked once, on opening: the dialog hands a new `onError` every render,
  // and the answer changes only when this section saves.
  const reportRef = useRef(onError);
  reportRef.current = onError;
  useEffect(() => {
    void api.earthdataStatus().then(setIsSet).catch((err: unknown) => reportRef.current(err));
  }, []);

  const save = (token: string) => {
    void api
      .setEarthdataToken(token)
      .then((now) => {
        setIsSet(now);
        setDraft("");
      })
      .catch(onError);
  };

  return (
    <section data-feature="settings:earthdata">
      <h3>{t("NASA Earthdata")}</h3>
      <p className="muted">
        {t(
          "A NASA Earthdata token lets the near-real-time import download CMC sea-surface temperature. Generate one at urs.earthdata.nasa.gov. It is kept apart from the other settings, sent only to NASA's PO.DAAC archive, and never shown again.",
        )}
      </p>
      <p className="muted" data-feature="settings:earthdata-status">
        {isSet === null ? "" : isSet ? t("A token is set.") : t("No token is set.")}
      </p>
      <label className="settings-field" data-feature="settings:earthdata-token">
        {t("Token")}
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={draft}
          placeholder={t("Paste a token")}
          onChange={(e) => setDraft(e.target.value)}
        />
      </label>
      <div className="settings-actions">
        <button type="button" disabled={draft.trim() === ""} onClick={() => save(draft)}>
          {t("Save token")}
        </button>
        {isSet === true && (
          <button type="button" onClick={() => save("")}>
            {t("Remove token")}
          </button>
        )}
      </div>
    </section>
  );
}
