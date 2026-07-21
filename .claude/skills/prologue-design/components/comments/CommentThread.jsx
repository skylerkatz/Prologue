import React from "react";
import { CommentCard } from "./CommentCard.jsx";
import { Button } from "../core/Button.jsx";

/**
 * Root comment + flat replies + trailing "\u21b3 Reply" action, wrapped in one
 * bordered card group. One level of nesting, indented once and no further.
 */
export function CommentThread({ root, replies = [], onReply, readOnly = false }) {
  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      <CommentCard {...root}>
        {(replies.length > 0 || (!readOnly && onReply)) && (
          <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: "0 12px 10px" }}>
            {replies.map((r) => <CommentCard key={r.id} {...r} isReply readOnly={readOnly} />)}
            {!readOnly && onReply && (
              <span>
                <Button variant="ghost" size="sm" onClick={onReply} style={{ color: "var(--text)", fontWeight: 600 }}>{"\u21b3"} Reply</Button>
              </span>
            )}
          </div>
        )}
      </CommentCard>
    </div>
  );
}
