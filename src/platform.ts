// Host services that differ between the Tauri app and web mode (the UI
// served by prologue-server). Everything Tauri-plugin-shaped that the UI
// needs goes through here, so a browser never calls into a missing bridge.

import { isTauri } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { open, message } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { version as packageVersion } from "../package.json";

export { isTauri };

/** The app version; web mode reports the frontend build's. */
export function appVersion(): Promise<string> {
  return isTauri() ? getVersion() : Promise.resolve(packageVersion);
}

/**
 * Ask for a repository directory. Web mode has no file picker into the
 * server's filesystem, so it asks for a path on the server instead.
 */
export async function pickDirectory(
  title: string,
  suggestion?: string,
): Promise<string | null> {
  if (isTauri()) {
    const selected = await open({ directory: true, multiple: false, title });
    return typeof selected === "string" ? selected : null;
  }
  const typed = window.prompt(
    `${title} (a path on the Prologue server)`,
    suggestion,
  );
  return typed?.trim() ? typed.trim() : null;
}

export async function showMessage(
  text: string,
  options: { title: string; kind?: "info" | "warning" | "error" },
): Promise<void> {
  if (isTauri()) {
    await message(text, options);
  } else {
    window.alert(`${options.title}\n\n${text}`);
  }
}

/** Open an http(s)/mailto/tel link outside the app: the system handler in
 * Tauri, a new tab in web mode. */
export async function openExternal(href: string): Promise<void> {
  if (isTauri()) {
    await openUrl(href);
  } else {
    window.open(href, "_blank", "noopener,noreferrer");
  }
}

/** Copy text; rejects where no clipboard is reachable. */
export function writeClipboard(text: string): Promise<void> {
  if (isTauri()) {
    return writeText(text);
  }
  if (navigator.clipboard === undefined) {
    return Promise.reject(new Error("No clipboard access in this browser"));
  }
  return navigator.clipboard.writeText(text);
}

/**
 * Hand an exported review to the user; resolves with the confirmation to
 * show. This is the one place exports leave the app: the clipboard (the
 * app, and browsers that allow it), else a file download. A webhook sink —
 * POSTing `{ format, text }` to a configured URL instead — belongs here
 * too, as another branch ahead of the clipboard.
 */
export async function deliverExport(
  text: string,
  label: string,
  fileStem = "prologue-review",
): Promise<string> {
  try {
    await writeClipboard(text);
    return `Copied ${label} to clipboard`;
  } catch (e) {
    if (isTauri()) {
      throw e;
    }
  }
  // Browsers refuse clipboard writes outside a secure context or after the
  // click's user activation expired (the export round-trip can outlast it).
  const fileName = `${fileStem}.txt`;
  const url = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
  return `Clipboard unavailable — downloaded ${label} as ${fileName}`;
}
