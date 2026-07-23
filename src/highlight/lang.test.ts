import { describe, expect, it } from "vitest";
import { detectLang } from "./lang";

describe("detectLang", () => {
  it("maps markdown extensions and extensionless doc names to markdown", () => {
    expect(detectLang("docs/README.md")).toBe("markdown");
    expect(detectLang("notes.markdown")).toBe("markdown");
    expect(detectLang("README")).toBe("markdown");
    expect(detectLang("CHANGELOG")).toBe("markdown");
    expect(detectLang("CONTRIBUTING")).toBe("markdown");
    expect(detectLang("CODE_OF_CONDUCT")).toBe("markdown");
    expect(detectLang("docs/SECURITY")).toBe("markdown");
  });

  it("leaves other extensionless names and unknown extensions plain", () => {
    expect(detectLang("LICENSE")).toBeNull();
    expect(detectLang("README.txt")).toBeNull();
    expect(detectLang("readme.rst")).toBeNull();
  });

  it("keeps the existing filename mappings", () => {
    expect(detectLang("Dockerfile")).toBe("dockerfile");
    expect(detectLang("Makefile")).toBe("make");
    expect(detectLang(".env.local")).toBe("dotenv");
  });
});
