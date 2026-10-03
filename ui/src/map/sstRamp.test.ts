/**
 * The temperature legend's colours and numbers (spec.md 4.10, M93).
 *
 * The reference is the backend's ramp as the brief states it — seven sRGB
 * stops from −2 °C to 32 °C — and conversions worked by hand.
 */
import { describe, expect, it } from "vitest";

import {
  SST_MAX_C,
  SST_MIN_C,
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
