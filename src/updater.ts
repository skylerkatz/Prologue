import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

/** The available update, or null (up to date, dev build, or offline). */
export async function checkForUpdate(): Promise<Update | null> {
  // Dev builds run against the working copy, which is usually ahead of the
  // newest release; polling GitHub from `tauri dev` would nag forever.
  if (!import.meta.env.PROD) {
    return null;
  }
  try {
    return await check();
  } catch {
    // Offline or the release feed is unreachable — never surface this; the
    // check runs again on next launch.
    return null;
  }
}

/**
 * Menu-triggered check: no dev gate, and errors propagate — a silent miss
 * would make Check for Updates… feel broken.
 */
export function checkForUpdateNow(): Promise<Update | null> {
  return check();
}

/** Download and install the update, then restart into the new version. */
export async function installAndRelaunch(update: Update): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}
