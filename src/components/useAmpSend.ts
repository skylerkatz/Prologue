import {
  createContext,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { isTauri } from "@tauri-apps/api/core";
import {
  ampSends,
  confirmAmpSend,
  sendToAmp,
  type AmpSendRecord,
} from "../ipc";
import { webConfig } from "../web";
import { useTauriEvent } from "../useTauriEvent";
import { COMMENTS_CHANGED_EVENT } from "../generated/events";
import type { Comment } from "../types";
import type { ExportTarget } from "./ExportButton";

/** Root comment ids by Send to Amp state, for each card's chip; null when
 * Send to Amp is off (the app, or a server without a webhook). */
export interface AmpCommentStates {
  /** Confirmed by the Amp thread. */
  sent: ReadonlySet<number>;
  /** Accepted by the webhook, waiting for the thread to confirm. */
  sending: ReadonlySet<number>;
}

export const AmpSentContext = createContext<AmpCommentStates | null>(null);

/** A send that did not reach the thread (refused, unreachable, or never
 * confirmed) whose comments are still unsent: the reviewer can paste its
 * message into the thread by hand. */
export interface FailedSend {
  sendId: string;
  handoff: string;
}

export interface AmpSend extends AmpCommentStates {
  /** Open thread roots neither sent nor in flight. */
  pendingCount: number;
  /** Roots waiting for the thread's confirmation. */
  sendingCount: number;
  /** Send the pending comments; resolves with the confirmation to show. */
  send: () => Promise<string>;
  /** The newest failed send still worth finishing by hand, if any. */
  failed: FailedSend | null;
  /** "I pasted it": mark the failed send's comments sent. */
  confirmManually: (sendId: string) => Promise<void>;
  /** Hide that failed send's fallback in this view. */
  dismiss: (sendId: string) => void;
}

/**
 * Send to Amp (web mode with PROLOGUE_AMP_WEBHOOK_URL set on the server):
 * the server POSTs the open, unsent comment ids to the webhook. A send is
 * `sending` until the Amp thread confirms it (`prologue confirm-send`), and
 * reverts to unsent if refused or unconfirmed. Every state change arrives
 * as `comments-changed`, so all viewers follow. Null when the feature is off.
 */
export function useAmpSend(
  target: ExportTarget | null,
  comments: Comment[],
): AmpSend | null {
  const [enabled, setEnabled] = useState(false);
  const [records, setRecords] = useState<AmpSendRecord[]>([]);
  // Dismissing a fallback hides it and every older failed send (they all
  // name comments the reviewer has decided about); newer failures show.
  const [dismissedUntil, setDismissedUntil] = useState("");
  const reviewId = target?.reviewId ?? null;

  useEffect(() => {
    if (!isTauri()) {
      webConfig()
        .then((config) => setEnabled(config.ampWebhook))
        .catch(() => setEnabled(false));
    }
  }, []);

  const [version, setVersion] = useState(0);
  useTauriEvent(COMMENTS_CHANGED_EVENT, () => setVersion((v) => v + 1));

  useEffect(() => {
    setRecords([]);
  }, [enabled, reviewId]);

  useEffect(() => {
    if (!enabled || reviewId === null) {
      return;
    }
    let cancelled = false;
    ampSends(reviewId)
      .then((next) => !cancelled && setRecords(next))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [enabled, reviewId, version]);

  const { sent, sending } = useMemo(() => {
    const ids = (state: AmpSendRecord["state"]) =>
      new Set(
        records.filter((r) => r.state === state).flatMap((r) => r.commentIds),
      );
    return { sent: ids("sent"), sending: ids("sending") };
  }, [records]);

  const { pendingCount, sendingCount } = useMemo(() => {
    const roots = comments.filter(
      (c) => c.parentId === null && c.state === "open" && !sent.has(c.id),
    );
    const inFlightCount = roots.filter((c) => sending.has(c.id)).length;
    return {
      pendingCount: roots.length - inFlightCount,
      sendingCount: inFlightCount,
    };
  }, [comments, sent, sending]);

  const failed = useMemo((): FailedSend | null => {
    const unsent = new Set(
      comments
        .filter(
          (c) =>
            c.parentId === null &&
            c.state === "open" &&
            !sent.has(c.id) &&
            !sending.has(c.id),
        )
        .map((c) => c.id),
    );
    const last = [...records]
      .reverse()
      .find(
        (r) =>
          r.state === "failed" &&
          r.handoff !== undefined &&
          r.createdAt > dismissedUntil &&
          r.commentIds.some((id) => unsent.has(id)),
      );
    return last === undefined
      ? null
      : { sendId: last.sendId, handoff: last.handoff! };
  }, [records, comments, sent, sending, dismissedUntil]);

  const confirmManually = useCallback(async (sendId: string) => {
    await confirmAmpSend(sendId);
    setVersion((v) => v + 1);
  }, []);

  const dismiss = useCallback(
    (sendId: string) => {
      const at = records.find((r) => r.sendId === sendId)?.createdAt;
      if (at !== undefined) {
        setDismissedUntil((prev) => (at > prev ? at : prev));
      }
    },
    [records],
  );

  const send = useCallback(async () => {
    if (target === null) {
      throw "Sending needs an active review";
    }
    const result = await sendToAmp(
      target.repoPath,
      target.base,
      target.reviewId,
    );
    setVersion((v) => v + 1);
    return `Sent ${result.count} comment${result.count === 1 ? "" : "s"} — waiting for the Amp thread to confirm`;
  }, [target]);

  return enabled
    ? {
        sent,
        sending,
        pendingCount,
        sendingCount,
        send,
        failed,
        confirmManually,
        dismiss,
      }
    : null;
}
