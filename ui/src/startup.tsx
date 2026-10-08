import { useEffect, useState, type ReactNode } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./ipc";
import { setLanguage } from "./i18n";
import { applyTheme } from "./settings/themes";
import type { AppSettings } from "./generated/AppSettings";

/** Reveal only after React has committed the screen in the saved palette. */
function RevealWindow() {
  useEffect(() => {
    if (isTauri()) void getCurrentWindow().show().catch(error => {
      void api.frontendLog("error", `show startup window: ${String(error)}`);
    });
  }, []);
  return null;
}

/** The native window starts hidden so its first frame uses saved preferences. */
export default function Startup({ children }: {
  children: (settings: AppSettings | null) => ReactNode;
}) {
  // Undefined means loading; null means IPC failed and defaults are in use.
  const [settings, setSettings] = useState<AppSettings | null | undefined>(undefined);
  useEffect(() => {
    let active = true;
    const ready = (saved: AppSettings | null) => {
      if (!active) return;
      applyTheme(saved?.theme, saved?.custom_theme);
      if (saved) setLanguage(saved.language);
      setSettings(saved);
    };
    void api.appSettings().then(ready).catch(() => ready(null));
    return () => { active = false; };
  }, []);
  if (settings === undefined) return null;
  return <>{children(settings)}<RevealWindow /></>;
}
