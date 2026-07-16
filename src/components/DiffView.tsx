import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { getContextLines, getFileDiff } from "../ipc";
import { guardReason, type GuardReason } from "../diff/guards";
import {
  buildRows,
  computeGaps,
  CONTEXT_CHUNK,
  estimateRowHeight,
  initialFileState,
  rowKey,
  type FileViewState,
  type Row,
} from "../diff/rows";
import type {
  DiffLine,
  DiffSummary,
  FileStatus,
  FileSummary,
  WorkingTreeMode,
} from "../types";

/** Parallel `get_file_diff` calls; each recomputes the repo diff in Rust. */
const MAX_CONCURRENT_LOADS = 3;

const STATUS_LABELS: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
};

interface DiffViewProps {
  repoPath: string;
  base: string;
  head: string;
  mode: WorkingTreeMode;
  summary: DiffSummary;
  /** Bumping `nonce` scrolls the file's header into view. */
  scrollTarget: { path: string; nonce: number } | null;
}

type ExpandDirection = "top" | "bottom" | "all";

export function DiffView({
  repoPath,
  base,
  head,
  mode,
  summary,
  scrollTarget,
}: DiffViewProps) {
  const [states, setStates] = useState<FileViewState[]>(() =>
    summary.files.map(initialFileState),
  );
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inFlight = useRef(new Set<number>());

  const updateState = useCallback(
    (fi: number, update: (state: FileViewState) => FileViewState) => {
      setStates((prev) => prev.map((s, i) => (i === fi ? update(s) : s)));
    },
    [],
  );

  const loadFile = useCallback(
    (fi: number) => {
      if (inFlight.current.has(fi)) {
        return;
      }
      inFlight.current.add(fi);
      getFileDiff(repoPath, base, head, mode, summary.files[fi].path)
        .then((diff) => {
          updateState(fi, (s) => ({
            ...s,
            diff,
            error: null,
            reveals: Array.from({ length: diff.hunks.length + 1 }, () => ({
              top: 0,
              bottom: 0,
            })),
          }));
        })
        .catch((e: unknown) => {
          updateState(fi, (s) => ({ ...s, error: errorText(e) }));
        })
        .finally(() => {
          inFlight.current.delete(fi);
        });
    },
    [repoPath, base, head, mode, summary, updateState],
  );

  const toggleFile = useCallback(
    (fi: number) => {
      updateState(fi, (s) => ({ ...s, expanded: !s.expanded }));
    },
    [updateState],
  );

  const forceLoadFile = useCallback(
    (fi: number) => {
      updateState(fi, (s) => ({ ...s, forceLoad: true }));
      loadFile(fi);
    },
    [updateState, loadFile],
  );

  const expandGap = useCallback(
    (fi: number, gi: number, direction: ExpandDirection) => {
      const state = states[fi];
      if (state.diff === null) {
        return;
      }
      const gap = computeGaps(state.diff)[gi];
      const reveal = state.reveals[gi] ?? { top: 0, bottom: 0 };
      const firstHidden = gap.start + reveal.top;
      const lastHidden = gap.end - reveal.bottom;
      if (firstHidden > lastHidden) {
        return;
      }
      let from = firstHidden;
      let to = lastHidden;
      if (direction === "top") {
        to = Math.min(firstHidden + CONTEXT_CHUNK - 1, lastHidden);
      } else if (direction === "bottom") {
        from = Math.max(lastHidden - CONTEXT_CHUNK + 1, firstHidden);
      }
      const count = to - from + 1;
      getContextLines(repoPath, head, mode, summary.files[fi].path, from, to)
        .then((ctx) => {
          updateState(fi, (s) => {
            const context = new Map(s.context);
            ctx.lines.forEach((content, i) => context.set(ctx.start + i, content));
            const reveals = s.reveals.map((r, i) => {
              if (i !== gi) {
                return r;
              }
              return direction === "bottom"
                ? { ...r, bottom: r.bottom + count }
                : { ...r, top: r.top + count };
            });
            return { ...s, context, reveals };
          });
        })
        .catch((e: unknown) => {
          updateState(fi, (s) => ({ ...s, error: errorText(e) }));
        });
    },
    [states, repoPath, head, mode, summary, updateState],
  );

  const rows = useMemo(
    () => buildRows(summary.files, states),
    [summary.files, states],
  );

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateRowHeight(rows[index]),
    getItemKey: (index) => rowKey(rows[index]),
    overscan: 12,
  });
  const items = virtualizer.getVirtualItems();

  // Lazily fetch hunks for expanded files whose rows are in the viewport.
  // Runs after every render; the guards keep it a cheap no-op once loaded.
  useEffect(() => {
    for (const item of items) {
      if (inFlight.current.size >= MAX_CONCURRENT_LOADS) {
        break;
      }
      const row = rows[item.index];
      if (row.kind !== "file" && row.kind !== "skeleton") {
        continue;
      }
      const state = states[row.fi];
      if (!state.expanded || state.diff !== null || state.error !== null) {
        continue;
      }
      if (guardReason(summary.files[row.fi]) !== null && !state.forceLoad) {
        continue;
      }
      loadFile(row.fi);
    }
  });

  useEffect(() => {
    if (scrollTarget === null) {
      return;
    }
    const index = rows.findIndex(
      (row) =>
        row.kind === "file" &&
        summary.files[row.fi].path === scrollTarget.path,
    );
    if (index >= 0) {
      virtualizer.scrollToIndex(index, { align: "start" });
    }
    // Keyed to the nonce alone: only a click should scroll, not row-model
    // changes.
  }, [scrollTarget?.nonce]);

  return (
    <div className="diff-scroll" ref={scrollRef}>
      <div
        className="diff-virtual"
        style={{ height: virtualizer.getTotalSize() }}
      >
        {items.map((item) => (
          <div
            key={item.key}
            data-index={item.index}
            ref={virtualizer.measureElement}
            className="diff-row"
            style={{ transform: `translateY(${item.start}px)` }}
          >
            <RowContent
              row={rows[item.index]}
              files={summary.files}
              states={states}
              onToggle={toggleFile}
              onLoad={forceLoadFile}
              onExpand={expandGap}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

interface RowContentProps {
  row: Row;
  files: FileSummary[];
  states: FileViewState[];
  onToggle: (fi: number) => void;
  onLoad: (fi: number) => void;
  onExpand: (fi: number, gi: number, direction: ExpandDirection) => void;
}

function RowContent({
  row,
  files,
  states,
  onToggle,
  onLoad,
  onExpand,
}: RowContentProps) {
  switch (row.kind) {
    case "file":
      return (
        <FileHeaderRow
          file={files[row.fi]}
          expanded={states[row.fi].expanded}
          onToggle={() => onToggle(row.fi)}
        />
      );
    case "notice":
      return (
        <GuardNoticeRow
          file={files[row.fi]}
          reason={row.reason}
          onLoad={() => onLoad(row.fi)}
        />
      );
    case "skeleton":
      return (
        <div className="diff-skeleton" style={{ height: row.height }}>
          Loading diff…
        </div>
      );
    case "error":
      return <div className="diff-file-error">{row.message}</div>;
    case "empty":
      return <div className="diff-file-empty">No content changes.</div>;
    case "hunk":
      return <div className="hunk-header">{row.header}</div>;
    case "line":
      return <LineRow line={row.line} />;
    case "expand":
      return (
        <ExpandRow
          hidden={row.hidden}
          growTop={row.growTop}
          growBottom={row.growBottom}
          onExpand={(direction) => onExpand(row.fi, row.gi, direction)}
        />
      );
  }
}

function FileHeaderRow({
  file,
  expanded,
  onToggle,
}: {
  file: FileSummary;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="diff-file-header">
      <button
        type="button"
        className="file-toggle"
        aria-expanded={expanded}
        aria-label={expanded ? "Collapse file" : "Expand file"}
        onClick={onToggle}
      >
        {expanded ? "▾" : "▸"}
      </button>
      <span
        className={`status-badge status-${file.status}`}
        title={file.status}
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
    </div>
  );
}

function GuardNoticeRow({
  file,
  reason,
  onLoad,
}: {
  file: FileSummary;
  reason: GuardReason;
  onLoad: () => void;
}) {
  if (reason === "binary") {
    return <div className="diff-notice">Binary file — no text diff.</div>;
  }
  const label =
    reason === "oversize"
      ? `Large diff: ${(file.additions + file.deletions).toLocaleString()} changed lines.`
      : "Lockfile / generated file — hidden by default.";
  return (
    <div className="diff-notice">
      <span>{label}</span>
      <button type="button" className="load-diff-button" onClick={onLoad}>
        Load diff
      </button>
    </div>
  );
}

function LineRow({ line }: { line: DiffLine }) {
  return (
    <div className={`diff-line diff-line-${line.kind}`}>
      <span className="lineno">{line.oldLineno ?? ""}</span>
      <span className="lineno">{line.newLineno ?? ""}</span>
      <span className="line-sign" aria-hidden="true">
        {line.kind === "addition" ? "+" : line.kind === "deletion" ? "−" : ""}
      </span>
      <span className="line-content">{line.content}</span>
    </div>
  );
}

function ExpandRow({
  hidden,
  growTop,
  growBottom,
  onExpand,
}: {
  hidden: number;
  growTop: boolean;
  growBottom: boolean;
  onExpand: (direction: ExpandDirection) => void;
}) {
  if (hidden <= CONTEXT_CHUNK) {
    return (
      <div className="expand-row">
        <button type="button" onClick={() => onExpand("all")}>
          ↕ Expand {hidden} hidden {hidden === 1 ? "line" : "lines"}
        </button>
      </div>
    );
  }
  return (
    <div className="expand-row">
      {growTop && (
        <button
          type="button"
          title="Show the next 20 lines below the hunk above"
          onClick={() => onExpand("top")}
        >
          ↓ Show {CONTEXT_CHUNK}
        </button>
      )}
      <span className="expand-hidden-count">
        {hidden.toLocaleString()} unchanged lines
      </span>
      {growBottom && (
        <button
          type="button"
          title="Show the previous 20 lines above the hunk below"
          onClick={() => onExpand("bottom")}
        >
          ↑ Show {CONTEXT_CHUNK}
        </button>
      )}
      <button type="button" onClick={() => onExpand("all")}>
        Expand all
      </button>
    </div>
  );
}

function errorText(e: unknown): string {
  return typeof e === "string" ? e : String(e);
}
