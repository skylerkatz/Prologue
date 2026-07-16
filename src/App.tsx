import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listBranches, openRepo } from "./ipc";
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
      onBranchChange={setBranch}
      onBaseBranchChange={setBaseBranch}
      onModeChange={setMode}
      onSwitchRepo={() => setOpenState(null)}
    />
  );
}

export default App;
