import { memo, useCallback, useMemo, useState } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Element, Properties, Root, RootContent } from "hast";
import {
  blockTargetLine,
  classifyBlock,
  type DeletionAnchor,
  type FileMarkers,
} from "../diff/markers";
import { MarkdownLink } from "./MarkdownLink";

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
 *
 * Entirely-new files carry no markers (bars on a file where everything
 * is new would be noise) but still want a path from preview to a line
 * comment; `addedFile` annotates every block as a comment target instead.
 */
export const MarkdownPreview = memo(function MarkdownPreview({
  content,
  path,
  markers,
  addedFile = false,
  onJumpToSource,
}: {
  content: string;
  path: string;
  /** Null renders clean — added files, or hunks not (yet) loaded. */
  markers: FileMarkers | null;
  /**
   * Entirely-new file: there is no diff to intersect — the new side IS
   * the document — so every top-level block becomes a click-to-comment
   * target at its remark start line, with no change bars.
   */
  addedFile?: boolean;
  onJumpToSource: (path: string, newLine: number) => void;
}) {
  const rehypePlugins = useMemo(
    () =>
      markers !== null
        ? [() => markerTransform(markers)]
        : addedFile
          ? [addedFileTransform]
          : [],
    [markers, addedFile],
  );

  // Tombstone divs planted by the transform become interactive markers;
  // every other div (none, normally — raw HTML is stripped) passes through.
  const components = useMemo<Components>(
    () => ({
      a: MarkdownLink,
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
      const line = previewClickTarget(
        event.target as HTMLElement,
        window.getSelection(),
      );
      if (line !== null) {
        onJumpToSource(path, line);
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
 * Resolve a delegated preview click to the new-side line it should jump
 * to, or null when the click must not navigate: mid-copy (finishing a
 * selection drag fires a click on the block it ended over — jumping away
 * would eat the selection), on an interactive element, or outside any
 * annotated block. Factored out of the component so the gesture stays
 * testable without a DOM environment (tests drive it with stub elements).
 */
export function previewClickTarget(
  target: HTMLElement,
  selection: Selection | null,
): number | null {
  if (selection !== null && !selection.isCollapsed) {
    return null;
  }
  if (target.closest("a, button, input, .md-del-marker") !== null) {
    return null;
  }
  const block = target.closest("[data-md-line]");
  if (block === null) {
    return null;
  }
  const line = Number(block.getAttribute("data-md-line"));
  return Number.isFinite(line) ? line : null;
}

/** Append a class through hast's polymorphic className property. */
function appendClass(props: Properties, name: string): void {
  const existing = props.className;
  props.className = Array.isArray(existing)
    ? [...existing, name]
    : existing !== undefined && existing !== null
      ? [String(existing), name]
      : [name];
}

/**
 * The added-file counterpart of markerTransform: with no diff to
 * intersect (the new side IS the document), remark's position data alone
 * maps each top-level block to its source start line. Every block becomes
 * a comment target through the same data attribute the delegated click
 * handler already reads — but no change classes; the commentable class
 * only carries the pointer/tooltip affordance.
 */
function addedFileTransform(): (tree: Root) => void {
  return (tree) => {
    for (const child of tree.children) {
      if (child.type !== "element" || child.position === undefined) {
        continue;
      }
      const props = (child.properties ??= {});
      appendClass(props, "md-block-commentable");
      props.dataMdLine = String(child.position.start.line);
      props.title = "Click to comment on this line in the source diff";
    }
  };
}

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
        appendClass(props, `md-block-${mark}`);
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
