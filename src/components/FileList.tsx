import { useState } from "react";
import type {
  DiffSummary,
  FileReviewState,
  FileStatus,
  FileSummary,
} from "../types";
import { useCopyPath } from "./useCopyPath";

const STATUS_LABELS: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
};

interface FileListProps {
  summary: DiffSummary;
  /** Repo root, for building the absolute path an ⌥ double-click copies. */
  repoPath: string;
  /** Open (unresolved) comment count per file path; absent means zero. */
  openCounts: ReadonlyMap<string, number>;
  /** Per-file reviewed state; reviewed rows dim, "changed" rows get a dot. */
  reviewStates: ReadonlyMap<string, FileReviewState>;
  onSelect: (path: string) => void;
}

function FileRow({
  file,
  openCount,
  selected,
  reviewState,
  onSelect,
  onCopyPath,
}: {
  file: FileSummary;
  openCount: number;
  selected: boolean;
  reviewState: FileReviewState | undefined;
  onSelect: (path: string) => void;
  onCopyPath: (absolute: boolean) => void;
}) {
  return (
    <li>
      <button
        type="button"
        className={
          "file-row" +
          (selected ? " selected" : "") +
          (reviewState === "reviewed" ? " reviewed" : "")
        }
        onClick={() => onSelect(file.path)}
      >
        <span
          className={`status-badge status-${file.status}`}
          title={file.status}
          aria-label={file.status}
        >
          {STATUS_LABELS[file.status]}
        </span>
        {/* Paths truncate from the LEFT: rtl direction + <bdi> keeps the
            filename end visible without reordering the text itself. */}
        <span
          className="file-path"
          title={`${file.path}\nDouble-click to copy the path (⌥ for absolute)`}
          onDoubleClick={(event) => onCopyPath(event.altKey)}
        >
          <bdi>
            {file.oldPath !== null && (
              <>
                <span className="file-old-path">{file.oldPath}</span>
                <span className="file-rename-arrow" aria-hidden="true">
                  {" → "}
                </span>
              </>
            )}
            {file.path}
          </bdi>
        </span>
        {reviewState === "reviewed" && (
          <span className="reviewed-check" title="Reviewed" aria-label="Reviewed">
            ✓
          </span>
        )}
        {reviewState === "changed" && (
          <span
            className="changed-dot"
            title="Changed since you reviewed it"
            aria-label="Changed since review"
          />
        )}
        {openCount > 0 && (
          <span
            className="comment-count"
            title={`${openCount} open ${openCount === 1 ? "comment" : "comments"}`}
          >
            {openCount}
          </span>
        )}
        {file.binary ? (
          <span className="file-counts file-binary">BIN</span>
        ) : (
          <span className="file-counts">
            <span className="count-added">+{file.additions}</span>
            <span className="count-deleted">−{file.deletions}</span>
          </span>
        )}
      </button>
    </li>
  );
}

export function FileList({
  summary,
  repoPath,
  openCounts,
  reviewStates,
  onSelect,
}: FileListProps) {
  // Purely visual: the ribbon bookmark on the row last clicked.
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const { copied, copyPath } = useCopyPath(repoPath);
  const reviewedCount = summary.files.filter(
    (f) => reviewStates.get(f.path) === "reviewed",
  ).length;
  return (
    <div className="file-list">
      <div className="file-list-header">
        <span>
          {summary.files.length}{" "}
          {summary.files.length === 1 ? "file" : "files"} changed
          {reviewedCount > 0 && (
            <span className="reviewed-progress">
              {" · "}
              {reviewedCount}/{summary.files.length} viewed
            </span>
          )}
        </span>
        <span className="file-counts">
          <span className="count-added">+{summary.totalAdditions}</span>
          <span className="count-deleted">−{summary.totalDeletions}</span>
        </span>
      </div>
      <ul>
        {summary.files.map((file) => (
          <FileRow
            key={file.path}
            file={file}
            openCount={openCounts.get(file.path) ?? 0}
            selected={selectedPath === file.path}
            reviewState={reviewStates.get(file.path)}
            onSelect={(path) => {
              setSelectedPath(path);
              onSelect(path);
            }}
            onCopyPath={(absolute) => copyPath(file.path, absolute)}
          />
        ))}
      </ul>
      {copied && (
        <div className="copy-toast" role="status">
          {copied === "absolute"
            ? "Copied absolute path to clipboard"
            : "Copied file path to clipboard"}
        </div>
      )}
    </div>
  );
}
