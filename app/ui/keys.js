// Keyboard: single-key shortcuts, the shortcut strip at the bottom (it changes with what
// you're doing), the ⌘K command palette and the ? sheet. Every action lives in ACTIONS once;
// the palette, the sheet and the strip are all drawn from it.
"use strict";

(function () {
  const { el, $, state: S } = window.D;

  const reading = () => D.reading();
  const hasItem = () => reading() && (!!S.selected || S.picked.size > 0);
  // Citations need papers: Research, or Saved when the target items are papers.
  // Papers: from arXiv, or added by DOI.
  const research = () => hasItem() && D.targets().every((k) => k.startsWith("arxiv:") || k.startsWith("doi:"));

  // key: what to press (shown as written); when: whether it applies right now.
  const ACTIONS = [
    { id: "next", name: "Next item", key: "j", alt: "↓", group: "Reading", when: reading, run: () => D.move(1) },
    { id: "prev", name: "Previous item", key: "k", alt: "↑", group: "Reading", when: reading, run: () => D.move(-1) },
    { id: "open", name: "Open in browser", key: "o", alt: "↵", group: "Reading", when: hasItem, run: () => D.openSelected() },
    { id: "pdf", name: "Open PDF", key: "p", group: "Reading", when: research, run: () => D.openSelected("pdf") },
    { id: "download", name: "Download the PDF (tidy name) and open it", key: "d", group: "Reading", when: research, run: () => D.downloadPdf() },
    { id: "read", name: "Mark read / unread", key: "u", group: "Reading", when: hasItem, run: () => D.toggleRead() },
    { id: "save", name: "Save / remove from Saved", key: "s", group: "Reading", when: hasItem, run: () => D.toggleSaved() },
    { id: "later", name: "Read later / take off", key: "l", group: "Reading", when: hasItem, run: () => D.toggleLater() },
    { id: "laterlist", name: "Show today's later list", key: "", group: "Reading", when: () => !!D.section(), run: () => D.showLater(true) },
    { id: "triage", name: "Triage the unread ones (keep, later, skip)", key: "e", group: "Reading", when: () => !!D.section() && !S.later, run: () => D.triage.start() },
    { id: "abstracts", name: "Abstract view: one at a time in large text (j/k to move)", key: "a", group: "Reading", when: () => reading() && D.visibleRows().length > 0 && !(S.view === "search" && S.search.mode !== "papers" && S.search.mode !== "notes"), run: () => D.triage.start("read") },
    { id: "skim", name: "Next topic with unread items", key: "g", group: "Reading", when: () => !!D.section(), run: () => D.nextUnreadTopic() },
    { id: "updates", name: "Hide / show updated papers (arXiv replacements)", key: "h", group: "Reading", when: () => S.view === "research", run: () => D.toggleUpdates() },
    { id: "note", name: "Write a note", key: "n", group: "Reading", when: () => hasItem() && S.picked.size < 2, run: () => D.focusNote() },
    { id: "selectall", name: "Select all items", key: "⌘A", group: "Reading", when: reading, run: () => D.selectAll() },
    { id: "allread", name: "Mark all read (this topic, with Undo)", key: "", group: "Reading", when: reading, run: () => D.markAllRead() },
    { id: "search-here", name: "Search here (this section and topic)", key: "/", group: "Reading", when: reading, run: () => D.searchHere() },
    { id: "nexttopic", name: "Next topic (Saved: tag, Search: mode)", key: "]", group: "Reading", when: reading, run: () => D.cycleTopic(1) },
    { id: "prevtopic", name: "Previous topic (Saved: tag, Search: mode)", key: "[", group: "Reading", when: reading, run: () => D.cycleTopic(-1) },
    { id: "bibtex", name: "Copy BibTeX", key: "b", group: "Cite", when: research, run: () => D.copyCitations("bibtex") },
    { id: "ris", name: "Copy RIS", key: "", group: "Cite", when: research, run: () => D.copyCitations("ris") },
    { id: "exportbib", name: "Export BibTeX file…", key: "", group: "Cite", when: research, run: () => D.exportCitations("bibtex") },
    { id: "exportris", name: "Export RIS file…", key: "", group: "Cite", when: research, run: () => D.exportCitations("ris") },
    { id: "zotero", name: "Open in Zotero (import BibTeX)", key: "", group: "Cite", when: () => research() && S.zotero, run: () => D.openInZotero() },
    { id: "cite", name: "Copy \\cite{key} (keys as in your .bib file)", key: "c", group: "Cite", when: research, run: () => D.copyLatex("cite") },
    { id: "bibitem", name: "Copy \\bibitem (for a thebibliography list)", key: "", group: "Cite", when: research, run: () => D.copyLatex("bibitem") },
    { id: "readinglog", name: "This week's reading log (notes as Markdown)", key: "", group: "Cite", when: () => true, run: () => D.settingsView.readingLog() },
    { id: "copylink", name: "Copy this paper's dispatch:// link", key: "", group: "Cite", when: () => research() && S.picked.size < 2, run: () => D.copyPaperLink() },
    { id: "zotero-send", name: "Send to Zotero (the open app, else Zotero online)", key: "", group: "Cite", when: () => research() && D.canSendZotero(), run: () => D.sendToZotero() },
    { id: "research", name: "Go to Research", key: "1", group: "Go to", when: () => true, run: () => D.showView("research") },
    { id: "news", name: "Go to News", key: "2", group: "Go to", when: () => true, run: () => D.showView("news") },
    { id: "saved", name: "Go to Saved", key: "3", group: "Go to", when: () => true, run: () => D.showView("saved") },
    { id: "search", name: "Search the archive", key: "4", alt: "⌘F", group: "Go to", when: () => true, run: () => D.searchFor(S.search.query) },
    { id: "stats", name: "Stats", key: "w", group: "Go to", when: () => true, run: () => D.showView("stats") },
    // t = your tags, ⌘T = the Topics tab (next to ⌘, for Settings).
    { id: "tags", name: "Your tags (Search → Tags)", key: "t", group: "Go to", when: () => true, run: () => D.showTags() },
    { id: "addpaper", name: "Add a paper (arXiv link, id or DOI)", key: "⌘N", group: "Go to", when: () => true, run: () => D.focusAdd() },
    { id: "topics", name: "Edit topics", key: "⌘T", group: "Go to", when: () => true, run: () => D.showView("topics") },
    { id: "settings", name: "Settings", key: "⌘,", group: "Go to", when: () => true, run: () => D.showView("settings") },
    { id: "following", name: "Followed authors, keywords and alerts", key: "", group: "Go to", when: () => true,
      run: () => { D.showView("settings"); setTimeout(() => document.getElementById("following")?.scrollIntoView(), 80); } },
    { id: "foryou", name: "Sort Research: newest ↔ for you", key: "", group: "Reading", when: () => S.view === "research", run: () => D.toggleForYou() },
    { id: "suggested", name: "Suggested: the past week's best papers you haven't opened", key: "", group: "Go to", when: () => true, run: () => D.showSuggested() },
    { id: "missed", name: "What you missed last week", key: "", group: "Go to", when: () => true, run: () => D.showMissed() },
    { id: "pin", name: "Pin this search to Research", key: "", group: "Reading", when: () => S.view === "search" && S.search.mode === "papers" && !!S.search.query.trim(), run: () => D.pinSearch(S.search.query, true) },
    { id: "digest", name: "This week digest (Research)", key: "", group: "Go to", when: () => S.view === "research", run: () => D.toggleDigest() },
    { id: "newsdigest", name: "Today: news of the last 24 hours, one line each", key: "", group: "Go to", when: () => S.view === "news", run: () => D.toggleDigest() },
    { id: "refresh", name: "Refresh now", key: "r", group: "App", when: () => true, run: () => D.refresh() },
    { id: "sources", name: "Show source status", key: "i", group: "App", when: () => true, run: () => D.toggleSources() },
    { id: "sync", name: "Sync with iCloud now", key: "", group: "App", when: () => !!S.settings?.icloud_sync, run: async () => {
      try { await D.invoke("sync_now"); D.toast("Synced with iCloud"); } catch (e) { D.toast(`Sync failed: ${e}`); } } },
    { id: "backup", name: "Back up now (database, topics, settings)", key: "", group: "App", when: () => true, run: () => D.settingsView.backupNow() },
    { id: "palette", name: "Command palette", key: "⌘K", group: "App", when: () => true, run: () => openPalette() },
    { id: "help", name: "All shortcuts", key: "?", group: "App", when: () => true, run: () => openHelp() },
    { id: "guide", name: "Guide: how each feature works", key: "", group: "App", when: () => true, run: () => D.guide.open() },
    { id: "savetopics", name: "Save topics", key: "⌘S", group: "Topics", when: () => S.view === "topics" && D.picker.dirty(), run: () => D.picker.save() },
  ];
  const byKey = new Map(ACTIONS.filter((a) => a.key.length === 1).map((a) => [a.key, a]));

  // ---- the strip ------------------------------------------------------------

  let mode = "list"; // "list" | "query" | "palette" | "help" | "topics" | "typing"

  function drawStrip() {
    const item = (k, label) => el("span", {}, el("kbd", { text: k }), label);
    let parts;
    if (D.triage?.finished && mode === "list") {
      parts = [item("esc", "back to the list"), item("z", "undo last")];
    } else if (D.triage?.active && D.triage.mode === "read" && mode === "list") {
      parts = [item("j/k", "next/previous"), item("s", "save"), item("l", "later"), item("o", "open"),
        S.view === "research" ? item("p", "PDF") : null, item("esc", "back to the list")];
    } else if (D.triage?.active && mode === "list") {
      parts = [item("s", "keep"), item("l", "later"), item("x", "skip"), item("z", "undo"), item("o", "open"),
        S.view === "research" ? item("p", "PDF") : null, item("esc", "stop")];
    } else if (mode === "palette") parts = [item("↑↓", "choose"), item("↵", "run"), item("esc", "close")];
    else if (mode === "help") parts = [item("esc", "close")];
    else if (mode === "helpsheet") parts = [item("g", "Guide"), item("esc", "close")];
    else if (mode === "query") parts = [item("↵", "to the results"), item("esc", "clear")];
    else if (mode === "typing" && document.activeElement === $("addbox")) parts = [item("⏎", "add the paper"), item("esc", "leave the field")];
    else if (mode === "typing") parts = [item("⌘S", "save"), item("esc", "leave the field")];
    else if (S.view === "settings" || S.view === "stats") {
      parts = [item("1", "Research"), item("2", "News"), item("⌘T", "topics"), item("⌘K", "commands")];
    } else if (S.view === "topics") {
      parts = [D.picker.dirty() ? item("⌘S", "save") : null, item("1", "Research"), item("2", "News"), item("⌘K", "commands"), item("?", "all")];
    } else {
      const many = S.picked.size > 1;
      parts = many
        ? [item("u", "read/unread"), item("s", "save"), research() ? item("b", "BibTeX") : null,
          item("⌘-click", "add/remove"), item("⇧-click", "range"), item("esc", "clear"), item("⌘K", "commands")]
        // Only the main keys, so the strip fits next to the status; the rest
        // are in ? and ⌘K.
        : [item("j/k", "move"), item("o", "open"),
          S.view === "research" ? item("p", "PDF") : null,
          item("s", "save"), item("u", "read"),
          S.view === "search" ? item("/", "edit search") : item("/", "search here"),
          item("⌘K", "commands"), item("?", "all shortcuts")];
    }
    $("strip").replaceChildren(...parts.filter(Boolean));
  }

  function setMode(m) {
    mode = m;
    drawStrip();
  }

  // ---- overlays -------------------------------------------------------------

  function closeOverlay() {
    $("overlay").hidden = true;
    $("overlay").replaceChildren();
    setMode("list");
  }

  function openPalette() {
    let choice = 0;
    let shown = [];
    const input = el("input", { placeholder: "Type a command or a topic…", spellcheck: "false", autocomplete: "off" });
    const menu = el("div", { class: "menu" });

    // Actions plus "Topic: …" entries for jumping straight to a topic.
    const entries = () => {
      const out = ACTIONS.filter((a) => a.id !== "palette" && a.when())
        .map((a) => ({ name: a.name, hint: a.key, run: a.run }));
      for (const section of ["research", "news"]) {
        for (const t of (S.topics?.[section] ?? []).filter((x) => x.enabled)) {
          out.push({
            name: `${section === "research" ? "Research" : "News"}: ${t.name}`,
            hint: "topic",
            run: async () => { if (S.view !== section) D.showView(section); D.setTopic(t.id); },
          });
        }
      }
      return out;
    };
    // Every typed letter must appear in order (so "mar" finds "Mark all read").
    const fits = (name, q) => {
      let i = 0;
      const n = name.toLowerCase();
      for (const ch of q.toLowerCase().replace(/\s+/g, "")) {
        i = n.indexOf(ch, i);
        if (i < 0) return false;
        i++;
      }
      return true;
    };
    const draw = () => {
      shown = entries().filter((e) => fits(e.name, input.value));
      choice = Math.min(choice, Math.max(0, shown.length - 1));
      menu.replaceChildren(...(shown.length ? shown.map((e, i) => el("button", {
        "aria-selected": String(i === choice),
        onclick: () => runEntry(e),
      }, e.name, el("span", { class: "sub", text: e.hint }))) : [el("div", { class: "none", text: "No matching command" })]));
      menu.querySelector("[aria-selected='true']")?.scrollIntoView({ block: "nearest" });
    };
    const runEntry = (e) => { closeOverlay(); e.run(); };
    input.addEventListener("input", () => { choice = 0; draw(); });
    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") { choice = Math.min(shown.length - 1, choice + 1); draw(); e.preventDefault(); }
      else if (e.key === "ArrowUp") { choice = Math.max(0, choice - 1); draw(); e.preventDefault(); }
      else if (e.key === "Enter" && shown[choice]) { runEntry(shown[choice]); e.preventDefault(); }
      else if (e.key === "Escape") { closeOverlay(); e.preventDefault(); }
      e.stopPropagation();
    });
    $("overlay").replaceChildren(el("div", { class: "sheet", onclick: (e) => e.stopPropagation() }, input, menu));
    $("overlay").hidden = false;
    setMode("palette");
    draw();
    input.focus();
  }

  function openHelp() {
    // The Guide explains the features; this sheet only lists keys.
    const rows = [el("div", { class: "help-guide" },
      el("button", { class: "btn", onclick: () => D.guide.open() }, "Open the Guide"),
      el("span", { class: "sub" }, "how Suggested, Later, triage, the .bib file, notes, backups and more work · ",
        el("kbd", { text: "g" }), " opens it while this sheet is open (in a list, ", el("kbd", { text: "g" }), " jumps to the next unread topic)"))];
    let group = null;
    for (const a of ACTIONS) {
      if (a.group !== group) { rows.push(el("h3", { text: a.group })); group = a.group; }
      rows.push(a.key
        ? el("span", {}, el("kbd", { text: a.key }), a.alt ? " " : "", a.alt ? el("kbd", { text: a.alt }) : null)
        : el("span", { class: "sub", text: "⌘K only" }));
      rows.push(el("span", { text: a.name }));
    }
    rows.push(el("h3", { text: "Mouse" }));
    rows.push(el("span", { text: "Divider" }), el("span", { text: "Drag to resize the list; double-click to reset" }));
    rows.push(el("span", { text: "Double-click" }), el("span", { text: "Open an item in the browser" }));
    rows.push(el("span", { text: "⌘-click / ⇧-click" }), el("span", { text: "Add to the selection / select a range; esc clears it" }));
    $("overlay").replaceChildren(el("div", { class: "sheet", onclick: (e) => e.stopPropagation() }, el("div", { class: "help" }, ...rows)));
    $("overlay").hidden = false;
    setMode("helpsheet");
  }

  $("overlay").addEventListener("click", closeOverlay);

  // ---- key handling ------------------------------------------------------------

  function isField(t) {
    return t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t?.isContentEditable;
  }

  document.addEventListener("focusin", (e) => {
    if (e.target === $("query")) setMode("query");
    else if (isField(e.target) && !$("overlay").contains(e.target)) setMode("typing");
  });
  document.addEventListener("focusout", (e) => {
    if (isField(e.target) && $("overlay").hidden) setMode("list");
  });

  document.addEventListener("keydown", (e) => {
    const cmd = e.metaKey || e.ctrlKey;
    if (cmd && e.key.toLowerCase() === "k") { e.preventDefault(); $("overlay").hidden ? openPalette() : closeOverlay(); return; }
    if (cmd && e.key.toLowerCase() === "a" && !isField(e.target) && $("overlay").hidden) {
      e.preventDefault();
      if (reading()) D.selectAll();
      return;
    }
    if (cmd && e.key === ",") { e.preventDefault(); D.showView("settings"); return; }
    if (cmd && e.key.toLowerCase() === "n") { e.preventDefault(); if ($("overlay").hidden) D.focusAdd(); return; }
    if (cmd && e.key.toLowerCase() === "t") { e.preventDefault(); if ($("overlay").hidden) D.showView("topics"); return; }
    if (cmd && e.key.toLowerCase() === "f") { e.preventDefault(); if ($("overlay").hidden) D.searchFor(S.search.query); return; }
    if (cmd && e.key.toLowerCase() === "s") {
      if (S.view === "topics") { e.preventDefault(); D.picker.save(); }
      return;
    }
    if (!$("overlay").hidden) {
      if (e.key === "Escape") closeOverlay();
      else if (mode === "helpsheet" && e.key === "g") { e.preventDefault(); D.guide.open(); }
      return;
    }
    if (!isField(e.target) && D.triage?.key(e)) return;

    const t = e.target;
    if (t === $("query")) {
      if (e.key === "Escape") { t.value = ""; t.dispatchEvent(new Event("input")); t.blur(); e.preventDefault(); }
      else if (e.key === "Enter" || e.key === "ArrowDown") { t.blur(); $("list").focus(); e.preventDefault(); }
      return;
    }
    if (isField(t)) { if (e.key === "Escape") t.blur(); return; }
    if (cmd || e.altKey) return;

    if (e.key === "Escape") { if (!D.clearPicked()) D.toggleSources(false); return; }
    let action = byKey.get(e.key);
    if (e.key === "ArrowDown") action = byKey.get("j");
    if (e.key === "ArrowUp") action = byKey.get("k");
    if (e.key === "Enter") action = byKey.get("o");
    if (action && action.when()) { e.preventDefault(); action.run(); }
  });

  D.keys = { drawStrip, openPalette, openHelp, closeOverlay, setMode };
  drawStrip();
})();
