import { isTauri } from "@tauri-apps/api/core";
import { load } from "@tauri-apps/plugin-store";

const STORE_FILE = "settings.json";
const RECENTS_KEY = "recentRepos";
const MAX_RECENTS = 10;

// Persisted in the app data dir via tauri-plugin-store, so recents survive
// app restarts. Web mode keeps them in the browser's localStorage.

const WEB_RECENTS_KEY = "prologue.recentRepos";

async function readRecents(): Promise<string[]> {
  if (!isTauri()) {
    const stored: unknown = JSON.parse(
      localStorage.getItem(WEB_RECENTS_KEY) ?? "[]",
    );
    return Array.isArray(stored) ? stored.map(String) : [];
  }
  const store = await load(STORE_FILE);
  return (await store.get<string[]>(RECENTS_KEY)) ?? [];
}

async function writeRecents(paths: string[]): Promise<void> {
  if (!isTauri()) {
    localStorage.setItem(WEB_RECENTS_KEY, JSON.stringify(paths));
    return;
  }
  const store = await load(STORE_FILE);
  await store.set(RECENTS_KEY, paths);
  await store.save();
}

export function getRecentRepos(): Promise<string[]> {
  return readRecents();
}

export async function addRecentRepo(path: string): Promise<string[]> {
  const current = await readRecents();
  const next = [path, ...current.filter((p) => p !== path)].slice(
    0,
    MAX_RECENTS,
  );
  await writeRecents(next);
  return next;
}

export async function removeRecentRepo(path: string): Promise<string[]> {
  const next = (await readRecents()).filter((p) => p !== path);
  await writeRecents(next);
  return next;
}
