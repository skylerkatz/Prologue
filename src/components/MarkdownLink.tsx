import type { ComponentPropsWithoutRef, SyntheticEvent } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Components } from "react-markdown";
import type { Element } from "hast";

/**
 * Shared `a` override for every react-markdown surface (file previews,
 * release notes). Links must never navigate the chrome-less window — no
 * address bar, no back button, so a stray navigation would soft-brick
 * it. The Rust side backs this up with an on_navigation guard pinned to
 * the app origin, but that guard kills navigations *silently*; this
 * wrapper's job is to give every link gesture a sensible outcome first:
 *
 * - Openable urls (http, https, mailto, tel — the opener:default scope)
 *   go to the system handler via openUrl() on any gesture: click (with
 *   or without cmd/ctrl), middle-click, and right-click. The webview's
 *   context menu is suppressed because everything useful on it ("Open
 *   Link", "Open in New Window") would hit the navigation guard and die.
 * - Fragment links (#anchor) keep native behavior: same-document, same
 *   origin, so the guard permits them; they scroll if the target exists.
 * - Anything else (relative paths, unknown schemes) has no meaningful
 *   target in this window, so it renders as plain text — no dead link
 *   affordance — with the raw href surfaced in the tooltip.
 */
export type LinkKind = "external" | "fragment" | "inert";

export function classifyHref(href: string | undefined): LinkKind {
  if (href === undefined || href === "") {
    return "inert";
  }
  if (/^(https?:\/\/|mailto:|tel:)/i.test(href)) {
    return "external";
  }
  if (href.startsWith("#")) {
    return "fragment";
  }
  return "inert";
}

export function MarkdownLink(
  props: ComponentPropsWithoutRef<"a"> & { node?: Element },
) {
  const { node: _node, href, children, ...rest } = props;
  const kind = classifyHref(href);

  if (kind === "inert") {
    const { title, ...spanRest } = rest;
    return (
      <span {...spanRest} className="md-link-inert" title={title ?? href}>
        {children}
      </span>
    );
  }

  if (kind === "fragment") {
    return (
      <a {...rest} href={href}>
        {children}
      </a>
    );
  }

  const open = (event: SyntheticEvent) => {
    event.preventDefault();
    if (href !== undefined) {
      void openUrl(href);
    }
  };
  return (
    <a
      {...rest}
      href={href}
      onClick={open}
      // Middle-click only: right-click fires auxclick too, but its open
      // belongs to the contextmenu handler below (else it opens twice).
      onAuxClick={(event) => {
        if (event.button === 1) {
          open(event);
        }
      }}
      onContextMenu={open}
    >
      {children}
    </a>
  );
}

/** Drop-in `components` for markdown surfaces that only need safe links. */
export const markdownLinkComponents: Components = { a: MarkdownLink };
