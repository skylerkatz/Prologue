import { useState } from "react";
import { errorText, requestPluginUpdate } from "../../ipc";
import { writeClipboard } from "../../platform";
import type { PluginUpdate } from "../../web";

/** localStorage key: the plugin version whose update notice was dismissed,
 * so the next version asks again. */
const DISMISSED_KEY = "prologue.pluginUpdateDismissed";

type Status =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "sent" }
  | { kind: "copied" }
  | { kind: "failed"; error: string };

/**
 * Web mode: the Amp plugin that started this server is older than the one
 * this release was built with. A current plugin takes the update request
 * through its webhook and adds the prompt to the thread itself; an older
 * one can't, so the prompt is copied for the reviewer to paste.
 */
export function PluginUpdateBanner({ update }: { update: PluginUpdate }) {
  const [dismissed, setDismissed] = useState(
    () => localStorage.getItem(DISMISSED_KEY) === String(update.latest),
  );
  const [status, setStatus] = useState<Status>({ kind: "idle" });

  if (dismissed) {
    return null;
  }

  function dismiss() {
    localStorage.setItem(DISMISSED_KEY, String(update.latest));
    setDismissed(true);
  }

  async function sendToThread() {
    setStatus({ kind: "sending" });
    try {
      await requestPluginUpdate();
      setStatus({ kind: "sent" });
    } catch (e) {
      setStatus({ kind: "failed", error: errorText(e) });
    }
  }

  async function copyPrompt() {
    try {
      await writeClipboard(update.prompt);
      setStatus({ kind: "copied" });
    } catch (e) {
      setStatus({ kind: "failed", error: errorText(e) });
    }
  }

  const text =
    status.kind === "sent"
      ? "Asked Amp to update the plugin in this thread."
      : status.kind === "copied"
        ? "Copied. Paste it into this thread and Amp will update the plugin."
        : status.kind === "failed"
          ? `Couldn't reach the thread: ${status.error}`
          : "A newer version of the Prologue Amp plugin is available.";
  // A failed request falls back to the prompt to paste.
  const direct = update.direct && status.kind !== "failed";

  return (
    <div className="update-banner plugin-update-banner" role="status">
      <span className="update-banner-text">{text}</span>
      {status.kind !== "sent" &&
        status.kind !== "copied" &&
        (direct ? (
          <button
            className="refresh-button"
            onClick={() => void sendToThread()}
            disabled={status.kind === "sending"}
          >
            {status.kind === "sending" ? "Sending…" : "Update with Amp"}
          </button>
        ) : (
          <button className="refresh-button" onClick={() => void copyPrompt()}>
            Copy update prompt
          </button>
        ))}
      <button
        className="update-banner-dismiss"
        onClick={dismiss}
        aria-label="Dismiss plugin update notice"
        title="Hide until the next plugin version"
      >
        ×
      </button>
    </div>
  );
}
