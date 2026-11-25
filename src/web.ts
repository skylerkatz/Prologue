// Web mode transport: what the Tauri bridge does in the app, done over
// HTTP against `prologue-server` (src-tauri/prologue-server). Commands go
// to POST /api/invoke/<command> with the same args object `invoke` takes;
// events arrive over Server-Sent Events from /api/events.

import {
  COMMENTS_CHANGED_EVENT,
  MENU_HIDE_RESOLVED_EVENT,
  MENU_VIEW_ARCHIVED_EVENT,
  REPO_CHANGED_EVENT,
} from "./generated/events";
import type { BranchList, DiffMode, RepoInfo, Review } from "./types";
import type { LoadedReview } from "./ipc";

/** Remove index.html's loading skeleton: the review, or an error, is on
 * screen. Safe to call again. */
export function finishBoot(): void {
  document.getElementById("boot-skeleton")?.remove();
}

/** What the server was started for (`prologue-server --repo … --base …`). */
export interface WebConfig {
  repoPath: string;
  baseRef: string | null;
  /** Send to Amp is configured (the webhook URL never leaves the server). */
  ampWebhook: boolean;
  /** open_repo, list_branches, and find_active_review for the served repo,
   * answered up front to save their round trips; null if any failed. */
  start: {
    repo: RepoInfo;
    branchList: BranchList;
    activeReview: Review | null;
  } | null;
}

/**
 * What prologue-server inlines into index.html so the first render needs
 * no API round trip: the /api/config and /api/me answers, and the review
 * the page opens first, with the coordinates it was computed for. Each
 * part is null when the server could not produce it; the page then makes
 * that call itself.
 */
interface Boot {
  config: WebConfig;
  me: WebViewer | null;
  review: {
    repoPath: string;
    branch: string;
    base: string;
    mode: DiffMode;
    ignoreWhitespace: boolean;
    data: LoadedReview;
  } | null;
}

let bootRead = false;
let boot: Boot | null = null;

function readBoot(): Boot | null {
  if (!bootRead) {
    bootRead = true;
    try {
      const text = document.getElementById("prologue-boot")?.textContent;
      boot = text ? (JSON.parse(text) as Boot) : null;
    } catch {
      boot = null;
    }
  }
  return boot;
}

/** The inlined review, once, and only for the exact view it was computed
 * for; later loads (and a different stored mode) ask the server. */
export function takeBootReview(
  repoPath: string,
  branch: string,
  base: string,
  mode: DiffMode,
  ignoreWhitespace: boolean,
): LoadedReview | null {
  const review = readBoot()?.review ?? null;
  if (review === null) {
    return null;
  }
  boot!.review = null;
  const matches =
    review.repoPath === repoPath &&
    review.branch === branch &&
    review.base === base &&
    review.mode === mode &&
    review.ignoreWhitespace === ignoreWhitespace;
  return matches ? review.data : null;
}

/** Tell the server which view this browser shows (localStorage is out of
 * its reach), so the review it inlines next time is the one asked for.
 * A null mode is `auto`: the viewer never picked one, so the server
 * chooses as App.tsx does (working tree when it has uncommitted changes). */
export function rememberView(
  mode: DiffMode | null,
  ignoreWhitespace: boolean,
): void {
  document.cookie = `prologue-view=${mode ?? "auto"}.${ignoreWhitespace ? 1 : 0}; path=/; max-age=31536000; SameSite=Lax`;
}

let configPromise: Promise<WebConfig> | null = null;

export function webConfig(): Promise<WebConfig> {
  const inlined = readBoot()?.config;
  if (inlined !== undefined) {
    configPromise ??= Promise.resolve(inlined);
  }
  configPromise ??= fetch("/api/config").then((res) => {
    if (!res.ok) {
      throw new Error(`Could not load the server config (${res.status})`);
    }
    return res.json() as Promise<WebConfig>;
  });
  return configPromise;
}

/** The Amp user viewing this page, as the portal's identity headers name
 * them; null outside a portal (or when the lookup fails). */
export interface WebViewer {
  id: string;
  displayName: string;
  email: string | null;
  /** The thread's owner: may edit and delete anyone's comments (the
   * server enforces it). */
  isOwner: boolean;
  /** An owner is configured: comments with no stored author are theirs. */
  ownerConfigured: boolean;
}

let viewer: WebViewer | null = null;
let viewerStarted = false;
const viewerListeners = new Set<() => void>();

/** Ask the server once who is viewing; main.tsx starts it at load, so it
 * runs in parallel with the first diff load and never blocks rendering. */
export function loadWebViewer(): void {
  if (viewerStarted) {
    return;
  }
  viewerStarted = true;
  const inlined = readBoot();
  if (inlined !== null) {
    viewer = inlined.me;
    return;
  }
  fetch("/api/me")
    .then((res) => (res.ok ? (res.json() as Promise<WebViewer | null>) : null))
    .then((me) => {
      viewer = me;
      viewerListeners.forEach((notify) => notify());
    })
    .catch(() => {});
}

/** useSyncExternalStore source for the viewer. */
export function subscribeWebViewer(notify: () => void): () => void {
  viewerListeners.add(notify);
  return () => viewerListeners.delete(notify);
}

export function getWebViewer(): WebViewer | null {
  return viewer;
}

// The native menu's state, mirrored from the commands that drive it in the
// app; the keyboard stand-ins below read it.
const menu = { repoOpen: false, hideResolved: false };
/** The repo this client asked the server to watch, re-sent after a
 * reconnect in case the server restarted and lost its watch. */
let watchedRepo: string | null = null;

/** `invoke`, over fetch. Rejects with the server's error string, like the
 * Tauri bridge rejects with the command's. */
export async function webInvoke<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  switch (command) {
    case "set_repo_menu_enabled":
      menu.repoOpen = args.enabled === true;
      return undefined as T;
    case "set_hide_resolved_checked":
      menu.hideResolved = args.checked === true;
      return undefined as T;
    case "start_watching":
      watchedRepo = String(args.repoPath);
      ensureEvents();
      break;
    case "stop_watching":
      watchedRepo = null;
      break;
  }
  let res: Response;
  try {
    res = await fetch(`/api/invoke/${command}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(args),
    });
  } catch {
    throw "Lost connection to the Prologue server — it may be restarting; try again in a moment.";
  }
  const body: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    throw typeof body === "string" ? body : `Server error (${res.status})`;
  }
  return body as T;
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const listeners = new Map<string, Set<Handler>>();

function dispatch(event: string, payload: unknown) {
  for (const handler of listeners.get(event) ?? []) {
    handler(payload);
  }
}

/** Subscribe to a backend event; returns the unsubscribe function. */
export function listenWeb(event: string, handler: Handler): () => void {
  ensureEvents();
  let set = listeners.get(event);
  if (set === undefined) {
    set = new Set();
    listeners.set(event, set);
  }
  set.add(handler);
  return () => set.delete(handler);
}

/** Events the server sends; each means "refetch", never a delta. */
const SERVER_EVENTS = [REPO_CHANGED_EVENT, COMMENTS_CHANGED_EVENT];
/** Short enough that viewers are back within seconds of a portal redeploy;
 * retries are one cheap GET each. */
const MAX_RETRY_MS = 5_000;
/** The server pings every 10s when idle; this much silence means the
 * stream is dead even if the connection looks open. */
const SILENCE_LIMIT_MS = 30_000;
let started = false;

/**
 * Keep one SSE stream open for the page's lifetime, through server
 * restarts and Amp redeploys. Two failure shapes need handling here:
 * EventSource gives up for good on an HTTP error (what a proxy answers
 * mid-deploy), and a proxy can keep the browser's half of a dead stream
 * open, so no error ever fires. A closed stream is reopened with backoff;
 * a silent one (no event or ping within SILENCE_LIMIT_MS) is closed and
 * reopened. Every `hello` after the first means events may have been
 * missed: re-target the watch if the server lost it and refresh as if
 * both events fired.
 */
function ensureEvents() {
  if (started) {
    return;
  }
  started = true;
  installMenuShortcuts();
  let connectedBefore = false;
  let retryMs = 1000;
  const connect = () => {
    const source = new EventSource("/api/events");
    let watchdog = 0;
    const reopen = (delayMs: number) => {
      clearTimeout(watchdog);
      source.close();
      setTimeout(connect, delayMs);
    };
    const alive = () => {
      clearTimeout(watchdog);
      watchdog = window.setTimeout(() => reopen(0), SILENCE_LIMIT_MS);
    };
    alive();
    source.addEventListener("ping", alive);
    source.addEventListener("hello", (e) => {
      alive();
      retryMs = 1000;
      const { watching } = JSON.parse((e as MessageEvent).data) as {
        watching: string | null;
      };
      // On the first connect the start_watching that opened this stream is
      // still in flight; only a reconnect can find the watch lost.
      if (connectedBefore && watchedRepo !== null && watching !== watchedRepo) {
        void webInvoke("start_watching", { repoPath: watchedRepo }).catch(
          () => {},
        );
      }
      if (connectedBefore) {
        dispatch(COMMENTS_CHANGED_EVENT, null);
        if (watchedRepo !== null) {
          dispatch(REPO_CHANGED_EVENT, watchedRepo);
        }
      }
      connectedBefore = true;
    });
    for (const name of SERVER_EVENTS) {
      source.addEventListener(name, (e) => {
        alive();
        dispatch(name, JSON.parse((e as MessageEvent).data));
      });
    }
    source.onerror = () => {
      if (source.readyState === EventSource.CLOSED) {
        reopen(retryMs);
        retryMs = Math.min(retryMs * 2, MAX_RETRY_MS);
      }
    };
  };
  connect();
}

/**
 * Keyboard stand-ins for the native menu items that have no other entry
 * point: ⌘/Ctrl+⇧+H (Hide Resolved Comments) and ⌘/Ctrl+⇧+A (Archived
 * Reviews…). Refresh is the watcher's job (and the browser's reload);
 * Keyboard Shortcuts is already `?`.
 */
function installMenuShortcuts() {
  window.addEventListener("keydown", (e) => {
    if (!menu.repoOpen || !(e.metaKey || e.ctrlKey) || !e.shiftKey) {
      return;
    }
    const key = e.key.toLowerCase();
    if (key === "h") {
      e.preventDefault();
      dispatch(MENU_HIDE_RESOLVED_EVENT, !menu.hideResolved);
    } else if (key === "a") {
      e.preventDefault();
      dispatch(MENU_VIEW_ARCHIVED_EVENT, null);
    }
  });
}
