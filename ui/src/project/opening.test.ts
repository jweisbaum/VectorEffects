/**
 * The loading page's store: what it shows, when it stays and when it goes.
 */
import { afterEach, describe, expect, it } from "vitest";

import type { DocumentTree } from "../generated/DocumentTree";
import {
  barFraction,
  barLabel,
  beginOpening,
  currentOpening,
  finishOpening,
  reportOpening,
  unreadLayers,
} from "./opening";

afterEach(() => finishOpening());

const report = (fraction: number, done = 0, total = 0, label = "wind.grib2") => ({
  label,
  done,
  total,
  fraction,
});

describe("an opening", () => {
  it("stays for the map's first frame once the command has answered", () => {
    const end = beginOpening("cyclone.veproj");
    expect(currentOpening()).toMatchObject({ title: "cyclone.veproj", stage: "reading" });
    end(true);
    expect(currentOpening()).toMatchObject({ title: "cyclone.veproj", stage: "drawing" });
    finishOpening();
    expect(currentOpening().title).toBeNull();
  });

  it("goes at once when the command is refused: there is no map to wait for", () => {
    beginOpening("cyclone.veproj")(false);
    expect(currentOpening().title).toBeNull();
  });

  it("drops a report that arrives with nothing opening, or after the reading", () => {
    reportOpening(report(0.5));
    expect(currentOpening().progress).toBeNull();
    const end = beginOpening("a.veproj");
    end(true);
    reportOpening(report(0.5));
    expect(currentOpening().progress).toBeNull();
  });

  it("is not ended by an earlier opening finishing late", () => {
    const first = beginOpening("first.veproj");
    beginOpening("second.veproj");
    first(false);
    expect(currentOpening()).toMatchObject({ title: "second.veproj", stage: "reading" });
  });
});

describe("the bar", () => {
  it("is never full while there is still a wait", () => {
    const end = beginOpening("a.veproj");
    expect(barFraction(currentOpening())).toBe(0);
    reportOpening(report(1));
    expect(barFraction(currentOpening())).toBeCloseTo(0.95);
    end(true);
    expect(barFraction(currentOpening())).toBeLessThan(1);
    expect(barFraction(currentOpening())).toBeGreaterThan(0.95);
  });

  it("says what is being read and how much of it, and counts nothing it cannot", () => {
    const end = beginOpening("a.veproj");
    expect(barLabel(currentOpening())).toBe("Reading");
    reportOpening(report(0.02, 0, 0, "a.veproj"));
    expect(barLabel(currentOpening())).toBe("a.veproj");
    reportOpening(report(0.4, 312, 1488, "routing_test"));
    expect(barLabel(currentOpening())).toBe(
      `routing_test · 312 of ${(1488).toLocaleString()}`,
    );
    end(true);
    expect(barLabel(currentOpening())).toBe("Drawing the map");
  });
});

describe("layers that could not be read", () => {
  const tree = (layers: { name: string; loaded: boolean | null }[]) =>
    ({
      layers: layers.map(({ name, loaded }) => ({
        name,
        grib: loaded === null ? null : { loaded },
      })),
    }) as unknown as DocumentTree;

  it("are named, and a painted layer is never one of them", () => {
    expect(unreadLayers(tree([{ name: "Paint", loaded: null }, { name: "Wind", loaded: true }]))).toBeNull();
    expect(unreadLayers(tree([{ name: "Wind (gfs.grib2)", loaded: false }]))).toContain(
      '"Wind (gfs.grib2)"',
    );
    const two = unreadLayers(
      tree([
        { name: "Wind", loaded: false },
        { name: "Currents", loaded: false },
        { name: "Paint", loaded: null },
      ]),
    );
    expect(two).toContain("2 layers (Wind, Currents)");
  });
});
