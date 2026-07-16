import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { listBranches, openRepo, startWatching, stopWatching } from "./ipc";
import { addRecentRepo, getRecentRepos, removeRecentRepo } from "./recents";
import type { BranchList, RepoInfo, WorkingTreeMode } from "./types";
import { ReviewShell } from "./components/ReviewShell";
import { WelcomePage } from "./components/WelcomePage";
import "./App.css";

interface OpenRepoState {
  repo: RepoInfo;
  branchList: BranchList;
}

function App() {
  const [recents, setRecents] = useState<string[]>([]);
  const [openState, setOpenState] = useState<OpenRepoState | null>(null);
  const [branch, setBranch] = useState("");
  const [baseBranch, setBaseBranch] = useState("");
  const [mode, setMode] = useState<WorkingTreeMode>("committed");
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
      setOpenState({ repo, branchList });
      setBranch(branchList.current);
      setBaseBranch(branchList.defaultBase);
      setMode("committed");
      setRecents(await addRecentRepo(repo.path));
      // Auto-refresh is an enhancement; if watching fails, the manual
      // Refresh button still covers everything.
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
  // .git activity; run the exact same path as the manual Refresh button.
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

  if (!openState) {
    return (
      <WelcomePage
        recents={recents}
        error={error}
        onPickRepo={() => void pickRepo()}
        onOpenRecent={(path) => void openRepoAt(path)}
        onRemoveRecent={(path) => void removeRecent(path)}
      />
    );
  }

  return (
    <ReviewShell
      repo={openState.repo}
      branchList={openState.branchList}
      branch={branch}
      baseBranch={baseBranch}
      mode={mode}
      refreshKey={refreshKey}
      onBranchChange={setBranch}
      onBaseBranchChange={setBaseBranch}
      onModeChange={setMode}
      onRefresh={() => void refresh()}
      onSwitchRepo={() => {
        stopWatching().catch(() => {});
        setOpenState(null);
      }}
    />
  );
}

export default App;
