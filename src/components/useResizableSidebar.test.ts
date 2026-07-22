import { describe, expect, it } from "vitest";

import {
  DEFAULT_SIDEBAR_WIDTH,
  MIN_SIDEBAR_WIDTH,
  clampSidebarWidth,
  initialSidebarWidth,
  parseStoredWidth,
  safeSetPointerCapture,
} from "./useResizableSidebar";

describe("clampSidebarWidth", () => {
  it("passes through widths inside the range", () => {
    expect(clampSidebarWidth(288, 1440)).toBe(288);
    expect(clampSidebarWidth(MIN_SIDEBAR_WIDTH, 1440)).toBe(MIN_SIDEBAR_WIDTH);
    expect(clampSidebarWidth(720, 1440)).toBe(720);
  });

  it("clamps below the minimum up to 200", () => {
    expect(clampSidebarWidth(0, 1440)).toBe(MIN_SIDEBAR_WIDTH);
    expect(clampSidebarWidth(-50, 1440)).toBe(MIN_SIDEBAR_WIDTH);
    expect(clampSidebarWidth(199, 1440)).toBe(MIN_SIDEBAR_WIDTH);
  });

  it("clamps above half the window down to 50vw", () => {
    expect(clampSidebarWidth(9999, 1440)).toBe(720);
    expect(clampSidebarWidth(701, 1400)).toBe(700);
  });

  it("lets the minimum win when the window is narrower than 2× min", () => {
    // 50% of a 300px window is 150px — below the usable floor.
    expect(clampSidebarWidth(250, 300)).toBe(MIN_SIDEBAR_WIDTH);
  });
});

describe("parseStoredWidth", () => {
  it("parses plain numeric strings", () => {
    expect(parseStoredWidth("288")).toBe(288);
    expect(parseStoredWidth("350.5")).toBe(350.5);
  });

  it("rejects missing, empty, and junk values", () => {
    expect(parseStoredWidth(null)).toBeNull();
    expect(parseStoredWidth("")).toBeNull();
    expect(parseStoredWidth("  ")).toBeNull();
    expect(parseStoredWidth("wide")).toBeNull();
    expect(parseStoredWidth("NaN")).toBeNull();
    expect(parseStoredWidth("Infinity")).toBeNull();
  });

  it("rejects non-positive numbers", () => {
    expect(parseStoredWidth("0")).toBeNull();
    expect(parseStoredWidth("-120")).toBeNull();
  });
});

describe("safeSetPointerCapture", () => {
  it("captures and reports success", () => {
    const captured: number[] = [];
    const target = {
      setPointerCapture: (id: number) => {
        captured.push(id);
      },
    };
    expect(safeSetPointerCapture(target, 7)).toBe(true);
    expect(captured).toEqual([7]);
  });

  it("swallows capture failures so the drag can still start", () => {
    const target = {
      setPointerCapture: () => {
        throw new DOMException("InvalidPointerId");
      },
    };
    expect(safeSetPointerCapture(target, 7)).toBe(false);
  });
});

describe("initialSidebarWidth", () => {
  it("uses the default when nothing valid is stored", () => {
    expect(initialSidebarWidth(null, 1440)).toBe(DEFAULT_SIDEBAR_WIDTH);
    expect(initialSidebarWidth("junk", 1440)).toBe(DEFAULT_SIDEBAR_WIDTH);
  });

  it("restores a stored width, clamped to the current window", () => {
    expect(initialSidebarWidth("350", 1440)).toBe(350);
    // Stored on a wide monitor, restored on a narrow window: 50vw wins.
    expect(initialSidebarWidth("900", 1200)).toBe(600);
    expect(initialSidebarWidth("120", 1440)).toBe(MIN_SIDEBAR_WIDTH);
  });
});
