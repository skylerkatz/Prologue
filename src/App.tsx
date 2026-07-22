import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import {
  findActiveReview,
  listBranches,
  openRepo,
  setHideResolvedChecked,
  setRepoMenuEnabled,
  startWatching,
  stopWatching,
} from "./ipc";
import { addRecentRepo, getRecentRepos, removeRecentRepo } from "./recents";
import type { BranchList, RepoInfo, WorkingTreeMode } from "./types";
import { ReviewShell } from "./components/ReviewShell";
import { TitleBar } from "./components/TitleBar";
import { WelcomePage } from "./components/WelcomePage";
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

/** The stored mode, or "committed" if nothing valid is stored. */
function readStoredMode(): WorkingTreeMode {
  const stored = localStorage.getItem(MODE_KEY);
  return stored === "committed" || stored === "staged" || stored === "all"
    ? stored
    : "committed";
}

function App() {
  const [recents, setRecents] = useState<string[]>([]);
  const [openState, setOpenState] = useState<OpenRepoState | null>(null);
  const [branch, setBranch] = useState("");
  const [baseBranch, setBaseBranch] = useState("");
  // Both toggles are global preferences, sticky across launches and repos.
  const [mode, setMode] = useState<WorkingTreeMode>(readStoredMode);
  const [hideWhitespace, setHideWhitespace] = useState(
    () => localStorage.getItem(HIDE_WHITESPACE_KEY) === "true",
  );
  const [hideResolved, setHideResolved] = useState(
    () => localStorage.getItem(HIDE_RESOLVED_KEY) === "true",
  );
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getRecentRepos()
      .then(setRecents)
      .catch(() => setRecents([]));
  }, []);

  async function openRepoAt(path: string) {
    setError(null);
    try {
      const repo = await openRepo(path);
      const branchList = await listBranches(repo.path);
      // Resume where the review left off: the branch's active review pins
      // the base it was last computed against, so reopening the app doesn't
      // snap back to the auto-detected default. Must happen before the
      // review shell's open_review call, which writes the base back.
      let baseBranch = branchList.defaultBase;
      try {
        const active = await findActiveReview(repo.path, branchList.current);
        if (active !== null && branchList.branches.includes(active.baseRef)) {
          baseBranch = active.baseRef;
        }
      } catch {
        // No stored base to restore; the detected default still works.
      }
      setOpenState({ repo, branchList });
      setBranch(branchList.current);
      setBaseBranch(baseBranch);
      setRecents(await addRecentRepo(repo.path));
      // Auto-refresh is an enhancement; if watching fails, the manual
      // View > Refresh still covers everything.
      startWatching(repo.path).catch(() => {});
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  async function pickRepo() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Choose a git repository",
    });
    if (typeof selected === "string") {
      await openRepoAt(selected);
    }
  }

  async function removeRecent(path: string) {
    setRecents(await removeRecentRepo(path));
  }

  function changeMode(next: WorkingTreeMode) {
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
    } catch {
      // Keep the stale branch list; the diff itself still recomputes.
    }
    setRefreshKey((key) => key + 1);
  }

  // A ref so the repo-changed subscription below survives across renders
  // without re-subscribing every time `refresh`'s closure is recreated.
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  // The Rust watcher emits `repo-changed` (debounced) on working-tree or
  // .git activity; run the exact same path as View > Refresh.
  const repoPath = openState?.repo.path ?? null;
  useEffect(() => {
    if (repoPath === null) {
      return;
    }
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<string>("repo-changed", (event) => {
      if (event.payload === repoPath) {
        void refreshRef.current();
      }
    }).then((fn) => {
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
  }, [repoPath]);

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
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<boolean>("menu-hide-resolved", (event) => {
      setHideResolved(event.payload);
      localStorage.setItem(HIDE_RESOLVED_KEY, String(event.payload));
    }).then((fn) => {
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
  }, []);

  // View > Refresh (⌘R) — same path as the old toolbar Refresh button.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen("menu-refresh", () => {
      void refreshRef.current();
    }).then((fn) => {
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
  }, []);

  return (
    <div className="app-shell">
      <TitleBar branch={openState ? branch : null} />
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
    </div>
  );
}

export default App;
