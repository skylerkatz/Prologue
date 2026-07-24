import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import Markdown from "react-markdown";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  classifyHref,
  MarkdownLink,
  markdownLinkComponents,
} from "./MarkdownLink";

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn(() => Promise.resolve()),
}));

const openUrlMock = vi.mocked(openUrl);

beforeEach(() => {
  openUrlMock.mockClear();
});

describe("classifyHref", () => {
  it("routes openable urls externally", () => {
    expect(classifyHref("https://example.com")).toBe("external");
    expect(classifyHref("http://example.com")).toBe("external");
    expect(classifyHref("HTTPS://EXAMPLE.COM")).toBe("external");
    expect(classifyHref("mailto:a@b.com")).toBe("external");
    expect(classifyHref("tel:+15551234567")).toBe("external");
  });

  it("keeps fragments in-page", () => {
    expect(classifyHref("#section")).toBe("fragment");
  });

  it("treats everything else as inert", () => {
    expect(classifyHref(undefined)).toBe("inert");
    expect(classifyHref("")).toBe("inert");
    expect(classifyHref("./docs/setup.md")).toBe("inert");
    expect(classifyHref("docs/setup.md")).toBe("inert");
    expect(classifyHref("/absolute/path")).toBe("inert");
    expect(classifyHref("javascript:alert(1)")).toBe("inert");
    expect(classifyHref("file:///etc/passwd")).toBe("inert");
  });
});

describe("MarkdownLink rendering", () => {
  function render(md: string): string {
    return renderToStaticMarkup(
      <Markdown components={markdownLinkComponents}>{md}</Markdown>,
    );
  }

  it("renders external links as anchors with their href", () => {
    const html = render("[site](https://example.com)");
    expect(html).toContain('<a href="https://example.com">site</a>');
  });

  it("renders fragment links as plain anchors", () => {
    const html = render("[jump](#section)");
    expect(html).toContain('<a href="#section">jump</a>');
  });

  it("renders relative links as inert text with the target in the tooltip", () => {
    const html = render("[docs](./docs/setup.md)");
    expect(html).not.toContain("<a");
    expect(html).toContain(
      '<span class="md-link-inert" title="./docs/setup.md">docs</span>',
    );
  });

  it("keeps an author-supplied title on inert links", () => {
    const html = render('[docs](./setup.md "read me")');
    expect(html).toContain('title="read me"');
  });
});

// No DOM test environment here, so gesture behavior is pinned by calling
// the component as a function and driving the handlers it attaches.
describe("MarkdownLink gestures", () => {
  function externalProps() {
    const element = MarkdownLink({
      href: "https://example.com",
      children: "site",
    });
    return element.props as {
      onClick: (e: unknown) => void;
      onAuxClick: (e: { preventDefault: () => void; button: number }) => void;
      onContextMenu: (e: unknown) => void;
    };
  }

  function fakeEvent(button = 0) {
    return { preventDefault: vi.fn(), button };
  }

  it("opens externally on click and blocks navigation", () => {
    const event = fakeEvent();
    externalProps().onClick(event);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(openUrlMock).toHaveBeenCalledWith("https://example.com");
  });

  it("opens externally on middle-click", () => {
    const event = fakeEvent(1);
    externalProps().onAuxClick(event);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(openUrlMock).toHaveBeenCalledWith("https://example.com");
  });

  it("leaves right-button auxclick to the contextmenu handler", () => {
    externalProps().onAuxClick(fakeEvent(2));
    expect(openUrlMock).not.toHaveBeenCalled();
  });

  it("opens externally on right-click instead of the dead context menu", () => {
    const event = fakeEvent(2);
    externalProps().onContextMenu(event);
    expect(event.preventDefault).toHaveBeenCalled();
    expect(openUrlMock).toHaveBeenCalledWith("https://example.com");
  });

  it("attaches no interception to fragment links", () => {
    const element = MarkdownLink({ href: "#section", children: "jump" });
    const props = element.props as Record<string, unknown>;
    expect(props.onClick).toBeUndefined();
    expect(props.onAuxClick).toBeUndefined();
    expect(props.onContextMenu).toBeUndefined();
  });
});
