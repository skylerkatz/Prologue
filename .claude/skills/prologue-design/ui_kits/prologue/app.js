/* Prologue UI kit — interactive recreation of the mockup.
   Welcome → Review; export menu + toast; archived overlay; comment threads. */
const { useState } = React;

/* ---------- primitives (cosmetic twins of components/) ---------- */
const MONO = "var(--font-mono)";
function Btn({ v = "secondary", sm, disabled, onClick, title, style, children }) {
  const [h, setH] = useState(false);
  const base = {
    primary: { background: h && !disabled ? "var(--accent-strong)" : "var(--accent)", color: "#fff", border: "1px solid transparent", fontWeight: 600 },
    secondary: { background: "var(--bg-card)", color: "var(--text)", border: `1px solid ${h && !disabled ? "var(--accent)" : "var(--border)"}`, fontWeight: 500 },
    outline: { background: "var(--bg-card)", color: "var(--accent-strong)", border: `1px solid ${h && !disabled ? "var(--accent)" : "var(--border)"}`, fontWeight: 600 },
    ghost: { background: h && !disabled ? "var(--orange-50)" : "transparent", color: h && !disabled ? "var(--accent-strong)" : "var(--text-muted)", border: "1px solid transparent", fontWeight: 500 },
    danger: { background: "transparent", color: "var(--danger)", border: "1px solid transparent", fontWeight: 500 },
  }[v];
  return (
    <button type="button" title={title} disabled={disabled} onClick={onClick}
      onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{ padding: sm ? "2px 9px" : "5px 13px", fontSize: sm ? 12 : 13, borderRadius: 6, cursor: disabled ? "default" : "pointer", whiteSpace: "nowrap", opacity: disabled ? 0.55 : 1, lineHeight: 1.45, ...base, ...style }}>
      {children}
    </button>
  );
}
function StatusBadge({ s = "modified" }) {
  const m = { added: ["A", "--badge-added-bg", "--badge-added-text"], modified: ["M", "--badge-modified-bg", "--badge-modified-text"], deleted: ["D", "--badge-deleted-bg", "--badge-deleted-text"], renamed: ["R", "--badge-renamed-bg", "--badge-renamed-text"] }[s];
  return <span style={{ flex: "none", width: 20, height: 20, display: "inline-flex", alignItems: "center", justifyContent: "center", borderRadius: 4, background: `var(${m[1]})`, color: `var(${m[2]})`, fontFamily: MONO, fontSize: 11.5, fontWeight: 700 }}>{m[0]}</span>;
}
function Counts({ a, d, size = 12.5 }) {
  return <span style={{ display: "inline-flex", gap: 8, fontFamily: MONO, fontSize: size, fontWeight: 600, flex: "none" }}><span style={{ color: "var(--added)" }}>+{a}</span><span style={{ color: "var(--removed)" }}>−{d}</span></span>;
}
function Pill({ n }) {
  return <span style={{ flex: "none", minWidth: 19, height: 19, padding: "0 5px", display: "inline-flex", alignItems: "center", justifyContent: "center", borderRadius: 999, background: "var(--teal-800)", color: "var(--text-on-dark)", fontFamily: MONO, fontSize: 11, fontWeight: 700 }}>{n}</span>;
}
function Avatar({ id, size = 26 }) {
  return <span style={{ flex: "none", width: size, height: size, borderRadius: 999, display: "inline-flex", alignItems: "center", justifyContent: "center", background: "var(--teal-800)", color: "var(--text-on-dark)", fontFamily: MONO, fontSize: size * 0.42, fontWeight: 700 }}>C{id}</span>;
}
function Select({ label, value }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      {label && <span style={{ color: "var(--text-muted)", fontSize: 12 }}>{label}</span>}
      <span style={{ display: "inline-flex", alignItems: "center", gap: 8, background: "var(--bg-card)", border: "1px solid var(--border-strong)", borderRadius: 6, padding: "4px 10px", fontFamily: MONO, fontSize: 13, cursor: "pointer", whiteSpace: "nowrap" }}>
        {value} <span style={{ fontSize: 9, color: "var(--text-muted)" }}>▾</span>
      </span>
    </span>
  );
}
function Composer({ placeholder, submitLabel = "Comment", onSubmit, onCancel }) {
  const [text, setText] = useState("");
  const [focus, setFocus] = useState(false);
  return (
    <div style={{ padding: "8px 10px 10px" }}>
      <textarea rows={3} placeholder={placeholder} value={text} autoFocus
        onFocus={() => setFocus(true)} onBlur={() => setFocus(false)}
        onChange={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && text.trim()) { onSubmit(text); } if (e.key === "Escape") onCancel(); }}
        style={{ width: "100%", resize: "vertical", font: "inherit", fontSize: 13, color: "var(--text)", background: "var(--bg-card)", border: `1px solid ${focus ? "var(--accent)" : "var(--border-strong)"}`, outline: "none", borderRadius: 6, padding: "7px 9px" }} />
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 7 }}>
        <Btn v="primary" sm disabled={!text.trim()} onClick={() => onSubmit(text)}>{submitLabel}</Btn>
        <Btn sm onClick={onCancel}>Cancel</Btn>
        <span style={{ marginLeft: "auto", fontSize: 11.5, color: "var(--text-muted)" }}>⌘↩ to submit</span>
      </div>
    </div>
  );
}

/* ---------- comment thread ---------- */
function CommentCard({ c, isReply, readOnly, onResolve, onDelete }) {
  const closed = c.state && c.state !== "open";
  return (
    <article style={{ background: isReply ? "var(--bg-subtle)" : "var(--bg-card)", border: "1px solid var(--border)", borderLeft: isReply ? "1px solid var(--border)" : "3px solid var(--comment-accent-border)", borderRadius: 8, marginLeft: isReply ? 24 : 0, opacity: closed ? 0.7 : 1 }}>
      <header style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 12px 0" }}>
        <Avatar id={c.id} size={isReply ? 22 : 26} />
        <span style={{ fontWeight: 700, color: "var(--text-strong)" }}>You</span>
        {c.location && <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{c.location}</span>}
        <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{c.time}</span>
        {closed && <span style={{ fontSize: 11, fontWeight: 600, padding: "1px 8px", borderRadius: 999, border: `1px solid ${c.state === "resolved" ? "var(--added)" : "var(--border-strong)"}`, color: c.state === "resolved" ? "var(--added)" : "var(--text-muted)", background: "var(--bg-card)" }}>{c.state === "resolved" ? "Resolved" : "Dismissed"}</span>}
        <span style={{ flex: 1 }} />
        {!readOnly && !closed && (
          <span style={{ display: "flex", gap: 2 }}>
            {!isReply && onResolve && <Btn v="ghost" sm title="Handled — a change was made or the new code is fine" onClick={onResolve}>Resolve</Btn>}
            <Btn v="ghost" sm>Edit</Btn>
            <Btn v="ghost" sm onClick={onDelete}>Delete</Btn>
          </span>
        )}
      </header>
      <p style={{ margin: 0, padding: "6px 12px 12px", fontSize: 13.5, lineHeight: 1.5, whiteSpace: "pre-wrap", userSelect: "text", cursor: "text" }}>{c.body}</p>
    </article>
  );
}
function Thread({ c, replies = [], readOnly, onResolve, onDelete, onReply }) {
  const [composing, setComposing] = useState(false);
  return (
    <div>
      <CommentCard c={c} readOnly={readOnly} onResolve={onResolve} onDelete={onDelete} />
      {(replies.length > 0 || (!readOnly && c.state === "open")) && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8, margin: "8px 0 0 0" }}>
          {replies.map((r) => <CommentCard key={r.key} c={r} isReply readOnly={readOnly} />)}
          {!readOnly && c.state === "open" && (composing ? (
            <div style={{ marginLeft: 24, background: "var(--bg-card)", border: "1px solid var(--border)", borderRadius: 8 }}>
              <Composer placeholder={`Reply to C${c.id}…`} submitLabel="Reply" onSubmit={(t) => { onReply(t); setComposing(false); }} onCancel={() => setComposing(false)} />
            </div>
          ) : (
            <span style={{ marginLeft: 24 }}><Btn v="ghost" sm style={{ color: "var(--text)", fontWeight: 600 }} onClick={() => setComposing(true)}>↳ Reply</Btn></span>
          ))}
        </div>
      )}
    </div>
  );
}

/* ---------- diff data ---------- */
const FILE1 = {
  path: "app/Http/Controllers/My/Billing/BillingController.php", short: "…llingController.php", a: 3, d: 1,
  hunks: [
    { header: "@@ -60,7 +60,8 @@ class BillingController extends Controller", above: { count: 59 },
      lines: [
        { o: 61, n: 61, code: "$active_subscription = $target_user->personalCompany()->active_subscription;" },
        { o: 62, n: 62, code: "$upcoming_subscription = $target_user->personalCompany()->latest_subscription()->where('starts_at', '>=', …)->…" },
        { k: "del", o: 63, code: "$is_non_billable_user = $target_user->isStaff() || $target_user->isLifetime() || $target_user->isRetired();" },
        { k: "add", n: 63, code: "$lifetime_but_billable = $target_user->isLifetime() && $billable_company->isMultiInspector() && $billable_comp…" },
        { k: "add", n: 64, code: "$is_non_billable_user = $target_user->isStaff() || ($target_user->isLifetime() && ! $lifetime_but_billable) ||…" },
        { o: 64, n: 65, code: "// Intentionally currentCompany(): this gates the consolidation flow, which" },
        { o: 65, n: 66, code: "// converts the managed company — not the company that bills the user." },
      ] },
    { header: "@@ -93,6 +94,7 @@ class BillingController extends Controller",
      lines: [
        { o: 95, n: 96, code: "'plan_lock_in' => $target_user->personalCompany()->plan_lock_in," },
        { k: "add", n: 97, code: "'is_lifetime_but_billable' => $lifetime_but_billable," },
        { o: 96, n: 98, code: "'is_non_billable_user' => $is_non_billable_user,", comment: true },
      ],
      below: { count: 21 } },
  ],
};
const FILE2 = {
  path: "resources/views/my/billing/partials/_management-options.blade.php", short: "…ement-options.blade.php", a: 2, d: 2,
  hunks: [
    { header: "@@ -4,7 +4,7 @@",
      lines: [
        { o: 5, n: 5, code: "</a>" },
        { k: "del", o: 7, code: "@unless($active_subscription->isAuthorizeNet() || $active_subscription->isStripe())" },
        { k: "add", n: 7, code: "@unless($active_subscription->isAuthorizeNet() || $active_subscription->isStripe() || $target_user->billableCompan…" },
        { o: 8, n: 8, code: "<a href=\"{{ route('my.billing.subscription.create') }}\" type=\"button\" class=\"mx-1 btn btn-sm btn-blue\">" },
        { k: "del", o: 9, ws: true, code: "  Switch to Online Billing" },
        { k: "add", n: 9, ws: true, code: "    Switch to Online Billing" },
      ] },
  ],
};
const FILES = [FILE1, FILE2, { path: "resources/views/my/billing/partials/_overview.blade.php", short: "…als/_overview.blade.php", a: 3, d: 1, hunks: [] }];

/* ---------- diff rendering ---------- */
function DiffLine({ line, selected, onGutter }) {
  const k = line.k;
  const bg = selected ? "var(--line-selected-bg)" : k === "add" ? "var(--diff-add-bg)" : k === "del" ? "var(--diff-del-bg)" : "var(--bg-card)";
  const gut = selected ? { background: "var(--accent)", color: "#fff" } : k === "add" ? { background: "var(--diff-add-gutter)" } : k === "del" ? { background: "var(--diff-del-gutter)" } : {};
  return (
    <div style={{ display: "grid", gridTemplateColumns: "52px 52px 18px 1fr", fontFamily: MONO, fontSize: 12, lineHeight: "21px", background: bg }}>
      <span onClick={onGutter} style={{ textAlign: "right", paddingRight: 8, color: "var(--text-muted)", cursor: "pointer", userSelect: "none", ...gut }}>{line.o ?? ""}</span>
      <span onClick={onGutter} style={{ textAlign: "right", paddingRight: 8, color: "var(--text-muted)", cursor: "pointer", userSelect: "none", ...gut }}>{line.n ?? ""}</span>
      <span style={{ textAlign: "center", color: k === "add" ? "var(--added)" : k === "del" ? "var(--removed)" : "var(--text-muted)" }}>{k === "add" ? "+" : k === "del" ? "−" : ""}</span>
      <span style={{ whiteSpace: "pre", overflow: "hidden", textOverflow: "ellipsis", paddingRight: 12, userSelect: "text", cursor: "text" }}>{line.code}</span>
    </div>
  );
}
function ExpandRow({ count, top }) {
  const link = { background: "none", border: "none", padding: "1px 6px", borderRadius: 4, fontSize: 12, color: "var(--accent-strong)", fontWeight: 600, cursor: "pointer" };
  return (
    <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 12, minHeight: 26, background: "var(--bg-subtle)", fontSize: 12, color: "var(--text-muted)" }}>
      {!top && <button style={link}>↓ Show 20</button>}
      <span>{count} unchanged lines</span>
      <button style={link}>↑ Show 20</button>
      <button style={link}>Expand all</button>
    </div>
  );
}

function FileCard({ file, threads, hideWs, onGutter, selectedKey, composerAt, onComposerSubmit, onComposerCancel, threadProps }) {
  const [expanded, setExpanded] = useState(true);
  return (
    <section style={{ border: "1px solid var(--border)", borderRadius: 8, background: "var(--bg-card)", boxShadow: "var(--shadow-card)", overflow: "hidden" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "9px 14px", borderBottom: expanded && file.hunks.length ? "1px solid var(--border)" : "none" }}>
        <button onClick={() => setExpanded(!expanded)} style={{ flex: "none", width: 20, height: 20, background: "none", border: "none", cursor: "pointer", color: "var(--text-muted)", fontSize: 10, padding: 0 }}>{expanded ? "▾" : "▸"}</button>
        <StatusBadge s="modified" />
        <span style={{ flex: 1, minWidth: 0, fontFamily: MONO, fontSize: 13, fontWeight: 700, color: "var(--text-strong)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{file.path}</span>
        <Counts a={file.a} d={file.d} />
        <Btn v="outline" sm>+ Add comment</Btn>
      </div>
      {expanded && file.hunks.map((h, hi) => {
        const wsHidden = hideWs ? h.lines.filter((l) => l.ws).length : 0;
        return (
        <React.Fragment key={hi}>
          {h.above && <ExpandRow count={h.above.count} top />}
          <div style={{ fontFamily: MONO, fontSize: 12, lineHeight: "26px", padding: "0 12px", background: "var(--hunk-bg)", color: "var(--hunk-text)" }}>{h.header}</div>
          {h.lines.map((line, li) => {
            if (hideWs && line.ws) return null;
            const key = `${file.path}:${hi}:${li}`;
            const thread = threads.find((t) => t.anchor === key);
            return (
              <React.Fragment key={li}>
                <DiffLine line={line} selected={selectedKey === key} onGutter={() => onGutter(key, line)} />
                {thread && (
                  <div style={{ padding: "8px 16px 8px 40px", background: "var(--bg-subtle)", borderTop: "1px solid var(--border)", borderBottom: "1px solid var(--border)" }}>
                    <div style={{ maxWidth: 820 }}><Thread c={thread} replies={thread.replies} {...threadProps(thread)} /></div>
                  </div>
                )}
                {composerAt === key && (
                  <div style={{ padding: "8px 16px 8px 40px", background: "var(--bg-subtle)", borderTop: "1px solid var(--border)", borderBottom: "1px solid var(--border)" }}>
                    <div style={{ maxWidth: 820, background: "var(--bg-card)", border: "1px solid var(--border)", borderRadius: 8 }}>
                      <Composer placeholder="Comment on the selected lines…" onSubmit={(t) => onComposerSubmit(key, line, t)} onCancel={onComposerCancel} />
                    </div>
                  </div>
                )}
              </React.Fragment>
            );
          })}
          {wsHidden > 0 && (
            <div style={{ display: "flex", alignItems: "center", justifyContent: "center", minHeight: 26, background: "var(--bg-subtle)", fontSize: 12, color: "var(--text-muted)", fontStyle: "italic" }}>
              {wsHidden} whitespace-only {wsHidden === 1 ? "line" : "lines"} hidden
            </div>
          )}
          {h.below && <ExpandRow count={h.below.count} />}
        </React.Fragment>
        );
      })}
      {expanded && !file.hunks.length && (
        <div style={{ display: "flex", alignItems: "center", justifyContent: "center", minHeight: 26, background: "var(--bg-subtle)", fontSize: 12, color: "var(--accent-strong)", fontWeight: 600, borderTop: "1px solid var(--border)", cursor: "pointer" }}>↕ Expand 3 hidden lines</div>
      )}
    </section>
  );
}

/* ---------- export menu ---------- */
function ExportMenu({ disabled, onCopied }) {
  const [open, setOpen] = useState(false);
  return (
    <span style={{ position: "relative" }}>
      <Btn sm={false} disabled={disabled} onClick={() => setOpen(!open)} title="Copy the review's open comments to the clipboard">Export ▾</Btn>
      {open && (
        <div role="menu" style={{ position: "absolute", top: "calc(100% + 4px)", right: 0, zIndex: 30, display: "flex", flexDirection: "column", minWidth: 208, background: "var(--bg-card)", border: "1px solid var(--border)", borderRadius: 6, boxShadow: "var(--shadow-menu)", overflow: "hidden" }}>
          {["Markdown", "JSON", "Agent prompt + Markdown", "Agent prompt + JSON"].map((l, i) => (
            <button key={l} onClick={() => { setOpen(false); onCopied(l); }} style={{ background: "none", border: "none", borderTop: i ? "1px solid var(--border)" : "none", padding: "8px 13px", fontSize: 13, textAlign: "left", cursor: "pointer" }}
              onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-subtle)"} onMouseLeave={(e) => e.currentTarget.style.background = "none"}>{l}</button>
          ))}
        </div>
      )}
    </span>
  );
}

/* ---------- archive overlay ---------- */
function ArchiveOverlay({ onClose }) {
  const [selected, setSelected] = useState(null);
  return (
    <div role="dialog" style={{ position: "absolute", inset: 0, background: "var(--overlay-scrim)", display: "flex", alignItems: "flex-start", justifyContent: "center", padding: "64px 16px 32px", zIndex: 10 }}>
      <div style={{ width: "min(720px,100%)", maxHeight: "80%", overflowY: "auto", background: "var(--bg-card)", border: "1px solid var(--border)", borderRadius: 10, padding: "12px 16px 16px", boxShadow: "var(--shadow-menu)" }}>
        <header style={{ display: "flex", alignItems: "center", gap: 10, paddingBottom: 8, borderBottom: "1px solid var(--border)" }}>
          {selected ? <Btn v="ghost" sm onClick={() => setSelected(null)}>← All archived reviews</Btn>
            : <span style={{ fontFamily: "var(--font-serif)", fontWeight: 600, fontSize: 16, color: "var(--text-strong)" }}>Archived reviews</span>}
          <span style={{ fontSize: 11, fontWeight: 600, padding: "1px 8px", borderRadius: 999, border: "1px solid var(--border-strong)", color: "var(--text-muted)" }}>read-only</span>
          <span style={{ flex: 1 }} />
          <Btn v="ghost" sm onClick={onClose}>Close</Btn>
        </header>
        {!selected ? (
          <ul style={{ listStyle: "none", margin: "8px 0 0", padding: 0 }}>
            {[["feature/int-4980-invoice-emails", "origin/production", 4, "Jul 2, 2026"], ["fix/int-4711-plan-lockin", "origin/production", 2, "Jun 18, 2026"]].map(([b, base, n, d]) => (
              <li key={b}>
                <button onClick={() => setSelected(b)} style={{ display: "flex", width: "100%", alignItems: "baseline", gap: 10, padding: "9px 6px", background: "none", border: "none", borderBottom: "1px solid var(--border)", cursor: "pointer", textAlign: "left" }}
                  onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-subtle)"} onMouseLeave={(e) => e.currentTarget.style.background = "none"}>
                  <span style={{ fontFamily: MONO, fontWeight: 600, fontSize: 13 }}>{b}</span>
                  <span style={{ color: "var(--text-muted)", fontSize: 12, fontFamily: MONO }}>← {base}</span>
                  <span style={{ flex: 1 }} />
                  <span style={{ color: "var(--text-muted)", fontSize: 12 }}>{n} comments</span>
                  <span style={{ color: "var(--text-muted)", fontSize: 12 }}>{d}</span>
                </button>
              </li>
            ))}
          </ul>
        ) : (
          <div>
            <p style={{ color: "var(--text-muted)", fontSize: 13, margin: "10px 0" }}><strong style={{ fontFamily: MONO, color: "var(--text)" }}>{selected}</strong> <span style={{ fontFamily: MONO }}>← origin/production</span> · committed · archived Jul 2, 2026</p>
            <Thread readOnly c={{ id: 1, key: "a1", body: "Ship it once the invoice email copy is approved.", time: "Jul 1 at 4:20 PM", state: "open" }} replies={[]} />
          </div>
        )}
      </div>
    </div>
  );
}

/* ---------- welcome ---------- */
function Welcome({ onOpen }) {
  const recents = [["www", "~/Code/www"], ["diff-viewer", "~/Code/diff-viewer"]];
  return (
    <main style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 12, padding: 32, background: "var(--bg-app)" }}>
      <img src="../../assets/prologue-logo.svg" width="72" height="72" alt="" style={{ borderRadius: 16, marginBottom: 4 }} />
      <h1 style={{ margin: 0, fontFamily: "var(--font-serif)", fontSize: 28, fontWeight: 600, color: "var(--text-strong)" }}>Prologue</h1>
      <p style={{ margin: "0 0 12px", color: "var(--text-muted)" }}>Review local branches with your agent — before the PR.</p>
      <Btn v="primary" onClick={onOpen}>Open Repository…</Btn>
      <section style={{ marginTop: 24, width: "min(480px,100%)" }}>
        <h2 style={{ fontSize: 11, fontWeight: 600, textTransform: "uppercase", letterSpacing: "0.06em", color: "var(--text-muted)", margin: "0 0 8px", fontFamily: MONO }}>Recent repositories</h2>
        <ul style={{ listStyle: "none", margin: 0, padding: 0, border: "1px solid var(--border)", borderRadius: 8, overflow: "hidden", background: "var(--bg-card)" }}>
          {recents.map(([name, dir], i) => (
            <li key={name} style={{ display: "flex", alignItems: "stretch", borderTop: i ? "1px solid var(--border)" : "none" }}>
              <button onClick={onOpen} style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", alignItems: "flex-start", gap: 2, background: "none", border: "none", padding: "9px 13px", textAlign: "left", cursor: "pointer" }}
                onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-subtle)"} onMouseLeave={(e) => e.currentTarget.style.background = "none"}>
                <span style={{ fontWeight: 600, fontFamily: MONO, fontSize: 13 }}>{name}</span>
                <span style={{ color: "var(--text-muted)", fontSize: 12, fontFamily: MONO }}>{dir}</span>
              </button>
              <button aria-label={`Remove ${name}`} style={{ background: "none", border: "none", padding: "0 13px", color: "var(--text-muted)", fontSize: 15, cursor: "pointer" }}>×</button>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}

/* ---------- review screen ---------- */
const WS_KEY = "prologue.hideWhitespace";
let nextId = 5;
function Review({ onArchive }) {
  const [mode, setMode] = useState("committed");
  const [hideWs, setHideWs] = useState(() => { try { return localStorage.getItem(WS_KEY) === "1"; } catch { return false; } });
  const toggleWs = () => setHideWs((v) => { const next = !v; try { localStorage.setItem(WS_KEY, next ? "1" : "0"); } catch {} return next; });
  const [selectedFile, setSelectedFile] = useState(0);
  const [toast, setToast] = useState(null);
  const [addingReview, setAddingReview] = useState(false);
  const [selectedKey, setSelectedKey] = useState(null);
  const [composerAt, setComposerAt] = useState(null);
  const [reviewComments, setReviewComments] = useState([]);
  const [threads, setThreads] = useState([
    { id: 2, key: "t2", anchor: `${FILE1.path}:1:2`, location: "Lines 95–98", body: "Here is a local comment", time: "Jul 16 at 3:14 PM", state: "open",
      replies: [{ id: 2, key: "t2r1", body: "And a reply — one level of nesting, indented once and no further.", time: "Jul 17 at 9:02 AM", state: "open" }] },
  ]);
  const pop = (text) => { setToast(text); window.setTimeout(() => setToast(null), 2500); };
  const openThreadCount = reviewComments.filter((c) => c.state === "open").length + threads.filter((t) => t.state === "open").length;
  const threadProps = (t) => ({
    onResolve: () => setThreads((p) => p.map((x) => x.key === t.key ? { ...x, state: "resolved" } : x)),
    onDelete: () => setThreads((p) => p.filter((x) => x.key !== t.key)),
    onReply: (body) => setThreads((p) => p.map((x) => x.key === t.key ? { ...x, replies: [...x.replies, { id: x.id, key: `${x.key}r${x.replies.length + 1}`, body, time: "Just now", state: "open" }] } : x)),
  });
  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", background: "var(--bg-app)" }}>
      {/* toolbar */}
      <header style={{ display: "flex", flexWrap: "wrap", rowGap: 8, alignItems: "center", gap: 12, padding: "10px 16px", borderBottom: "1px solid var(--border)", flex: "none" }}>
        <span title="~/Code/www" style={{ background: "var(--teal-800)", color: "var(--text-on-dark)", fontFamily: MONO, fontSize: 12.5, fontWeight: 600, padding: "5px 12px", borderRadius: 6, cursor: "pointer" }}>www</span>
        <Select label="Base" value="origin/production" />
        <span style={{ color: "var(--text-muted)" }}>←</span>
        <Select label="Branch" value="feature/int-5065-allow-lifetim…" />
        <span style={{ flex: 1 }} />
        <span style={{ display: "inline-flex", border: "1px solid var(--border-strong)", borderRadius: 6, overflow: "hidden", background: "var(--bg-subtle)" }}>
          {[["committed", "Committed only"], ["staged", "Include staged"], ["all", "Staged + unstaged"]].map(([v, l], i) => (
            <button key={v} onClick={() => setMode(v)} style={{ display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 12px", fontSize: 12.5, fontWeight: mode === v ? 600 : 400, color: mode === v ? "var(--text-strong)" : "var(--text-muted)", background: mode === v ? "var(--bg-card)" : "transparent", border: "none", borderLeft: i ? "1px solid var(--border)" : "none", cursor: "pointer", whiteSpace: "nowrap" }}>
              {mode === v && <span style={{ width: 7, height: 7, borderRadius: 999, background: "var(--accent)" }} />}{l}
            </button>
          ))}
        </span>
        <button type="button" onClick={toggleWs} aria-pressed={hideWs}
          title="Hide whitespace-only changes in all files — remembered across reviews"
          style={{ display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 12px", fontSize: 12.5, fontWeight: hideWs ? 600 : 400, color: hideWs ? "var(--text-strong)" : "var(--text-muted)", background: hideWs ? "var(--bg-card)" : "var(--bg-subtle)", border: "1px solid var(--border-strong)", borderRadius: 6, cursor: "pointer", whiteSpace: "nowrap" }}>
          {hideWs && <span style={{ width: 7, height: 7, borderRadius: 999, background: "var(--accent)" }} />}
          Hide whitespace
        </button>
        <ExportMenu disabled={openThreadCount === 0} onCopied={(l) => pop(`Copied ${l} to clipboard`)} />
        <Btn onClick={onArchive} title="Browse archived reviews (read-only)">Archived</Btn>
        <Btn title="Refresh branches and diff">↻ Refresh</Btn>
      </header>
      <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
        {/* sidebar */}
        <aside style={{ flex: "none", width: 288, overflowY: "auto", borderRight: "1px solid var(--border)", padding: "14px 10px" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", padding: "0 10px 10px" }}>
            <span style={{ fontFamily: MONO, fontSize: 11, fontWeight: 600, letterSpacing: "0.06em", textTransform: "uppercase", color: "var(--text-muted)" }}>3 files changed</span>
            <Counts a={8} d={4} size={12} />
          </div>
          {FILES.map((f, i) => {
            const n = i === 0 ? threads.filter((t) => t.state === "open").length : 0;
            const sel = selectedFile === i;
            return (
              <button key={f.path} title={f.path} onClick={() => setSelectedFile(i)} style={{ display: "flex", alignItems: "center", gap: 8, width: "100%", padding: "6px 10px", background: sel ? "var(--surface-selected)" : "none", border: "none", borderRadius: 6, boxShadow: sel ? "inset 3px 0 0 var(--accent)" : "none", textAlign: "left", cursor: "pointer" }}>
                <StatusBadge s="modified" />
                <span style={{ flex: 1, minWidth: 0, fontFamily: MONO, fontSize: 12.5, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", direction: "rtl", textAlign: "left" }}><bdi>{f.short}</bdi></span>
                {n > 0 && <Pill n={n} />}
                <Counts a={f.a} d={f.d} size={12} />
              </button>
            );
          })}
        </aside>
        {/* main pane */}
        <div style={{ flex: 1, minWidth: 0, overflowY: "auto", padding: "18px 22px 40px" }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 12 }}>
            <h2 style={{ margin: 0, fontFamily: "var(--font-serif)", fontSize: 20, fontWeight: 600, color: "var(--text-strong)" }}>Review comments{reviewComments.length > 0 && ` (${reviewComments.length})`}</h2>
            {!addingReview && <Btn v="primary" onClick={() => setAddingReview(true)}>+ Add review comment</Btn>}
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 10, marginBottom: 18 }}>
            {reviewComments.map((c) => (
              <Thread key={c.key} c={c} replies={c.replies}
                onResolve={() => setReviewComments((p) => p.map((x) => x.key === c.key ? { ...x, state: "resolved" } : x))}
                onDelete={() => setReviewComments((p) => p.filter((x) => x.key !== c.key))}
                onReply={(body) => setReviewComments((p) => p.map((x) => x.key === c.key ? { ...x, replies: [...x.replies, { id: x.id, key: `${x.key}r${x.replies.length + 1}`, body, time: "Just now", state: "open" }] } : x))} />
            ))}
            {addingReview && (
              <div style={{ background: "var(--bg-card)", border: "1px solid var(--border)", borderRadius: 8 }}>
                <Composer placeholder="Overall notes about this review…" onSubmit={(t) => { setReviewComments((p) => [...p, { id: nextId++, key: `rc${Date.now()}`, body: t, time: "Just now", state: "open", replies: [] }]); setAddingReview(false); }} onCancel={() => setAddingReview(false)} />
              </div>
            )}
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {[FILE1, FILE2].map((f) => (
              <FileCard key={f.path} file={f} threads={threads} hideWs={hideWs} selectedKey={selectedKey} composerAt={composerAt}
                onGutter={(key) => { setSelectedKey(key); setComposerAt(key); }}
                onComposerSubmit={(key, line, t) => {
                  const lineNo = line.n ?? line.o;
                  setThreads((p) => [...p, { id: nextId++, key: `t${Date.now()}`, anchor: key, location: `Line ${lineNo}`, body: t, time: "Just now", state: "open", replies: [] }]);
                  setComposerAt(null); setSelectedKey(null);
                }}
                onComposerCancel={() => { setComposerAt(null); setSelectedKey(null); }}
                threadProps={threadProps} />
            ))}
            <FileCard file={FILES[2]} threads={[]} onGutter={() => {}} threadProps={threadProps} />
          </div>
        </div>
      </div>
      {toast && <div role="status" style={{ position: "absolute", bottom: 20, left: "50%", transform: "translateX(-50%)", zIndex: 40, padding: "8px 16px", border: "1px solid var(--border)", borderRadius: 6, background: "var(--bg-card)", fontSize: 13, boxShadow: "var(--shadow-menu)" }}>{toast}</div>}
    </div>
  );
}

/* ---------- app shell ---------- */
function App() {
  const [screen, setScreen] = useState("review");
  const [archive, setArchive] = useState(false);
  return (
    <div style={{ height: "100%", padding: 24, display: "flex" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", borderRadius: 10, overflow: "hidden", boxShadow: "0 24px 64px rgba(0,0,0,0.35)", position: "relative", background: "var(--bg-app)" }}>
        <header style={{ height: 44, display: "flex", alignItems: "center", justifyContent: "center", position: "relative", background: "var(--bg-titlebar)", flex: "none" }}>
          <div style={{ position: "absolute", left: 16, display: "flex", gap: 8 }}>
            <span onClick={() => setScreen(screen === "review" ? "welcome" : "review")} title="Toggle welcome/review" style={{ width: 12, height: 12, borderRadius: 999, background: "#ff5f57", cursor: "pointer" }} />
            <span style={{ width: 12, height: 12, borderRadius: 999, background: "#febc2e" }} />
            <span style={{ width: 12, height: 12, borderRadius: 999, background: "#28c840" }} />
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
            <svg width="13" height="19" viewBox="182 0 148 336" aria-hidden="true"><path d="M182 0 H330 V336 L256 284 L182 336 Z" fill="#F6A33C" /></svg>
            <span style={{ fontFamily: "var(--font-serif)", fontSize: 17, fontWeight: 600, color: "var(--text-on-dark)" }}>Prologue</span>
            {screen === "review" && <span style={{ fontSize: 13.5, color: "var(--text-on-dark)", opacity: 0.62 }}>— feature/int-5065-allow-lifetime</span>}
          </div>
        </header>
        {screen === "welcome" ? <Welcome onOpen={() => setScreen("review")} /> : <Review onArchive={() => setArchive(true)} />}
        {archive && <ArchiveOverlay onClose={() => setArchive(false)} />}
      </div>
    </div>
  );
}
ReactDOM.createRoot(document.getElementById("root")).render(<App />);
