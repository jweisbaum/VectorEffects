import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { api } from "./ipc";
import "./styles.css";

// Frontend failures must reach the same log file as everything else, or the app
// just shows nothing and the reason is only in a console nobody is watching.
window.addEventListener("error", (event) => {
  void api.frontendLog("error", `${event.message} (${event.filename}:${event.lineno})`);
});
window.addEventListener("unhandledrejection", (event) => {
  void api.frontendLog("error", `unhandled rejection: ${String(event.reason)}`);
});

const container = document.getElementById("root");
if (container === null) throw new Error("missing #root element");

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
