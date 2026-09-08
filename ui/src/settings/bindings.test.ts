import { describe, expect, it } from "vitest";

import type { AppSettings } from "../generated/AppSettings";
import { actionFor, bindingFor, chordLabel, chordOf, toolChord } from "./bindings";

const settings: AppSettings = {
  shortcuts: [
    { action: "play_pause", tool: "", key: " ", shift: false, alt: false, accel: false },
    { action: "step_back", tool: "", key: "arrowleft", shift: false, alt: false, accel: false },
    { action: "nudge_left", tool: "", key: "arrowleft", shift: true, alt: false, accel: false },
    { action: "pan_left", tool: "", key: "arrowleft", shift: false, alt: true, accel: false },
    { action: "tool", tool: "brush", key: "p", shift: false, alt: false, accel: false },
    { action: "deselect", tool: "", key: "d", shift: false, alt: false, accel: true },
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

  /**
   * The command key is a modifier of the chord, not a disqualifier (M47).
   * It was the latter until the deselect needed accel-D — but a chord
   * carrying it still spells differently from the bare one, so a tool letter
   * cannot fire on cmd-P and nothing bound before now answers to a menu key.
   */
  it("reads command and control as one modifier, spelled apart from the bare key", () => {
    for (const mod of ["metaKey", "ctrlKey"] as const) {
      const event = { key: "p", shiftKey: false, metaKey: false, ctrlKey: false, altKey: false };
      expect(chordOf({ ...event, [mod]: true })).toBe("accel+p");
      expect(chordOf({ ...event, [mod]: true })).not.toBe(chordOf(event));
    }
    // And a menu key finds no action, because nothing is bound to it.
    expect(actionFor(settings, "accel+p")).toBeNull();
  });

  /** Which is what makes the deselect reachable at all. */
  it("finds the deselect on the command key", () => {
    const event = { key: "d", shiftKey: false, metaKey: true, ctrlKey: false, altKey: false };
    const chord = chordOf(event);
    expect(chord).toBe("accel+d");
    expect(actionFor(settings, chord as string)?.action).toBe("deselect");
    // The bare letter is the fill tool's territory, not the deselect's.
    expect(actionFor(settings, "d")).toBeNull();
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
