import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { MarkdownPreview, previewClickTarget } from "./MarkdownPreview";
import type { FileMarkers } from "../diff/markers";

// Renders the real react-markdown pipeline, so these tests pin the parts
// that plain unit tests can't: block positions surviving into hast, the
// class/data-attribute annotation, and tombstone placement in the flow.

const doc = [
  "# Title", // line 1
  "",
  "First paragraph.", // line 3
  "",
  "Second paragraph line one.", // line 5
  "second paragraph line two.", // line 6
  "",
  "Third paragraph.", // line 8
].join("\n");

const noop = () => {};

function render(markers: FileMarkers | null): string {
  return renderToStaticMarkup(
    <MarkdownPreview
      content={doc}
      path="README.md"
      markers={markers}
      onJumpToSource={noop}
    />,
  );
}

describe("MarkdownPreview markers", () => {
  it("renders clean without markers", () => {
    const html = render(null);
    expect(html).toContain("<h1>Title</h1>");
    expect(html).not.toContain("md-block-");
    expect(html).not.toContain("md-del-marker");
  });

  it("marks a fully added block green and a partial one amber", () => {
    const html = render({
      added: [
        { start: 3, end: 3 },
        { start: 6, end: 6 },
      ],
      deletions: [],
    });
    expect(html).toContain(
      '<p class="md-block-added" data-md-line="3"',
    );
    expect(html).toContain(
      '<p class="md-block-modified" data-md-line="6"',
    );
    // Untouched blocks stay unmarked.
    expect(html).toContain("<h1>Title</h1>");
    expect(html).toContain("<p>Third paragraph.</p>");
  });

  it("plants a click-to-peek tombstone at the deletion anchor", () => {
    const html = render({
      added: [],
      deletions: [
        {
          afterLine: 5,
          lines: [
            { oldLineno: 6, content: "old line a" },
            { oldLineno: 7, content: "old line b" },
          ],
        },
      ],
    });
    // A deletion inside the second paragraph marks it modified and puts
    // the tombstone directly after it, before the third paragraph.
    const block = html.indexOf("md-block-modified");
    const marker = html.indexOf("md-del-marker");
    const third = html.indexOf("Third paragraph.");
    expect(block).toBeGreaterThan(-1);
    expect(marker).toBeGreaterThan(block);
    expect(third).toBeGreaterThan(marker);
    expect(html).toContain("2 deleted lines");
    // Collapsed by default: the deleted content isn't in the DOM yet.
    expect(html).not.toContain("old line a");
  });

  it("puts a between-blocks deletion between the blocks", () => {
    const html = render({
      added: [],
      deletions: [{ afterLine: 3, lines: [{ oldLineno: 4, content: "x" }] }],
    });
    // Deletion after the first paragraph's last line: between it and the
    // second paragraph, with neither block marked.
    expect(html).not.toContain("md-block-");
    const first = html.indexOf("First paragraph.");
    const marker = html.indexOf("md-del-marker");
    const second = html.indexOf("Second paragraph");
    expect(marker).toBeGreaterThan(first);
    expect(second).toBeGreaterThan(marker);
    expect(html).toContain("1 deleted line<");
  });

  it("puts a start-of-file deletion before the first block", () => {
    const html = render({
      added: [],
      deletions: [{ afterLine: 0, lines: [{ oldLineno: 1, content: "x" }] }],
    });
    const marker = html.indexOf("md-del-marker");
    const title = html.indexOf("Title");
    expect(marker).toBeGreaterThan(-1);
    expect(title).toBeGreaterThan(marker);
  });

  it("puts an end-of-file deletion after the last block", () => {
    const html = render({
      added: [],
      deletions: [{ afterLine: 8, lines: [{ oldLineno: 9, content: "x" }] }],
    });
    const third = html.indexOf("Third paragraph.");
    const marker = html.indexOf("md-del-marker");
    expect(marker).toBeGreaterThan(third);
  });
});

describe("MarkdownPreview added files", () => {
  function renderAdded(): string {
    return renderToStaticMarkup(
      <MarkdownPreview
        content={doc}
        path="README.md"
        markers={null}
        addedFile
        onJumpToSource={noop}
      />,
    );
  }

  it("annotates every top-level block with its start line", () => {
    const html = renderAdded();
    expect(html).toContain('data-md-line="1"'); // heading
    expect(html).toContain('data-md-line="3"'); // first paragraph
    expect(html).toContain('data-md-line="5"'); // two-line paragraph: start
    expect(html).toContain('data-md-line="8"'); // third paragraph
  });

  it("carries the pointer/tooltip affordance without change styling", () => {
    const html = renderAdded();
    expect(html).toContain("md-block-commentable");
    expect(html).toContain(
      "Click to comment on this line in the source diff",
    );
    expect(html).not.toContain("md-block-added");
    expect(html).not.toContain("md-block-modified");
    expect(html).not.toContain("md-del-marker");
  });

  it("stays inert without the added-file flag", () => {
    // markers=null alone is a changed file whose diff hasn't landed yet —
    // nothing to jump to, so nothing gets annotated.
    const html = render(null);
    expect(html).not.toContain("data-md-line");
    expect(html).not.toContain("md-block-commentable");
  });
});

// No DOM test environment here (same as MarkdownLink.test.tsx), so the
// click gesture is pinned through the extracted resolver, with stub
// elements standing in for the event target.
describe("previewClickTarget", () => {
  function stubTarget({
    control = false,
    line = "5" as string | null,
  } = {}): HTMLElement {
    const block =
      line === null
        ? null
        : ({ getAttribute: () => line } as unknown as Element);
    return {
      closest: (selector: string) =>
        selector.includes("data-md-line")
          ? block
          : control
            ? ({} as Element)
            : null,
    } as unknown as HTMLElement;
  }
  const collapsed = { isCollapsed: true } as Selection;
  const dragged = { isCollapsed: false } as Selection;

  it("resolves an annotated block's line on a plain click", () => {
    expect(previewClickTarget(stubTarget(), collapsed)).toBe(5);
    // A browser with no selection object at all still jumps.
    expect(previewClickTarget(stubTarget(), null)).toBe(5);
  });

  it("ignores the click that ends a selection drag", () => {
    expect(previewClickTarget(stubTarget(), dragged)).toBeNull();
  });

  it("leaves interactive elements to their own handlers", () => {
    expect(
      previewClickTarget(stubTarget({ control: true }), collapsed),
    ).toBeNull();
  });

  it("ignores clicks outside any annotated block", () => {
    expect(previewClickTarget(stubTarget({ line: null }), collapsed)).toBeNull();
  });

  it("rejects a non-numeric line annotation", () => {
    expect(
      previewClickTarget(stubTarget({ line: "bogus" }), collapsed),
    ).toBeNull();
  });
});

describe("MarkdownPreview links", () => {
  // Behavior details live in MarkdownLink.test.tsx; this pins that the
  // preview actually routes anchors through the shared wrapper.
  it("uses the shared link wrapper", () => {
    const html = renderToStaticMarkup(
      <MarkdownPreview
        content="[site](https://example.com) and [docs](./docs.md)"
        path="README.md"
        markers={null}
        onJumpToSource={noop}
      />,
    );
    expect(html).toContain('<a href="https://example.com">site</a>');
    expect(html).toContain(
      '<span class="md-link-inert" title="./docs.md">docs</span>',
    );
  });
});
