import { createContext, useCallback, useEffect, useMemo, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { ampSentComments, sendToAmp } from "../ipc";
import { webConfig } from "../web";
import type { Comment } from "../types";
import type { ExportTarget } from "./ExportButton";

/**
 * Ids of root comments already sent to Amp, for the "pending" badge; null
 * when Send to Amp is off (the app, or a server without a webhook).
 */
export const AmpSentContext = createContext<ReadonlySet<number> | null>(null);

export interface AmpSend {
  sent: ReadonlySet<number>;
  /** Open thread roots not yet sent. */
  pendingCount: number;
  /** Send the pending comments; resolves with the confirmation to show. */
  send: () => Promise<string>;
}

/**
 * Send to Amp (web mode with PROLOGUE_AMP_WEBHOOK_URL set on the server):
 * the server POSTs the open, unsent comment ids to the webhook. Null when
 * the feature is off, so callers fall back to the plain export.
 */
export function useAmpSend(
  target: ExportTarget | null,
  comments: Comment[],
): AmpSend | null {
  const [enabled, setEnabled] = useState(false);
  const [sent, setSent] = useState<ReadonlySet<number>>(new Set());
  const reviewId = target?.reviewId ?? null;

  useEffect(() => {
    if (!isTauri()) {
      webConfig()
        .then((config) => setEnabled(config.ampWebhook))
        .catch(() => setEnabled(false));
    }
  }, []);

  useEffect(() => {
    setSent(new Set());
    if (!enabled || reviewId === null) {
      return;
    }
    let cancelled = false;
    ampSentComments(reviewId)
      .then((ids) => !cancelled && setSent(new Set(ids)))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [enabled, reviewId]);

  const pendingCount = useMemo(
    () =>
      comments.filter(
        (c) => c.parentId === null && c.state === "open" && !sent.has(c.id),
      ).length,
    [comments, sent],
  );

  const send = useCallback(async () => {
    if (target === null) {
      throw "Sending needs an active review";
    }
    const result = await sendToAmp(target.repoPath, target.base, target.reviewId);
    setSent(
      (prev) =>
        new Set([...prev, ...result.commentIds.map((id) => Number(id.slice(1)))]),
    );
    return `Sent ${result.count} comment${result.count === 1 ? "" : "s"} to Amp`;
  }, [target]);

  return enabled ? { sent, pendingCount, send } : null;
}
