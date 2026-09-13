import { describe, expect, it } from "vitest";
import { PlaybackClock } from "./clock";

describe("sustained playback timing", () => {
  for (const hz of [60, 120]) {
    for (const rate of [0.5, 8, 12, 23.976, 24, 30, 60]) {
      it(`keeps ${rate} steps/s on a ${hz} Hz display`, () => {
        const clock = new PlaybackClock(0, rate);
        let advances = 0;
        for (let frame = 1; frame <= hz * 60; frame++) {
          const now = frame * 1000 / hz;
          if (clock.due(now, rate)) {
            clock.advance(now);
            advances++;
          }
        }
        expect(Math.abs(advances - rate * 60)).toBeLessThanOrEqual(1);
      });
    }
  }

  it("carries display jitter without accumulating drift", () => {
    const clock = new PlaybackClock(0, 24);
    let advances = 0;
    for (let frame = 1; frame <= 3600; frame++) {
      const now = frame * 1000 / 60 + (frame % 3 - 1) * 2;
      if (clock.due(now, 24)) {
        clock.advance(now);
        advances++;
      }
    }
    expect(Math.abs(advances - 1440)).toBeLessThanOrEqual(1);
  });

  it("resumes a buffer or suspended window without a catch-up burst", () => {
    for (const buffering of [false, true]) {
      const clock = new PlaybackClock(0, 8);
      if (buffering) clock.hold();
      expect(clock.due(5000, 8)).toBe(true);
      clock.advance(5000);
      expect(clock.due(5017, 8)).toBe(false);
      expect(clock.due(5125, 8)).toBe(true);
    }
  });

  it("rebases even a short readiness stall and a live rate change", () => {
    const clock = new PlaybackClock(0, 8);
    expect(clock.due(130, 8)).toBe(true);
    clock.hold();
    clock.advance(150);
    expect(clock.due(250, 8)).toBe(false);
    expect(clock.due(275, 8)).toBe(true);
    expect(clock.due(280, 24)).toBe(false);
    expect(clock.due(322, 24)).toBe(true);
  });
});
