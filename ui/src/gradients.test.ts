/**
 * The gradient catalogue, and the copy of it the frontend keeps (M42).
 *
 * The colours live once, in `ve_core::colour`. What is checked here is that
 * nothing in the frontend has quietly grown a second opinion: the fallback
 * copy in `ramp.ts`, the ceiling the shader reserves, and what happens to a
 * name that is not in the table.
 */
import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { DEFAULT_GRADIENT_ID, stopsOf } from "./gradients";
import type { GradientView } from "./generated/GradientView";
import { RAMP_COLOURS, rampColour, rampStops } from "./map/ramp";
import { RAMP_MAX_STOPS } from "./map/shaders";

/** The catalogue as the backend serves it, for the cases below. */
const CATALOGUE: GradientView[] = [
  { id: "vector", label: "VectorEffects", note: "", stops: RAMP_COLOURS.map((c) => [...c]) },
  {
    id: "greyscale",
    label: "Greyscale",
    note: "",
    stops: [
      [0, 0, 0],
      [1, 1, 1],
    ],
  },
];

describe("choosing a gradient", () => {
  it("finds the one an identifier names", () => {
    expect(stopsOf(CATALOGUE, "greyscale")).toEqual([
      [0, 0, 0],
      [1, 1, 1],
    ]);
  });

  /**
   * A project written by a later version may name a gradient this build has
   * never heard of. It is drawn with the default rather than not drawn: the
   * map showing the wrong colours is recoverable, the map showing nothing is
   * not.
   */
  it("falls back to the default for a name it does not have", () => {
    expect(stopsOf(CATALOGUE, "from-a-later-version")).toEqual(
      stopsOf(CATALOGUE, DEFAULT_GRADIENT_ID),
    );
  });

  /** The first frames run before the catalogue has been fetched. */
  it("has a gradient to draw with before the catalogue arrives", () => {
    expect(stopsOf([], "viridis")).toEqual(RAMP_COLOURS);
    expect(stopsOf([], "vector")).toEqual(RAMP_COLOURS);
  });
});

/**
 * The real catalogue, read out of the Rust that defines it.
 *
 * The colours live once and the frontend keeps two things that have to agree
 * with them: a copy of the default gradient, for the frames before the fetch
 * lands, and the length of the uniform array the shader uploads into. Neither
 * can be checked against a fixture — a fixture would only agree with itself —
 * so the table is parsed from the source, the way `shaders.test.ts` lifts the
 * projection out of the GLSL.
 */
function rustGradients(): { id: string; stops: number[][] }[] {
  const source = readFileSync(
    new URL("../../crates/ve-core/src/colour.rs", import.meta.url),
    "utf8",
  );
  // One chunk per entry, so every triple after that chunk's `stops:` belongs
  // to it. Simpler than one expression over the whole table, and it fails
  // loudly — with no entries — rather than quietly matching across two.
  return source
    .split("Gradient {")
    .slice(1)
    .map((chunk) => {
      const id = /id:\s*"([^"]+)"/.exec(chunk)?.[1] ?? "";
      // `stops: &[` is an entry; `stops: &'static [[f32; 3]]` is the struct
      // that declares them, and parsing that would read "f32; 3" as a colour.
      const at = chunk.indexOf("stops: &[");
      if (id === "" || at < 0) return { id: "", stops: [] };
      // Past the opening bracket of `&[`, or the first match swallows it and
      // reads the opening of the first stop as a channel.
      const list = chunk.slice(at + "stops: &[".length);
      const stops = [...list.matchAll(/\[([^\]]+)\]/g)].map((stop) =>
        (stop[1] as string).split(",").map((n) => Number(n.trim())),
      );
      return { id, stops };
    })
    .filter((entry) => entry.id !== "");
}

describe("the gradients Rust defines", () => {
  const table = rustGradients();

  it("were found in the source at all", () => {
    expect(table.length).toBeGreaterThan(4);
    expect(table.map((g) => g.id)).toContain(DEFAULT_GRADIENT_ID);
    // Every channel parsed as a number: a stop read as NaN would make every
    // comparison below pass or fail for the wrong reason.
    for (const gradient of table) {
      for (const stop of gradient.stops) {
        expect(stop.length, gradient.id).toBe(3);
        for (const channel of stop) expect(Number.isFinite(channel), gradient.id).toBe(true);
      }
    }
  });

  /**
   * `RAMP_COLOURS` is a copy of the default entry, kept for the frames before
   * the catalogue arrives. A copy that drifted would make the map change
   * colour a moment after it opened.
   */
  it("include the default the frontend copies, unchanged", () => {
    const fallback = table.find((entry) => entry.id === DEFAULT_GRADIENT_ID);
    expect(fallback).toBeDefined();
    expect(fallback?.stops.length).toBe(RAMP_COLOURS.length);
    fallback?.stops.forEach((stop, at) => {
      stop.forEach((channel, c) => {
        expect(channel, `stop ${at} channel ${c}`).toBeCloseTo(
          (RAMP_COLOURS[at] as readonly number[])[c] as number,
          6,
        );
      });
    });
  });

  /**
   * A gradient is uploaded into a uniform array of fixed length, and stops
   * past the end are dropped without a word — the map would simply stop
   * partway up the scale. Every gradient offered has to fit.
   */
  it("all fit in the array the shader reserves", () => {
    for (const gradient of table) {
      expect(gradient.stops.length, gradient.id).toBeGreaterThanOrEqual(2);
      expect(gradient.stops.length, gradient.id).toBeLessThanOrEqual(RAMP_MAX_STOPS);
    }
  });
});

describe("sampling a chosen gradient", () => {
  /** The ends are the ends, whichever gradient is in hand. */
  it("returns the first and last stop at the ends", () => {
    const grey = stopsOf(CATALOGUE, "greyscale");
    expect(rampColour(0, grey)).toEqual([0, 0, 0]);
    expect(rampColour(1, grey)).toEqual([1, 1, 1]);
    expect(rampColour(0.5, grey)).toEqual([0.5, 0.5, 0.5]);
  });

  /** And the legend's CSS stops are the same colours the sampler returns. */
  it("agrees with the legend's gradient at every stop", () => {
    const grey = stopsOf(CATALOGUE, "greyscale");
    const stops = rampStops(grey);
    expect(stops).toEqual(["#000000", "#ffffff"]);
    stops.forEach((css, index) => {
      const [r, g, b] = rampColour(index / (stops.length - 1), grey);
      const hex = `#${[r, g, b].map((c) => Math.round(c * 255).toString(16).padStart(2, "0")).join("")}`;
      expect(hex).toBe(css);
    });
  });
});
