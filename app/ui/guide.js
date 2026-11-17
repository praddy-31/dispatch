// The Guide: one short section per feature, opened from the ? sheet and ⌘K.
// Offline: the text is here, drawn with textContent; `x` in a line becomes a key cap.
"use strict";

(function () {
  const { el, $ } = window.D;

  // where: how to get there. text: short paragraphs.
  const SECTIONS = [
    { id: "suggested", title: "Suggested", where: "Research → Suggested chip, or ⌘K",
      text: [
        "The past week's best papers you haven't opened yet, across all your topics (about 30). "
        + "Best means watched keywords and followed authors first, then papers like the ones you saved, "
        + "noted or tagged (papers you opened count half). The preview says why each one is there.",
        "Saved, noted and tagged papers are left out. Opening a paper (`o` or `p`) takes it off the list. "
        + "The menu bar's five papers come from here once you've seen today's.",
        "On Mondays a note above Research says how many good papers you missed last week; click it to see them.",
      ] },
    { id: "later", title: "Read later", where: "`l` on a paper; the ⏱ Later chip; Settings → Reading",
      text: [
        "A list that isn't Saved: nothing on it goes into dispatch.bib. `l` adds the paper or takes it off; the ⏱ Later chip shows the list.",
        "A paper stays a week (change it in Settings → Reading → Keep Read later for), and leaves as soon as you open its page or PDF.",
      ] },
    { id: "triage", title: "Triage", where: "`e` in Research or News",
      text: [
        "The unread papers of the current list, one at a time, full width. "
        + "`s` keeps it (saves it), `l` puts it on today's later list, `x` skips it (marks it seen), `z` undoes the last one.",
        "Stop with Esc at any time; what you did so far stays done.",
      ] },
    { id: "abstracts", title: "Abstract view", where: "`a` in any list",
      text: [
        "The current list one paper at a time in large text, for reading abstracts rather than sorting them. "
        + "`j`/`k` move, `s` saves, `l` adds to later, `o` opens the page, `p` the PDF, Esc goes back to the list.",
        "`g` (in the list) jumps to the next topic that still has unread papers.",
      ] },
    { id: "log", title: "Reading log", where: "⌘K: This week's reading log, or Settings → Research workflow",
      text: [
        "This week's notes (Monday to Sunday) as one Markdown document, copied for pasting anywhere: "
        + "each paper you wrote a note on (`n`), with its authors and link. Later and saves without a note don't count.",
        "With a notes folder set (below), the log is also written there as a file.",
      ] },
    { id: "documents", title: "Dispatch documents", where: "Settings → Research workflow → Dispatch documents",
      text: [
        "One folder you choose for your .bib file and the PDFs Dispatch downloads. "
        + "Move… picks a new folder and takes Dispatch's files along (the .bib and PDFs it named); "
        + "anything else in the old folder stays, and nothing in the new one is replaced.",
      ] },
    { id: "bib", title: "Live .bib file", where: "Settings → Research workflow → .bib file (switch)",
      text: [
        "dispatch.bib in Dispatch documents holds your saved papers (★, `s`), all of them or those with one tag. "
        + "Save a paper and it's added within a few seconds; unsave it and it's taken out the same way. "
        + "Read later (`l`) never adds to it. Point LaTeX or Overleaf (through Git) at it. Switching it off leaves the file where it is.",
        "Keys are smith2024adiabatic or the arXiv id (Citation keys). `c` copies \\cite{key} with the same key; "
        + "⌘K → Copy \\bibitem gives an entry for a thebibliography list.",
        "Dispatch only writes a file it made itself, so it never replaces a .bib of yours.",
      ] },
    { id: "notes", title: "Obsidian notes", where: "Settings → Research workflow → Notes folder",
      text: [
        "One Markdown note per saved paper (title, authors, abstract, tags, your note) in a folder you choose, "
        + "for example inside your Obsidian vault. Notes follow your changes in Dispatch.",
        "Once you edit or delete a note there, Dispatch leaves it alone from then on.",
      ] },
    { id: "pdf", title: "PDFs", where: "`p` and `d`; Settings → Research workflow → PDF app",
      text: [
        "`p` opens the PDF: in the browser (online), or in the PDF app you chose. An app needs the file, "
        + "so Dispatch downloads it once, named 2026 Smith - Title.pdf, into Dispatch documents (Downloads if you haven't chosen one) "
        + "and opens it from there. `d` always downloads.",
        "PDF app lists the PDF apps installed on this Mac; Other app… lets you pick any app.",
      ] },
    { id: "scholar", title: "Similar, cited by, references", where: "A paper's preview → Semantic Scholar (Settings → Discovery)",
      text: [
        "With Semantic Scholar on (Settings → Discovery), each paper's preview has a Semantic Scholar box. "
        + "One click asks for three lists: similar papers, the papers citing it (most cited first), and its references. "
        + "Without a key it takes 10 seconds or more (everyone shares one limit, so it's sometimes busy); with your own free key, about 3.",
        "Lists are kept, so they show again offline; fetch again for newer citations. Papers you have in Dispatch open in Dispatch, others on arXiv or doi.org.",
        "Only the paper's arXiv id or DOI is sent. New papers take a day or two to appear there.",
      ] },
    { id: "news", title: "News: today, mutes, discussions", where: "News → Today chip; a story's preview; Settings → News",
      text: [
        "Today (chip, or ⌘K) shows the last 24 hours by topic, one line per story, unread first.",
        "The same story from several feeds shows once, marked \u201c3 sources\u201d; its preview links the other feeds' versions, and reading one marks them all read.",
        "Mute a source from a story's preview, or a word in Settings → News, for a day, a week, a month or until you unmute it. "
        + "Muted stories are hidden and marked read (Stats doesn't count them). The same button then says Unmute, in case it was a slip.",
        "When a feed links each story's discussion page (RSS `<comments>`), the preview has a Discussion link.",
      ] },
    { id: "menubar", title: "Menu bar", where: "The menu-bar icon; Settings → Menu bar",
      text: [
        "`⌃⌥S` opens Dispatch on Search from any app (change it or turn it off in Settings → Menu bar).",
        "Beside the icon: nothing, a dot for new papers since you last opened the window, Suggested papers left, "
        + "today's papers by people and keywords you follow, the weekly goal (papers read / goal), today's later queue, or the unread counts. "
        + "The icon turns red while a feed's last fetch failed; Sources in the menu lists it with Retry.",
        "The menu also copies the BibTeX of the paper you opened last, lists your recently saved papers, "
        + "says when arXiv has nothing new (weekends), and pauses notifications for an hour or until tomorrow morning. "
        + "macOS Focus already holds notifications back by itself.",
        "SketchyBar: turn on Status for SketchyBar, then add the Dispatch item in Palette → Bar. Pick what the pill shows "
        + "(papers read today by default). Dispatch writes status.json in its data folder; any script may read it.",
      ] },
    { id: "health", title: "Data health and privacy", where: "Settings → Data",
      text: [
        "Data health: items per source, feeds gone quiet, notes without a paper, and SQLite's own check of the database. "
        + "Compact database gives unused space back to macOS.",
        "Export everything writes plain files any program reads: dispatch.json (everything, with your saves, notes, tags and read state) "
        + "and saved.csv (saved papers, for a spreadsheet).",
        "Network log: every host Dispatch contacted in the last 7 days, how often and why. Only host names are kept, on this Mac.",
        "Settings → Sync → Sync check says when iCloud sync last merged and how many conflict copies it has resolved.",
      ] },
    { id: "deadlines", title: "Deadlines", where: "Settings → Research workflow → Edit deadlines…",
      text: [
        "Conference and journal deadlines in deadlines.json, a small file you edit by hand "
        + "(Edit deadlines… makes an example and opens it in TextEdit).",
        "The next three show in the menu-bar menu, the next ten in Stats, up to 90 days ahead. The menu updates within seconds of saving the file.",
      ] },
    { id: "pins", title: "Pinned searches", where: "Search → a query → 📌 Pin to Research (or ⌘K)",
      text: [
        "A search you run often becomes a 🔎 chip in Research, with its count of unread matches from the last 30 days. "
        + "The chip lists matches, unread first.",
        "Pins live in topics.json, so they sync with your topics. Unpin from the chip's list header.",
      ] },
    { id: "links", title: "dispatch:// links", where: "Settings → Links; Copy link in each preview",
      text: [
        "Links that open Dispatch from anywhere: Raycast, Shortcuts (Open URLs), SketchyBar, a note, or Terminal with open \"…\".",
        "dispatch://open/saved opens a tab, dispatch://search?q=spin%20chain searches, dispatch://paper/2410.01234 "
        + "shows a paper, dispatch://refresh refreshes. Settings → Links has them with Copy buttons.",
        "Add a paper: ⌘N opens Saved's Add a paper box; paste an arXiv link, an arXiv id or a DOI. dispatch://add?q=… does the same "
        + "from other apps. A paper that isn't on arXiv is kept in Saved, with its BibTeX from doi.org.",
      ] },
    { id: "backups", title: "Backups", where: "Settings → Data",
      text: [
        "One backup a day by itself (database, topics, settings), the newest five kept in backups/ in the data folder.",
        "Back up now makes one at once; Export… copies one to a folder you choose; Restore puts one back "
        + "(database and topics; your settings stay) after backing up what you have first.",
      ] },
  ];

  // "press `s` to save" → text with <kbd>s</kbd>; still textContent only.
  function line(text) {
    return el("p", {}, ...text.split("`").map((part, i) => (i % 2 ? el("kbd", { text: part }) : part)));
  }

  function open(id) {
    const body = el("div", { class: "guide-body" }, ...SECTIONS.map((s) => el("section", { id: `guide-${s.id}` },
      el("h3", { text: s.title }),
      el("div", { class: "where" }, line(s.where)),
      ...s.text.map(line))));
    const index = el("nav", { class: "guide-index" }, ...SECTIONS.map((s) => el("button", {
      class: "chip", onclick: () => body.querySelector(`#guide-${s.id}`)?.scrollIntoView({ block: "start" }),
    }, s.title)));
    const sheet = el("div", { class: "sheet guide", tabindex: "-1", onclick: (e) => e.stopPropagation() },
      el("div", { class: "guide-head" }, el("h2", { text: "Guide" }),
        el("span", { class: "sub", text: "Esc closes · ? lists every shortcut" })),
      index, body);
    $("overlay").replaceChildren(sheet);
    $("overlay").hidden = false;
    D.keys.setMode("help");
    sheet.focus(); // so ↑/↓ and space scroll it
    if (id) body.querySelector(`#guide-${id}`)?.scrollIntoView({ block: "start" });
  }

  D.guide = { open, sections: SECTIONS.map((s) => s.id) };
})();
