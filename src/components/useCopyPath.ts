import { useCallback, useEffect, useRef, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

/**
 * Double-clicking a file name copies its repo-relative path; ⌥ double-click
 * copies the absolute path instead. Returns the transient toast state (which
 * of the two was copied) alongside the copy action.
 */
export function useCopyPath(repoPath: string): {
  copied: "relative" | "absolute" | null;
  copyPath: (relativePath: string, absolute: boolean) => void;
} {
  const [copied, setCopied] = useState<"relative" | "absolute" | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  // Stable so handlers built on it don't defeat row memoization.
  const copyPath = useCallback(
    (relativePath: string, absolute: boolean) => {
      void writeText(absolute ? `${repoPath}/${relativePath}` : relativePath)
        .then(() => {
          window.clearTimeout(timer.current);
          setCopied(absolute ? "absolute" : "relative");
          timer.current = window.setTimeout(() => setCopied(null), 2000);
        })
        .catch(() => {
          // Clipboard writes only fail in odd environments; the double-click
          // is a convenience, so fail silently rather than surface an error.
        });
    },
    [repoPath],
  );

  return { copied, copyPath };
}
