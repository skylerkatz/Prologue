import { useCallback } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useToast, type ToastMessage } from "./useToast";

/**
 * Double-clicking a file name copies its repo-relative path; ⌥ double-click
 * copies the absolute path instead. Returns the confirmation toast alongside
 * the copy action.
 */
export function useCopyPath(repoPath: string): {
  toast: ToastMessage | null;
  copyPath: (relativePath: string, absolute: boolean) => void;
} {
  const { toast, showToast } = useToast();

  // Stable so handlers built on it don't defeat row memoization.
  const copyPath = useCallback(
    (relativePath: string, absolute: boolean) => {
      void writeText(absolute ? `${repoPath}/${relativePath}` : relativePath)
        .then(() =>
          showToast(
            absolute
              ? "Copied absolute path to clipboard"
              : "Copied file path to clipboard",
            2000,
          ),
        )
        .catch(() => {
          // Clipboard writes only fail in odd environments; the double-click
          // is a convenience, so fail silently rather than surface an error.
        });
    },
    [repoPath, showToast],
  );

  return { toast, copyPath };
}
