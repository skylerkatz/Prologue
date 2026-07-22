import { describe, expect, it, vi } from "vitest";

// ipc.ts imports the Tauri invoke bridge at module level; stub it so the
// pure helpers are importable outside a webview.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { errorText } from "./ipc";

describe("errorText", () => {
  it("passes through the strings Rust rejects with", () => {
    expect(errorText("not a git repository")).toBe("not a git repository");
  });

  it("stringifies Error objects and other values", () => {
    expect(errorText(new Error("boom"))).toBe("Error: boom");
    expect(errorText(42)).toBe("42");
    expect(errorText(undefined)).toBe("undefined");
  });
});
