import { useCallback, useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";

/** localStorage key for the review sidebar width (app-wide preference). */
export const SIDEBAR_WIDTH_KEY = "prologue.sidebarWidth";

/** Default sidebar width, matching the original fixed layout. */
export const DEFAULT_SIDEBAR_WIDTH = 288;

/** Narrowest useful sidebar; file rows still truncate readably here. */
export const MIN_SIDEBAR_WIDTH = 200;

/**
 * Clamp a candidate width to [MIN_SIDEBAR_WIDTH, half the window]. When the
 * window is narrower than twice the minimum, the minimum wins — the sidebar
 * never collapses below a usable width.
 */
export function clampSidebarWidth(width: number, windowWidth: number): number {
  return Math.max(MIN_SIDEBAR_WIDTH, Math.min(width, windowWidth / 2));
}

/** Parse a persisted width; null for anything non-numeric or non-positive. */
export function parseStoredWidth(raw: string | null): number | null {
  if (raw === null || raw.trim() === "") return null;
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

/** Width to mount with: the stored value clamped, or the default. */
export function initialSidebarWidth(
  raw: string | null,
  windowWidth: number,
): number {
  const stored = parseStoredWidth(raw);
  return stored === null
    ? DEFAULT_SIDEBAR_WIDTH
    : clampSidebarWidth(stored, windowWidth);
}

/**
 * Capture the pointer, tolerating failure: setPointerCapture throws for
 * pointers that can't be captured (synthetic pointers, detached elements).
 * Without capture the drag still works while the pointer stays over the
 * divider — strictly better than never starting it.
 */
export function safeSetPointerCapture(
  target: Pick<Element, "setPointerCapture">,
  pointerId: number,
): boolean {
  try {
    target.setPointerCapture(pointerId);
    return true;
  } catch {
    return false;
  }
}

interface DragState {
  startX: number;
  startWidth: number;
  width: number;
}

export interface ResizableSidebar {
  /** Committed sidebar width in px; drives `--sidebar-width`. */
  width: number;
  dragging: boolean;
  /** Attach to the element carrying the `--sidebar-width` variable. */
  containerRef: React.RefObject<HTMLDivElement | null>;
  /** Spread onto the divider element. */
  dividerProps: {
    onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
    onPointerMove: (event: ReactPointerEvent<HTMLDivElement>) => void;
    onPointerUp: () => void;
    onPointerCancel: () => void;
    onDoubleClick: () => void;
  };
}

/**
 * Drag-to-resize state for the file sidebar. During a drag the width is
 * written straight to the `--sidebar-width` variable on the container (no
 * React re-render per pixel); the committed value lands in state — and
 * localStorage — only on pointer up. Double-click resets to the default.
 */
export function useResizableSidebar(): ResizableSidebar {
  const [width, setWidth] = useState(() =>
    initialSidebarWidth(
      localStorage.getItem(SIDEBAR_WIDTH_KEY),
      window.innerWidth,
    ),
  );
  const [dragging, setDragging] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const widthRef = useRef(width);
  widthRef.current = width;

  useEffect(() => {
    if (!dragging) return;
    document.body.classList.add("sidebar-resizing");
    return () => document.body.classList.remove("sidebar-resizing");
  }, [dragging]);

  const onPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      event.preventDefault();
      safeSetPointerCapture(event.currentTarget, event.pointerId);
      dragRef.current = {
        startX: event.clientX,
        startWidth: widthRef.current,
        width: widthRef.current,
      };
      setDragging(true);
    },
    [],
  );

  const onPointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (drag === null) return;
      const next = clampSidebarWidth(
        drag.startWidth + (event.clientX - drag.startX),
        window.innerWidth,
      );
      drag.width = next;
      containerRef.current?.style.setProperty("--sidebar-width", `${next}px`);
    },
    [],
  );

  const endDrag = useCallback(() => {
    const drag = dragRef.current;
    if (drag === null) return;
    dragRef.current = null;
    setDragging(false);
    setWidth(drag.width);
    localStorage.setItem(SIDEBAR_WIDTH_KEY, String(drag.width));
  }, []);

  const onDoubleClick = useCallback(() => {
    setWidth(DEFAULT_SIDEBAR_WIDTH);
    localStorage.removeItem(SIDEBAR_WIDTH_KEY);
  }, []);

  return {
    width,
    dragging,
    containerRef,
    dividerProps: {
      onPointerDown,
      onPointerMove,
      onPointerUp: endDrag,
      onPointerCancel: endDrag,
      onDoubleClick,
    },
  };
}
