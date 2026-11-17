// Triage: one unread item at a time, full
// width, until the inbox is empty. Keep (s) saves it, Later (l) puts it on today's queue,
// Skip (x) just marks it seen; each one moves on. z undoes the last step, o/p open it, Esc
// stops. The inbox is the unread items of the list you started from, in its order.
//
// It also has the abstract view (`a`), the same full-width card in large text for reading
// rather than deciding: every item of the list from the one under the cursor, j/k (or the
// arrows, space) to move, s save, l later, b BibTeX, o/p open, Esc back to the list at the
// item you were on. An item counts as seen after a second on screen, as in the list.
"use strict";

(function () {
  const { el, $, invoke, state: S } = window.D;

  let inbox = [];   // the rows to go through (a snapshot taken at start)
  let at = 0;       // index of the item shown
  let done = [];    // [{ row, what, was: { saved, later } }] for Undo and the summary
  let active = false;
  let mode = "triage"; // "triage" | "read" (the abstract view)
  let seenTimer = null;

  const row = () => inbox[at];

  function start(how = "triage") {
    if (how === "read") {
      inbox = D.visibleRows().slice();
      if (!inbox.length) { D.toast("Nothing here to read."); return; }
      at = Math.max(0, inbox.findIndex((r) => r.key === S.selected));
      mode = "read";
    } else {
      if (!D.section() || S.later) return;
      inbox = D.visibleRows().filter((r) => !r.read);
      if (!inbox.length) { D.toast("Nothing unread here to triage."); return; }
      at = 0;
      mode = "triage";
    }
    done = [];
    active = true;
    D.draw();
  }

  /// Leaves triage and shows the list again, reloaded (read, saved and later changed).
  function stop() {
    if (!active) return;
    active = false;
    clearTimeout(seenTimer);
    // The abstract view returns to the item you were reading; triage to the top.
    S.selected = mode === "read" ? row()?.key ?? null : null;
    D.loadCounts();
    D.loadItems().then(() => document.querySelector(".row[aria-current='true']")?.scrollIntoView({ block: "center" }));
    D.draw();
  }

  async function act(what) {
    const r = row();
    if (!r) return;
    const was = { saved: r.saved, later: D.isLater(r) };
    try {
      if (what === "keep" && !r.saved) await invoke("set_saved", { keys: [r.key], saved: true });
      if (what === "later" && !was.later) await invoke("set_later", { keys: [r.key], on: true });
      await invoke("set_read", { key: r.key, read: true });
    } catch (e) { D.toast(`Couldn't do that: ${e}`); return; }
    if (what === "keep") r.saved = true;
    if (what === "later") D.markLater([r.key], true);
    r.read = true;
    done.push({ row: r, what, was });
    at += 1;
    draw();
    D.loadCounts(); // the tab, chip and Later numbers follow as you go
  }

  /// Puts the last item back exactly as it was before you decided.
  async function undo() {
    const last = done.pop();
    if (!last) return;
    const r = last.row;
    try {
      if (last.what === "keep" && !last.was.saved) await invoke("set_saved", { keys: [r.key], saved: false });
      if (last.what === "later" && !last.was.later) await invoke("set_later", { keys: [r.key], on: false });
      await invoke("set_read", { key: r.key, read: false });
    } catch (e) { D.toast(`Couldn't undo: ${e}`); done.push(last); return; }
    r.saved = last.was.saved;
    if (last.what === "later" && !last.was.later) D.markLater([r.key], false);
    r.read = false;
    at = inbox.indexOf(r);
    draw();
    D.loadCounts();
  }

  function open(which) {
    const r = row();
    if (which === "pdf") D.openPdf(r);
    else if (r?.link) D.openItemLink(r.key, r.link);
  }

  // ---- abstract view ----

  function go(step) {
    const next = Math.max(0, Math.min(inbox.length - 1, at + step));
    if (next === at) { if (step > 0) D.toast("That's the last one. Esc goes back to the list."); return; }
    at = next;
    draw();
  }

  /// Runs a list action (save, later, BibTeX) on the item shown, then redraws the card.
  async function onShown(fn) {
    S.selected = row()?.key ?? null;
    S.picked = new Set();
    await fn();
    draw();
  }

  function readKey(e) {
    const run = {
      j: () => go(1), ArrowDown: () => go(1), ArrowRight: () => go(1), " ": () => go(1),
      k: () => go(-1), ArrowUp: () => go(-1), ArrowLeft: () => go(-1),
      s: () => onShown(() => D.toggleSaved()), l: () => onShown(() => D.toggleLater([row().key])),
      b: () => row()?.section === "research" && onShown(() => D.copyCitations("bibtex")),
      o: () => open("link"), Enter: () => open("link"), p: () => open("pdf"), Escape: () => stop(),
      a: () => stop(),
    }[e.key];
    if (!run) return true;
    e.preventDefault();
    run();
    return true;
  }

  /// Keys while triaging; returns true when it took the key.
  function key(e) {
    if (!active) return false;
    if (mode === "read" && !(e.metaKey || e.ctrlKey || e.altKey)) return readKey(e);
    if (e.metaKey && e.key.toLowerCase() === "z") { e.preventDefault(); undo(); return true; }
    if (e.metaKey || e.ctrlKey || e.altKey) return false;
    const run = {
      s: () => act("keep"), l: () => act("later"), x: () => act("skip"), " ": () => act("skip"),
      ArrowRight: () => act("skip"), z: () => undo(), ArrowLeft: () => undo(),
      o: () => open("link"), Enter: () => open("link"), p: () => open("pdf"), Escape: () => stop(),
    }[e.key];
    if (!run) return true; // everything else is ignored while triaging
    e.preventDefault();
    run();
    return true;
  }

  function summary() {
    const n = (what) => done.filter((d) => d.what === what).length;
    const parts = [[n("keep"), "kept (saved)"], [n("later"), "for later"], [n("skip"), "skipped"]]
      .filter(([k]) => k).map(([k, label]) => `${k} ${label}`);
    return parts.join(" · ") || "Nothing changed";
  }

  function draw() {
    const pane = $("triage");
    pane.hidden = !active;
    pane.classList.toggle("reading", active && mode === "read");
    if (!active) return;
    if (mode === "read") { drawRead(pane); return; }
    const r = row();
    const button = (label, title, onclick, cls = "") => el("button", { class: `btn ${cls}`, title, onclick }, label);
    if (!r) {
      pane.replaceChildren(el("div", { class: "triage-card triage-done" },
        el("h1", { text: "Inbox empty" }),
        el("p", { class: "muted-note", text: summary() }),
        el("div", { class: "links" },
          button("Back to the list", "Esc", () => stop()),
          done.length ? button("Undo last", "z", () => undo()) : null)));
      D.keys?.drawStrip();
      return;
    }
    const research = r.section === "research";
    const title = el("h1", {}, r.title);
    const abstract = el("div", { class: "abstract", text: r.summary || "No summary in the feed." });
    if (research) { D.math(title); D.math(abstract); }
    const facts = [el("span", { class: "fact", text: D.when(r.published || r.first_seen) })];
    if (research) for (const c of r.categories.slice(0, 6)) facts.push(el("span", { class: "fact fact-cat", text: c }));
    if (r.announce_type && r.announce_type !== "new") facts.push(el("span", { class: "fact replace", text: r.announce_type.replace("-", " ") }));
    pane.replaceChildren(el("div", { class: "triage-card" },
      el("div", { class: "triage-progress" },
        el("span", { text: `${at + 1} of ${inbox.length}` }),
        el("span", { class: "triage-bar" }, el("span", { style: `width: ${(100 * at) / inbox.length}%` }))),
      title,
      el("div", { class: "who", text: research ? r.authors.join(", ") : r.source_name }),
      el("div", { class: "facts" }, ...facts),
      abstract,
      el("div", { class: "links triage-actions" },
        button("★ Keep", "Save it (s)", () => act("keep"), "keep"),
        button("⏱ Later", "Read later (l)", () => act("later")),
        button("Skip", "Mark seen and move on (x, space, →)", () => act("skip")),
        button("Undo", "z or ←", () => undo()),
        button(r.key.startsWith("doi:") ? "Journal page" : research ? "arXiv page" : "Open", "o", () => open("link")),
        r.pdf || r.key.startsWith("arxiv:") ? button("PDF", "p", () => open("pdf")) : null,
        button("Stop", "Esc", () => stop()))));
    pane.scrollTop = 0;
    D.keys?.drawStrip();
  }

  function drawRead(pane) {
    const r = row();
    S.selected = r.key;
    const research = r.section === "research";
    const button = (label, title, onclick, cls = "") => el("button", { class: `btn ${cls}`, title, onclick }, label);
    const title = el("h1", {}, r.saved ? el("span", { class: "star", text: "★ " }) : null, r.title);
    const abstract = el("div", { class: "abstract", text: r.summary || "No summary in the feed." });
    if (research) { D.math(title); D.math(abstract); }
    const facts = [el("span", { class: "fact", text: D.when(r.published || r.first_seen) })];
    if (research) for (const c of r.categories.slice(0, 6)) facts.push(el("span", { class: "fact fact-cat", text: c }));
    if (r.journal_ref || r.doi) facts.push(el("span", { class: "fact published", text: "Published" }));
    for (const h of r.hints ?? []) facts.push(el("span", { class: "fact hint", text: h }));
    pane.replaceChildren(el("div", { class: "triage-card abstract-view" },
      el("div", { class: "triage-progress" },
        el("span", { text: `${at + 1} of ${inbox.length}` }),
        el("span", { class: "triage-bar" }, el("span", { style: `width: ${(100 * (at + 1)) / inbox.length}%` }))),
      title,
      el("div", { class: "who", text: research ? r.authors.join(", ") : r.source_name }),
      el("div", { class: "facts" }, ...facts),
      abstract,
      r.code?.length ? el("div", { class: "links" }, ...r.code.map((c) =>
        button(`↗ ${c.kind}`, c.url, () => D.invoke("open_link", { url: c.url })))) : null,
      el("div", { class: "links triage-actions" },
        button("↑ Previous", "k", () => go(-1)),
        button("Next ↓", "j, space", () => go(1)),
        button(r.saved ? "★ Saved" : "☆ Save", "s", () => onShown(() => D.toggleSaved())),
        button(D.isLater(r) ? "⏱ On Later" : "⏱ Later", "l", () => onShown(() => D.toggleLater([r.key]))),
        button(r.key.startsWith("doi:") ? "Journal page" : research ? "arXiv page" : "Open", "o", () => open("link")),
        r.pdf || r.key.startsWith("arxiv:") ? button("PDF", "p", () => open("pdf")) : null,
        button("Back to the list", "Esc", () => stop()))));
    pane.scrollTop = 0;
    clearTimeout(seenTimer);
    if (!r.read) seenTimer = setTimeout(() => { if (active && row() === r) D.setRead(r.key, true); }, 1000);
    D.keys?.drawStrip();
  }

  D.triage = {
    start, stop, key, draw,
    get active() { return active; },
    get mode() { return mode; },
    get finished() { return active && mode === "triage" && !row(); },
  };
})();
