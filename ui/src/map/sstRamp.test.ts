/**
 * The temperature legend's colours and numbers (spec.md 4.10, M93).
 *
 * The reference is the backend's ramp as the brief states it — seven sRGB
 * stops from −2 °C to 32 °C — and conversions worked by hand.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  SST_CODE_MAX_C,
  SST_CODE_MIN_C,
  SST_MAX_C,
  SST_MIN_C,
  sstRampOf,
  sstTileRange,
  unpackTemperature,
  formatTemperature,
  legendTemperature,
  sstColour,
  sstGradientStops,
} from "./sstRamp";

describe("the temperature ramp", () => {
  it("runs from −2 °C to 32 °C", () => {
    expect(SST_MIN_C).toBe(-2);
    expect(SST_MAX_C).toBe(32);
  });

  it("is the backend's colours at its ends and its middle", () => {
    expect(sstColour(-2)).toEqual([40, 26, 120]);
    expect(sstColour(32)).toEqual([165, 0, 38]);
    // Seven stops over 34 degrees are 34 / 6 apart; the fourth is at
    // −2 + 3 × 34 / 6 = 15 °C.
    expect(sstColour(15)).toEqual([153, 213, 148]);
  });

  /** Halfway between the first two stops is halfway between their colours. */
  it("blends between stops", () => {
    // The second stop is at −2 + 34 / 6 = 3.6667 °C; halfway to it is 0.8333.
    const [r, g, b] = sstColour(-2 + 34 / 12);
    expect(r).toBeCloseTo((40 + 33) / 2);
    expect(g).toBeCloseTo((26 + 102) / 2);
    expect(b).toBeCloseTo((120 + 172) / 2);
  });

  it("takes the end colour beyond either end", () => {
    expect(sstColour(-30)).toEqual([40, 26, 120]);
    expect(sstColour(40)).toEqual([165, 0, 38]);
  });

  it("gives the legend the stops evenly spaced", () => {
    expect(sstGradientStops()).toEqual([
      "rgb(40, 26, 120) 0.00%",
      "rgb(33, 102, 172) 16.67%",
      "rgb(67, 170, 196) 33.33%",
      "rgb(153, 213, 148) 50.00%",
      "rgb(254, 224, 139) 66.67%",
      "rgb(244, 109, 67) 83.33%",
      "rgb(165, 0, 38) 100.00%",
    ]);
  });
});

describe("a temperature shown", () => {
  /** 18.4 × 9 / 5 + 32 = 33.12 + 32 = 65.12. */
  it("reads to a tenth in either unit", () => {
    expect(formatTemperature(18.4, "celsius")).toBe("18.4 °C");
    expect(formatTemperature(18.4, "fahrenheit")).toBe("65.1 °F");
    expect(formatTemperature(0, "fahrenheit")).toBe("32.0 °F");
    expect(formatTemperature(-1.26, "celsius")).toBe("−1.3 °C");
    // Below zero but not by a tenth: no sign on a zero.
    expect(formatTemperature(-0.04, "celsius")).toBe("0.0 °C");
  });

  /** −2 °C is 28.4 °F and 32 °C is 89.6 °F: 28 and 90 in whole degrees. */
  it("puts whole degrees at the legend's ends", () => {
    expect(legendTemperature(SST_MIN_C, "celsius")).toBe("−2 °C");
    expect(legendTemperature(SST_MAX_C, "celsius")).toBe("32 °C");
    expect(legendTemperature(SST_MIN_C, "fahrenheit")).toBe("28 °F");
    expect(legendTemperature(SST_MAX_C, "fahrenheit")).toBe("90 °F");
  });
});

describe("a temperature tile", () => {
  /** Packs one pixel as `ve_app::sst::pack` does: low byte, high byte, 0, alpha. */
  function pixel(celsius: number | null): number[] {
    if (celsius === null) return [0, 0, 0, 0];
    const t = Math.min(Math.max((celsius - SST_CODE_MIN_C) / (SST_CODE_MAX_C - SST_CODE_MIN_C), 0), 1);
    const code = Math.round(t * 65535);
    return [code & 0xff, code >> 8, 0, 255];
  }

  /** The code range is the backend's, read from its own source. */
  it("carries the backend's code range", () => {
    const rust = readFileSync(fileURLToPath(new URL("../../../crates/ve-app/src/sst.rs", import.meta.url)), "utf8");
    expect(rust).toMatch(new RegExp(`CODE_MIN_C: f32 = ${SST_CODE_MIN_C}\\.0;`));
    expect(rust).toMatch(new RegExp(`CODE_MAX_C: f32 = ${SST_CODE_MAX_C}\\.0;`));
  });

  it("unpacks to within a step of what was packed", () => {
    const step = (SST_CODE_MAX_C - SST_CODE_MIN_C) / 65535;
    for (const c of [-1.8, 0, 14.37, 31.9]) {
      const [lo, hi] = pixel(c);
      expect(Math.abs(unpackTemperature(lo!, hi!) - c)).toBeLessThanOrEqual(step);
    }
  });

  /** Land is clear and is no part of the range. */
  it("reports the coldest and warmest water it holds", () => {
    const bytes = new Uint8Array([...pixel(12.5), ...pixel(null), ...pixel(18.25), ...pixel(15)]);
    const range = sstTileRange(bytes);
    expect(range?.[0]).toBeCloseTo(12.5, 3);
    expect(range?.[1]).toBeCloseTo(18.25, 3);
    expect(sstTileRange(new Uint8Array([...pixel(null), ...pixel(null)]))).toBeNull();
  });
});

describe("the temperature ramp in force", () => {
  it("is the fixed ramp with the auto scale off, or nothing seen", () => {
    expect(sstRampOf(null)).toEqual({ min: SST_MIN_C, max: SST_MAX_C, auto: false });
  });

  /** As the speed ramp does, it spans what is in view. */
  it("spans the temperatures in view with the auto scale on", () => {
    expect(sstRampOf({ min: 14, max: 18.5 })).toEqual({ min: 14, max: 18.5, auto: true });
  });

  /** One temperature everywhere would put the whole view in one colour. */
  it("never spans less than a degree", () => {
    expect(sstRampOf({ min: 20, max: 20.2 })).toEqual({ min: 20, max: 21, auto: true });
  });
});

describe("a legend end of a narrow ramp", () => {
  /** An auto-scaled ramp a degree or two wide would read 14 … 16 in whole degrees. */
  it("takes a decimal when the ramp spans under five degrees", () => {
    expect(legendTemperature(14.34, "celsius", 1.3)).toBe("14.3 °C");
    expect(legendTemperature(14.34, "celsius", 34)).toBe("14 °C");
    // 1.3 °C is 2.34 °F wide, still narrow in the unit shown.
    expect(legendTemperature(14.34, "fahrenheit", 1.3)).toBe("57.8 °F");
    expect(legendTemperature(-0.02, "celsius", 1)).toBe("0.0 °C");
  });
});
