import { useCallback, useEffect, useRef, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

/**
 * Double-clicking a file name copies its absolute path to the clipboard so
 * it can be pasted straight into an editor. Returns the transient toast
 * state alongside the copy action.
 */
export function useCopyPath(): {
  copied: boolean;
  copyPath: (path: string) => void;
} {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  // Stable so handlers built on it don't defeat row memoization.
  const copyPath = useCallback((path: string) => {
    void writeText(path)
      .then(() => {
        window.clearTimeout(timer.current);
        setCopied(true);
        timer.current = window.setTimeout(() => setCopied(false), 2000);
      })
      .catch(() => {
        // Clipboard writes only fail in odd environments; the double-click
        // is a convenience, so fail silently rather than surface an error.
      });
  }, []);

  return { copied, copyPath };
}
