import type { DiffSummary, FileStatus, FileSummary } from "../types";

const STATUS_LABELS: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
};

interface FileListProps {
  summary: DiffSummary;
  /** Open (unresolved) comment count per file path; absent means zero. */
  openCounts: ReadonlyMap<string, number>;
  onSelect: (path: string) => void;
}

function FileRow({
  file,
  openCount,
  onSelect,
}: {
  file: FileSummary;
  openCount: number;
  onSelect: (path: string) => void;
}) {
  return (
    <li>
      <button
        type="button"
        className="file-row"
        onClick={() => onSelect(file.path)}
      >
        <span
          className={`status-badge status-${file.status}`}
          title={file.status}
          aria-label={file.status}
        >
          {STATUS_LABELS[file.status]}
        </span>
        <span className="file-path" title={file.path}>
          {file.oldPath !== null && (
            <>
              <span className="file-old-path">{file.oldPath}</span>
              <span className="file-rename-arrow" aria-hidden="true">
                {" → "}
              </span>
            </>
          )}
          {file.path}
        </span>
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

export function FileList({ summary, openCounts, onSelect }: FileListProps) {
  return (
    <div className="file-list">
      <div className="file-list-header">
        <span>
          {summary.files.length}{" "}
          {summary.files.length === 1 ? "file" : "files"} changed
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
            onSelect={onSelect}
          />
        ))}
      </ul>
    </div>
  );
}
