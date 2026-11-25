import { useEffect, useState } from "react";
import { appVersion, isTauri, pickDirectory, showMessage } from "./platform";
import { finishBoot, rememberView, webConfig } from "./web";
import type { PluginUpdate } from "./web";
import {
  errorText,
  findActiveReview,
  listBranches,
  openRepo,
  setHideResolvedChecked,
  setRepoMenuEnabled,
  startWatching,
  stopWatching,
} from "./ipc";
import { useTauriEvent } from "./useTauriEvent";
import {
  MENU_CHECK_UPDATES_EVENT,
  MENU_HIDE_RESOLVED_EVENT,
  MENU_REFRESH_EVENT,
  MENU_WHATS_NEW_EVENT,
  REPO_CHANGED_EVENT,
} from "./generated/events";
import { addRecentRepo, getRecentRepos, removeRecentRepo } from "./recents";
import {
  checkForUpdate,
  checkForUpdateNow,
  installAndRelaunch,
} from "./updater";
import type { Update } from "@tauri-apps/plugin-updater";
import type { BranchList, RepoInfo, DiffMode, Review } from "./types";
import { ReviewShell } from "./components/ReviewShell";
import { WhatsNew, hasNotesFor } from "./components/WhatsNew";
import { TitleBar } from "./components/TitleBar";
import { WelcomePage } from "./components/WelcomePage";
import { PluginUpdateBanner } from "./components/web/PluginUpdateBanner";
import "./App.css";

interface OpenRepoState {
  repo: RepoInfo;
  branchList: BranchList;
}

/** localStorage key for the "Hide whitespace changes" preference. */
const HIDE_WHITESPACE_KEY = "prologue.hideWhitespace";

/** localStorage key for the working-tree mode preference. */
const MODE_KEY = "prologue.mode";

/** localStorage key for the "Hide Resolved Comments" preference. */
const HIDE_RESOLVED_KEY = "prologue.hideResolvedComments";

/** localStorage key for the last version whose release notes were shown. */
const LAST_SEEN_VERSION_KEY = "prologue.lastSeenVersion";

/** The mode the user picked, or null if they never picked one. */
function readStoredMode(): DiffMode | null {
  const stored = localStorage.getItem(MODE_KEY);
  return stored === "committed" || stored === "staged" || stored === "all"
    ? stored
    : null;
}

function App() {
  const [recents, setRecents] = useState<string[]>([]);
  const [openState, setOpenState] = useState<OpenRepoState | null>(null);
  const [branch, setBranch] = useState("");
  const [baseBranch, setBaseBranch] = useState("");
  // Both toggles are global preferences, sticky across launches and repos.
  const [mode, setMode] = useState<DiffMode>(
    () => readStoredMode() ?? "committed",
  );
  const [hideWhitespace, setHideWhitespace] = useState(
    () => localStorage.getItem(HIDE_WHITESPACE_KEY) === "true",
  );
  const [hideResolved, setHideResolved] = useState(
    () => localStorage.getItem(HIDE_RESOLVED_KEY) === "true",
  );
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<Update | null>(null);
  const [updateState, setUpdateState] = useState<
    "idle" | "installing" | "failed"
  >("idle");
  const [showWhatsNew, setShowWhatsNew] = useState(false);
  const [pluginUpdate, setPluginUpdate] = useState<PluginUpdate | null>(null);

  useEffect(() => {
    getRecentRepos()
      .then(setRecents)
      .catch(() => setRecents([]));
  }, []);

  // Web mode serves one repository (prologue-server --repo); open it on
  // load. Its configured base arrives as the branch list's default base.
  useEffect(() => {
    if (!isTauri()) {
      webConfig()
        .then((config) => {
          setPluginUpdate(config.pluginUpdate);
          return config.start !== null
            ? showRepo(
                config.start.repo,
                config.start.branchList,
                config.start.activeReview,
              )
            : openRepoAt(config.repoPath);
        })
        .catch((e) => setError(errorText(e)));
    }
    // Once per page load; openRepoAt only uses state setters.
  }, []);

  // The repository didn't open: drop web mode's loading skeleton to show
  // the welcome page with the error.
  useEffect(() => {
    if (error !== null) {
      finishBoot();
    }
  }, [error]);

  // Mirror the view into a cookie: the server inlines the review for this
  // view into the next page load (it cannot read localStorage).
  useEffect(() => {
    if (!isTauri()) {
      rememberView(readStoredMode() === null ? null : mode, hideWhitespace);
    }
  }, [mode, hideWhitespace]);

  // One update check per launch; checkForUpdate resolves null in dev builds
  // and on network failure, so the banner only ever appears on a real hit.
  useEffect(() => {
    checkForUpdate().then(setUpdate);
  }, []);

  // Open What's New once after an update. A missing stored version means a
  // fresh install (or first launch with this feature): record it silently
  // rather than greeting a new user with a changelog.
  useEffect(() => {
    appVersion().then((version) => {
      const seen = localStorage.getItem(LAST_SEEN_VERSION_KEY);
      localStorage.setItem(LAST_SEEN_VERSION_KEY, version);
      if (seen !== null && seen !== version && hasNotesFor(version)) {
        setShowWhatsNew(true);
      }
    });
  }, []);

  async function installUpdate() {
    if (update === null) {
      return;
    }
    setUpdateState("installing");
    try {
      await installAndRelaunch(update);
    } catch {
      setUpdateState("failed");
    }
  }

  async function openRepoAt(path: string) {
    setError(null);
    try {
      const repo = await openRepo(path);
      const branchList = await listBranches(repo.path);
      // Must happen before the review shell's open_review call, which
      // writes the base back.
      const active = await findActiveReview(repo.path, branchList.current).catch(
        // No stored base to restore; the detected default still works.
        () => null,
      );
      await showRepo(repo, branchList, active);
    } catch (e) {
      setError(errorText(e));
    }
  }

  /** Show an opened repo. Web mode gets these three answers from
   * /api/config in one round trip instead of three. */
  async function showRepo(
    repo: RepoInfo,
    branchList: BranchList,
    active: Review | null,
  ) {
    // Resume where the review left off: the branch's active review pins
    // the base it was last computed against, so reopening the app doesn't
    // snap back to the auto-detected default.
    const baseBranch =
      active !== null && branchList.branches.includes(active.baseRef)
        ? active.baseRef
        : branchList.defaultBase;
    setOpenState({ repo, branchList });
    showUncommittedWork(branchList);
    setBranch(branchList.current);
    setBaseBranch(baseBranch);
    // Auto-refresh is an enhancement; if watching fails, the manual
    // View > Refresh still covers everything.
    startWatching(repo.path).catch(() => {});
    setRecents(await addRecentRepo(repo.path));
  }

  async function pickRepo() {
    const selected = await pickDirectory(
      "Choose a git repository",
      recents[0],
    );
    if (selected !== null) {
      await openRepoAt(selected);
    }
  }

  async function removeRecent(path: string) {
    setRecents(await removeRecentRepo(path));
  }

  /** Web mode: agents leave their work uncommitted, so until the viewer
   * picks a mode, a checkout with uncommitted changes shows them (and
   * keeps showing them after a commit: that mode includes committed work).
   * prologue-server's inlined review follows the same rule. */
  function showUncommittedWork(branchList: BranchList) {
    if (!isTauri() && readStoredMode() === null && branchList.uncommitted) {
      setMode("all");
    }
  }

  function changeMode(next: DiffMode) {
    setMode(next);
    localStorage.setItem(MODE_KEY, next);
  }

  function changeHideWhitespace(hide: boolean) {
    setHideWhitespace(hide);
    localStorage.setItem(HIDE_WHITESPACE_KEY, String(hide));
  }

  /** Re-list branches (new ones appear) and recompute the current diff. */
  async function refresh() {
    if (!openState) {
      return;
    }
    try {
      const branchList = await listBranches(openState.repo.path);
      setOpenState({ repo: openState.repo, branchList });
      showUncommittedWork(branchList);
      // Web mode reviews whatever the server's checkout has out: a branch
      // switch there moves the review with it.
      if (!isTauri()) {
        setBranch(branchList.current);
      }
    } catch {
      // Keep the stale branch list; the diff itself still recomputes.
    }
    setRefreshKey((key) => key + 1);
  }

  // The Rust watcher emits `repo-changed` (debounced) on working-tree or
  // .git activity; run the exact same path as View > Refresh.
  const repoPath = openState?.repo.path ?? null;
  useTauriEvent<string>(REPO_CHANGED_EVENT, (event) => {
    if (repoPath !== null && event.payload === repoPath) {
      void refresh();
    }
  });

  // View > Refresh / Archived Reviews… only make sense with a repo open;
  // the items start disabled on the Rust side and follow the repo from here.
  useEffect(() => {
    setRepoMenuEnabled(repoPath !== null).catch(() => {});
  }, [repoPath]);

  // The menu check mark always mirrors the stored preference: corrects the
  // item on startup (it is built unchecked) and no-ops after menu clicks.
  useEffect(() => {
    setHideResolvedChecked(hideResolved).catch(() => {});
  }, [hideResolved]);

  // View > Hide Resolved Comments — Rust sends the item's new checked
  // state (macOS toggles it natively); adopt it as the preference.
  useTauriEvent<boolean>(MENU_HIDE_RESOLVED_EVENT, (event) => {
    setHideResolved(event.payload);
    localStorage.setItem(HIDE_RESOLVED_KEY, String(event.payload));
  });

  // View > Refresh (⌘R) — same path as the old toolbar Refresh button.
  useTauriEvent(MENU_REFRESH_EVENT, () => {
    void refresh();
  });

  // Prologue > Check for Updates… — unlike the launch check, a manual check
  // must answer either way: banner on a hit, dialog on a miss or failure.
  useTauriEvent(MENU_CHECK_UPDATES_EVENT, () => {
    void (async () => {
      try {
        const found = await checkForUpdateNow();
        if (found !== null) {
          setUpdate(found);
          setUpdateState("idle");
        } else {
          const version = await appVersion();
          await showMessage(`Prologue ${version} is the newest version.`, {
            title: "You're up to date",
          });
        }
      } catch (e) {
        await showMessage(errorText(e), {
          title: "Could not check for updates",
          kind: "error",
        });
      }
    })();
  });

  // Prologue > What's New… — reopen the release notes on demand.
  useTauriEvent(MENU_WHATS_NEW_EVENT, () => {
    setShowWhatsNew(true);
  });

  return (
    <div className="app-shell">
      {/* Web mode has no window chrome to dress up. */}
      {isTauri() && <TitleBar branch={openState ? branch : null} />}
      {update !== null && (
        <div className="update-banner" role="status">
          <span className="update-banner-text">
            {updateState === "failed"
              ? "The update could not be installed. It will be retried on the next launch."
              : `Prologue ${update.version} is available.`}
          </span>
          {updateState !== "failed" && (
            <button
              className="primary"
              onClick={() => void installUpdate()}
              disabled={updateState === "installing"}
            >
              {updateState === "installing"
                ? "Installing…"
                : "Restart to Update"}
            </button>
          )}
          <button
            className="update-banner-dismiss"
            onClick={() => setUpdate(null)}
            aria-label="Dismiss update notice"
            disabled={updateState === "installing"}
          >
            ×
          </button>
        </div>
      )}
      {pluginUpdate !== null && <PluginUpdateBanner update={pluginUpdate} />}
      {!openState ? (
        <WelcomePage
          recents={recents}
          error={error}
          onPickRepo={() => void pickRepo()}
          onOpenRecent={(path) => void openRepoAt(path)}
          onRemoveRecent={(path) => void removeRecent(path)}
        />
      ) : (
        <ReviewShell
          repo={openState.repo}
          branchList={openState.branchList}
          branch={branch}
          baseBranch={baseBranch}
          mode={mode}
          hideWhitespace={hideWhitespace}
          hideResolved={hideResolved}
          refreshKey={refreshKey}
          onBranchChange={setBranch}
          onBaseBranchChange={setBaseBranch}
          onModeChange={changeMode}
          onHideWhitespaceChange={changeHideWhitespace}
          onSwitchRepo={() => {
            stopWatching().catch(() => {});
            setOpenState(null);
          }}
        />
      )}
      {showWhatsNew && <WhatsNew onClose={() => setShowWhatsNew(false)} />}
    </div>
  );
}

export default App;
