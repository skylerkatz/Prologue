import { memo, useCallback, useMemo, useState } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Element, Root, RootContent } from "hast";
import {
  blockTargetLine,
  classifyBlock,
  type DeletionAnchor,
  type FileMarkers,
} from "../diff/markers";

/**
 * Rendered new-side markdown for a file's rich view. Raw HTML in the
 * source is stripped (react-markdown's default) — the preview renders to
 * React elements only. Memoized on the document text so scroll-driven
 * re-renders of the virtual row never re-parse a large README.
 *
 * With `markers` set, top-level blocks get change bars (remark keeps each
 * block's source line range through to hast; intersecting it with the
 * hunks' new-side ranges is plain interval math) and deletion anchors
 * become red tombstones that expand the deleted source lines in place.
 * Rich view stays read-only: clicking a marked block hands off to
 * `onJumpToSource`, which flips the file to the source diff.
 */
export const MarkdownPreview = memo(function MarkdownPreview({
  content,
  path,
  markers,
  onJumpToSource,
}: {
  content: string;
  path: string;
  /** Null renders clean — added files, or hunks not (yet) loaded. */
  markers: FileMarkers | null;
  onJumpToSource: (path: string, newLine: number) => void;
}) {
  const rehypePlugins = useMemo(
    () => (markers === null ? [] : [() => markerTransform(markers)]),
    [markers],
  );

  // Tombstone divs planted by the transform become interactive markers;
  // every other div (none, normally — raw HTML is stripped) passes through.
  const components = useMemo<Components>(
    () => ({
      div(props) {
        const { node: _node, ...rest } = props;
        const anchorAttr = (rest as Record<string, unknown>)[
          "data-del-anchor"
        ];
        if (typeof anchorAttr === "string" && markers !== null) {
          const anchor = markers.deletions[Number(anchorAttr)];
          if (anchor !== undefined) {
            return <DeletionMarker anchor={anchor} />;
          }
        }
        return <div {...rest} />;
      },
    }),
    [markers],
  );

  // One delegated handler instead of a callback per block: the transform
  // can't carry functions through hast properties, only data attributes.
  const handleClick = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      const target = event.target as HTMLElement;
      if (target.closest("a, button, input, .md-del-marker") !== null) {
        return;
      }
      const block = target.closest("[data-md-line]");
      if (block !== null) {
        const line = Number(block.getAttribute("data-md-line"));
        if (Number.isFinite(line)) {
          onJumpToSource(path, line);
        }
      }
    },
    [onJumpToSource, path],
  );

  return (
    <div className="markdown-preview" onClick={handleClick}>
      <Markdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={rehypePlugins}
        components={components}
      >
        {content}
      </Markdown>
    </div>
  );
});

/**
 * Annotate top-level blocks with change classes + the new-side line a
 * comment should land on, and plant a tombstone div at each deletion
 * anchor: between blocks where the deletion fell between them, directly
 * after the block it fell inside. Anchors arrive sorted (hunks are).
 */
function markerTransform(markers: FileMarkers): (tree: Root) => void {
  return (tree) => {
    let next = 0;
    const out: RootContent[] = [];
    const tombstone = (index: number): Element => ({
      type: "element",
      tagName: "div",
      properties: { dataDelAnchor: String(index) },
      children: [],
    });
    const flushBefore = (line: number) => {
      while (
        next < markers.deletions.length &&
        markers.deletions[next].afterLine < line
      ) {
        out.push(tombstone(next));
        next++;
      }
    };
    for (const child of tree.children) {
      if (child.type !== "element" || child.position === undefined) {
        out.push(child);
        continue;
      }
      const start = child.position.start.line;
      const end = child.position.end.line;
      flushBefore(start);
      const mark = classifyBlock(markers, start, end);
      if (mark !== null) {
        const props = (child.properties ??= {});
        const marker = `md-block-${mark}`;
        const existing = props.className;
        props.className = Array.isArray(existing)
          ? [...existing, marker]
          : existing !== undefined && existing !== null
            ? [String(existing), marker]
            : [marker];
        const target = blockTargetLine(markers, start, end);
        if (target !== null) {
          props.dataMdLine = String(target);
          props.title = "Click to comment on this change in the source diff";
        }
      }
      out.push(child);
      // Anchors inside [start, end) belong to this block; emit them right
      // under it. An anchor at exactly `end` sits between blocks instead.
      flushBefore(end);
    }
    while (next < markers.deletions.length) {
      out.push(tombstone(next));
      next++;
    }
    tree.children = out;
  };
}

/**
 * Red tombstone with click-to-peek: expands into the deleted source lines
 * (data the hunks already shipped). Expansion is local state so a toggle
 * re-renders only this marker — the surrounding document never re-parses;
 * the virtualizer's ResizeObserver absorbs the height change.
 */
function DeletionMarker({ anchor }: { anchor: DeletionAnchor }) {
  const [expanded, setExpanded] = useState(false);
  const count = anchor.lines.length;
  return (
    <div className="md-del-marker">
      <button
        type="button"
        className="md-del-toggle"
        aria-expanded={expanded}
        title={expanded ? "Hide the deleted lines" : "Show the deleted lines"}
        onClick={() => setExpanded((v) => !v)}
      >
        <span className="md-del-icon" aria-hidden="true" />
        {count} deleted {count === 1 ? "line" : "lines"}
      </button>
      {expanded && (
        <div className="md-del-lines">
          {anchor.lines.map((line, i) => (
            <div key={i} className="md-del-line">
              <span className="md-del-lineno">{line.oldLineno ?? ""}</span>
              <span className="md-del-content">{line.content}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
