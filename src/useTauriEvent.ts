import { useEffect, useRef } from "react";
import { listen, type Event } from "@tauri-apps/api/event";

/**
 * Subscribe to a Tauri event for the component's lifetime. The handler is
 * read through a latest-ref, so callers can pass a fresh closure every
 * render without re-subscribing. `listen` resolves asynchronously, so
 * cleanup both unlistens and flags disposal in case the subscription
 * arrives after unmount.
 */
export function useTauriEvent<T = unknown>(
  event: string,
  handler: (event: Event<T>) => void,
): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<T>(event, (e) => handlerRef.current(e)).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [event]);
}
