import type { DiffSummary, FileStatus, FileSummary } from "../types";

const STATUS_LABELS: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
};

interface FileListProps {
  summary: DiffSummary;
}

function FileRow({ file }: { file: FileSummary }) {
  return (
    <li className="file-row">
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
      {file.binary ? (
        <span className="file-counts file-binary">BIN</span>
      ) : (
        <span className="file-counts">
          <span className="count-added">+{file.additions}</span>
          <span className="count-deleted">−{file.deletions}</span>
        </span>
      )}
    </li>
  );
}

export function FileList({ summary }: FileListProps) {
  if (summary.files.length === 0) {
    return (
      <div className="diff-empty">
        <p>
          No changes between <code>{summary.baseRef}</code> and{" "}
          <code>{summary.headRef}</code>.
        </p>
      </div>
    );
  }

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
          <FileRow key={file.path} file={file} />
        ))}
      </ul>
    </div>
  );
}
