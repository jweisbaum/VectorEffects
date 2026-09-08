/**
 * The frame token the map and the backend agree about (M40).
 *
 * Every one of these is about the *shape* of a string that addresses tiles.
 * Getting it wrong is silent: the tiles of one scene, correctly cached, under
 * an address that names another. The round trip is the assertion — a token
 * built here must read back as what it was built from.
 */
import { describe, expect, it } from "vitest";

import { beneathToken, frameToken, onlyToken, parseFrameToken } from "./frameToken";

describe("the frame token", () => {
  it("round trips the whole stack", () => {
    const token = frameToken(1788, 7);
    expect(token).toBe("1788/7");
    expect(parseFrameToken(token)).toEqual({ revision: 1788, step: 7, scope: { kind: "whole" } });
  });

  /**
   * The layer left out is what makes an erase take one layer rather than the
   * composite, so it has to survive the trip exactly.
   */
  it("round trips a stack with a layer left out", () => {
    const token = beneathToken(1788, 7, 42);
    expect(token).toBe("without/42/1788/7");
    expect(parseFrameToken(token)).toEqual({
      revision: 1788,
      step: 7,
      scope: { kind: "without", layer: 42 },
    });
  });

  /**
   * A clone reads its source from the edited layer alone (M45), which is a
   * different scene again.
   */
  it("round trips one layer by itself", () => {
    const token = onlyToken(1788, 7, 42);
    expect(token).toBe("only/42/1788/7");
    expect(parseFrameToken(token)).toEqual({
      revision: 1788,
      step: 7,
      scope: { kind: "only", layer: 42 },
    });
  });

  /** All three are different addresses, or one would serve another's tiles. */
  it("keeps the three shapes apart", () => {
    const three = [frameToken(1788, 7), beneathToken(1788, 7, 42), onlyToken(1788, 7, 42)];
    expect(new Set(three).size).toBe(3);
    expect(parseFrameToken(frameToken(1788, 7))?.scope).toEqual({ kind: "whole" });
  });

  /**
   * A revision is seeded from the clock, so it is far above 2^32 and is a
   * double by the time it reaches here. It still has to survive the token.
   */
  it("carries a clock-seeded revision", () => {
    const revision = 1788521341064;
    expect(parseFrameToken(frameToken(revision, 0))?.revision).toBe(revision);
    expect(parseFrameToken(beneathToken(revision, 3, 9))).toEqual({
      revision,
      step: 3,
      scope: { kind: "without", layer: 9 },
    });
  });

  /**
   * A token that does not parse must be refused rather than guessed at: as a
   * request for revision `NaN` the backend refuses it and the map asks again,
   * forever.
   */
  it("refuses anything that is not a token", () => {
    for (const bad of ["", "1788", "1788/7/2", "without/1788/7", "without/x/1788/7", "only/x/1/2", "a/b"]) {
      expect(parseFrameToken(bad), bad).toBeNull();
    }
  });
});
