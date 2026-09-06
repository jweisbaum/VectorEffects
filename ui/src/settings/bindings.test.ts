import { describe, expect, it } from "vitest";

import type { AppSettings } from "../generated/AppSettings";
import { actionFor, bindingFor, chordLabel, chordOf, toolChord } from "./bindings";

const settings: AppSettings = {
  shortcuts: [
    { action: "play_pause", tool: "", key: " ", shift: false, alt: false },
    { action: "step_back", tool: "", key: "arrowleft", shift: false, alt: false },
    { action: "nudge_left", tool: "", key: "arrowleft", shift: true, alt: false },
    { action: "pan_left", tool: "", key: "arrowleft", shift: false, alt: true },
    { action: "tool", tool: "brush", key: "p", shift: false, alt: false },
  ],
  autosave: "recovery",
  default_wind_scale_knots: 60,
  default_current_scale_knots: 6,
  macro_directory: "",
  projection: "equirectangular",
  auto_scale: false,
};

describe("chordOf", () => {
  it("reads a bare key and a shifted one as different chords", () => {
    const base = { metaKey: false, ctrlKey: false, altKey: false };
    expect(chordOf({ ...base, key: "ArrowLeft", shiftKey: false })).toBe("arrowleft");
    expect(chordOf({ ...base, key: "ArrowLeft", shiftKey: true })).toBe("shift+arrowleft");
  });

  it("leaves command and control combinations alone", () => {
    // Those belong to the application's own menu keys; a tool letter must not
    // fire on cmd-P.
    for (const mod of ["metaKey", "ctrlKey"] as const) {
      const event = { key: "p", shiftKey: false, metaKey: false, ctrlKey: false, altKey: false };
      expect(chordOf({ ...event, [mod]: true })).toBeNull();
    }
  });

  it("reads alt as a modifier of its own", () => {
    const event = { key: "p", shiftKey: false, metaKey: false, ctrlKey: false, altKey: true };
    expect(chordOf(event)).toBe("alt+p");
  });
});

describe("actionFor", () => {
  it("finds the action a chord performs", () => {
    expect(actionFor(settings, " ")).toEqual({ action: "play_pause", tool: "" });
    expect(actionFor(settings, "p")).toEqual({ action: "tool", tool: "brush" });
  });

  it("tells the timeline's arrow from the nudge's and the map's", () => {
    // The bare arrows step the playhead, shift nudges the selection and alt
    // pans the map (D67). One table, three chords, no ambiguity.
    expect(actionFor(settings, "arrowleft")).toEqual({ action: "step_back", tool: "" });
    expect(actionFor(settings, "shift+arrowleft")).toEqual({ action: "nudge_left", tool: "" });
    expect(actionFor(settings, "alt+arrowleft")).toEqual({ action: "pan_left", tool: "" });
  });

  it("spells alt before shift, as the backend does", () => {
    expect(
      chordOf({ key: "ArrowUp", shiftKey: true, altKey: true, metaKey: false, ctrlKey: false }),
    ).toBe("alt+shift+arrowup");
  });

  it("is nothing for an unbound chord", () => {
    expect(actionFor(settings, "z")).toBeNull();
  });
});

describe("chordLabel", () => {
  it("names the keys that have no printable form", () => {
    expect(chordLabel(bindingFor(settings, "play_pause"))).toBe("Space");
    expect(chordLabel(bindingFor(settings, "step_back"))).toBe("←");
    expect(chordLabel(bindingFor(settings, "nudge_left"))).toBe("Shift-←");
    expect(chordLabel(bindingFor(settings, "pan_left"))).toBe("Alt-←");
  });

  it("says so when nothing is bound", () => {
    expect(chordLabel(null)).toBe("unbound");
  });
});

describe("toolChord", () => {
  it("gives a palette button the key it actually has", () => {
    // The tooltip reads the table, so a rebound key shows up in it without
    // anything else changing (M15's acceptance).
    expect(toolChord(settings, "brush")).toBe("P");
    expect(toolChord(settings, "circle")).toBe("unbound");
  });
});
