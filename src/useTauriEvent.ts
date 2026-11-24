import { useEffect, useRef } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { listenWeb } from "./web";

/** The part of a Tauri event the handlers read; web mode builds the same. */
export interface BackendEvent<T> {
  payload: T;
}

/**
 * Subscribe to a backend event for the component's lifetime: a Tauri event
 * in the app, the server's SSE stream in web mode. The handler is read
 * through a latest-ref, so callers can pass a fresh closure every render
 * without re-subscribing. `listen` resolves asynchronously, so cleanup both
 * unlistens and flags disposal in case the subscription arrives after
 * unmount.
 */
export function useTauriEvent<T = unknown>(
  event: string,
  handler: (event: BackendEvent<T>) => void,
): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  useEffect(() => {
    if (!isTauri()) {
      return listenWeb(event, (payload) =>
        handlerRef.current({ payload: payload as T }),
      );
    }
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
