import type { DiffSummary, FileStatus, FileSummary } from "../types";

const STATUS_LABELS: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
};

interface FileListProps {
  summary: DiffSummary;
  onSelect: (path: string) => void;
}

function FileRow({
  file,
  onSelect,
}: {
  file: FileSummary;
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

export function FileList({ summary, onSelect }: FileListProps) {
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
          <FileRow key={file.path} file={file} onSelect={onSelect} />
        ))}
      </ul>
    </div>
  );
}
