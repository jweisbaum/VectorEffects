// @vitest-environment happy-dom
import { afterEach, expect, it } from "vitest";
import { fold, setLanguage } from "./index";

afterEach(() => setLanguage("en"));

it("ignores case and the accents a person leaves off", () => {
  expect(fold("Échelle Größe")).toBe("echelle große");
  expect(fold("ＡＢＣ１２")).toBe("abc12");
});

it("ignores Arabic vowel signs but keeps the letters", () => {
  setLanguage("ar");
  expect(fold("كَتَبَ")).toBe(fold("كتب"));
  expect(fold("سرعة الرياح")).toBe("سرعة الرياح");
});

it("keeps the marks that make a different Japanese letter", () => {
  setLanguage("ja");
  expect(fold("が")).not.toBe(fold("か"));
  expect(fold("ﾌﾞﾗｼ")).toBe(fold("ブラシ"));
});

it("marks a right-to-left language on the page, and only that one", () => {
  setLanguage("ar");
  expect(document.documentElement.dataset.textDir).toBe("rtl");
  expect(document.documentElement.lang).toBe("ar");
  setLanguage("zh");
  expect(document.documentElement.dataset.textDir).toBe("ltr");
  expect(document.documentElement.lang).toBe("zh-Hans");
});

it("isolates each substituted value in a right-to-left language, and only there", async () => {
  const { t } = await import("./index");
  setLanguage("ar");
  expect(t("{found} of {total} pages", { found: "+0 h", total: 9 })).toContain("⁨+0 h⁩");
  setLanguage("fr");
  expect(t("{found} of {total} pages", { found: 2, total: 9 })).not.toContain("⁨");
});
