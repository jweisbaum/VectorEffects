import { api } from "../ipc";
import { reportError } from "../hint";
import type { AppSettings } from "../generated/AppSettings";
import { LANGUAGES, language, setLanguage, useLanguage, useT } from "./index";

/**
 * The interface language, in the title bar and on the start page (spec.md
 * 5.7). The switch is shown at once and saved behind it; a save that fails
 * puts the previous language back, so what is on screen is what was saved.
 */
export default function LanguagePicker({ onSettings }: { onSettings?: ((settings: AppSettings) => void) | undefined }) {
  const t = useT();
  const current = useLanguage();
  return <select className="language-picker" data-feature="shell:language" aria-label={t("Language")}
    title={t("Interface language")} value={current}
    onChange={event => {
      const before = language();
      const next = event.target.value;
      setLanguage(next);
      api.setLanguage(next).then(settings => onSettings?.(settings)).catch(error => {
        setLanguage(before);
        reportError(t("The language could not be saved: {error}", { error: String(error) }));
      });
    }}>
    {LANGUAGES.map(({ id, name }) => <option key={id} value={id} lang={id}>{name}</option>)}
  </select>;
}
