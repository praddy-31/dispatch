// Dispatch's window: the reader (sections, topic chips, list, preview, divider), read
// state, KaTeX and the status footer. topics.js adds the topic picker; keys.js adds the
// shortcuts, the shortcut strip, ⌘K and the shortcut sheet. They share the `D` object.
// Search is a reading view too: the box at the top searches the whole archive.
//
// Feed text is untrusted: it is only ever set with textContent (KaTeX then builds its own
// nodes from the text), never as HTML.
"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const D = (window.D = {
  state: {
    view: "research",   // "research" | "news" | "saved" | "search" | "stats" | "topics" | "settings"
    topic: null,        // null = all topics in the section
    topics: null,       // topics.json as the app has it
    catalogue: null,    // arXiv groups and category names
    rows: [],
    selected: null,     // item key (the cursor)
    picked: new Set(),  // multi-selection (⌘A, ⌘-click, ⇧-click); empty = just `selected`
    zotero: false,      // Zotero.app is installed
    settings: null,     // settings.json as the app has it
    digestChoice: null, // null = automatic; true/false = the "This week" chip was clicked
    newsDigest: false,  // News → "Today", one line per story by topic
    tag: null,          // Saved view: show only this tag
    tags: [],           // [tag, count] for chips and suggestions
    savedCount: 0,
    status: null,
    // Search view: what was typed, where to look, the order, and how many matched in all.
    // mode: papers | people | tags | notes; person: the card shown in People.
    // Search also took over the list's Filter box: topic, unread only, and since
    // (any | week | month | year).
    search: { mode: "papers", query: "", section: null, topic: null, saved: false, unread: false, since: "any",
      sort: "best", total: 0, person: null },
    // Research's sort: false = newest first, true = "For you". Remembered.
    forYou: localStorage.getItem("dispatch.forYou") === "1",
    // Research hides arXiv's replacements (remembered); the "Later" chip shows
    // today's read-later queue of the section; how many each section has queued.
    hideUpdates: localStorage.getItem("dispatch.hideUpdates") === "1",
    later: false,
    // A Research list that isn't a topic: { kind: "suggested" | "missed", from, to }
    // or { kind: "pin", query }. null = topics as usual.
    extra: null,
    pinCounts: [],      // [query, unread matches in 30 days] for the pinned chips
    missed: 0,          // Monday: good papers left unread last week
    laterCount: { research: 0, news: 0 },
    unread: { research: {}, news: {} },
  },
  invoke,
});
const S = D.state;
const $ = (D.$ = (id) => document.getElementById(id));

D.el = function el(tag, props = {}, ...children) {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "class") n.className = v;
    else if (k === "text") n.textContent = v;
    else if (k === "value") n.value = v;
    else if (k === "checked") n.checked = !!v;
    else if (k.startsWith("on")) n.addEventListener(k.slice(2), v);
    else if (v !== null && v !== undefined && v !== false) n.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children) if (c !== null && c !== undefined && c !== false) n.append(c);
  return n;
};
const el = D.el;

D.section = () => (S.view === "research" || S.view === "news" ? S.view : null);
/// Research, News, Saved or Search: the views with a list and a preview.
D.reading = () => !!D.section() || S.view === "saved" || S.view === "search";

// ---- dates ----------------------------------------------------------------

function sameDay(a, b) {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

D.when = function when(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  const now = new Date();
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  if (sameDay(d, now)) return `today ${time}`;
  const y = new Date(now); y.setDate(now.getDate() - 1);
  if (sameDay(d, y)) return `yesterday ${time}`;
  const t = new Date(now); t.setDate(now.getDate() + 1);
  if (sameDay(d, t)) return `tomorrow ${time}`;
  if (Math.abs(now - d) < 6 * 86400e3) return `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
  return d.toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" });
};

function dayLabel(d) {
  const now = new Date();
  const y = new Date(now); y.setDate(now.getDate() - 1);
  const date = d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
  if (sameDay(d, now)) return `Today · ${date}`;
  if (sameDay(d, y)) return `Yesterday · ${date}`;
  return date;
}

// ---- maths -----------------------------------------------------------------

/// Renders $…$ and $$…$$ in research text. A formula KaTeX can't read stays as plain text.
D.math = function math(node) {
  if (!window.renderMathInElement) return;
  try {
    window.renderMathInElement(node, {
      delimiters: [
        { left: "$$", right: "$$", display: true },
        { left: "\\[", right: "\\]", display: true },
        { left: "$", right: "$", display: false },
        { left: "\\(", right: "\\)", display: false },
      ],
      throwOnError: false,
      errorColor: "inherit",
      trust: false,
    });
  } catch (_) { /* leave the text as it is */ }
};

/// Click a formula in the preview or the abstract view to copy its LaTeX (KaTeX
/// keeps the source in a MathML <annotation>).
document.addEventListener("click", (e) => {
  const k = e.target.closest?.(".katex");
  if (!k || !k.closest("#preview, #triage")) return;
  const tex = k.querySelector('annotation[encoding="application/x-tex"]')?.textContent?.trim();
  if (!tex) return;
  e.preventDefault();
  D.settingsView.copy(tex, "the formula's LaTeX");
});

// ---- data ------------------------------------------------------------------

/// "For you": the days stay in order (newest first) and each day's papers go from
/// the best score down; equal scores keep their newest-first order.
function forYouOrder(rows) {
  const dayOf = (r) => new Date(r.published || r.first_seen).toDateString();
  const dayIndex = new Map();
  for (const r of rows) if (!dayIndex.has(dayOf(r))) dayIndex.set(dayOf(r), dayIndex.size);
  return rows.map((r, i) => [r, i])
    .sort(([a, i], [b, j]) => dayIndex.get(dayOf(a)) - dayIndex.get(dayOf(b)) || (b.score ?? 0) - (a.score ?? 0) || i - j)
    .map(([r]) => r);
}

// ---- Hide replacements, read later today ---------------------------------

const hideReplaced = (section) => section === "research" && S.hideUpdates;

/// Unix seconds of this Mac's last midnight: the "Read later" queue holds what was queued
/// since then, so it empties itself overnight without a timer.
D.midnight = function midnight() {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
};

/// The oldest "Read later" mark still on the list (Settings → Reading → Keep Read
/// later for; 1 = today only, 0 = no limit), as Unix seconds.
/// "for a week", for the Later list's notes.
function laterKept() {
  const days = S.settings?.later_days ?? 7;
  return ({ 0: "kept until you open it", 1: "kept until midnight", 3: "kept for 3 days", 7: "kept for a week",
    14: "kept for 2 weeks", 30: "kept for a month" })[days] || `kept for ${days} days`;
}

D.laterSince = function laterSince() {
  const days = S.settings?.later_days ?? 7;
  if (!days) return 0;
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - (days - 1));
  return Math.floor(d.getTime() / 1000);
};

/// This week's Monday, 00:00 on this Mac (Unix seconds).
D.monday = function monday() {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return Math.floor(d.getTime() / 1000);
};

function lastWeek() {
  const to = D.monday();
  return { from: to - 7 * 86400, to };
}

// ---- Suggested, what you missed, pinned searches --------------------------

/// The Suggested chip: the best papers of the past 7 days not opened yet, across all topics.
D.showSuggested = function showSuggested() {
  const now = Math.floor(Date.now() / 1000);
  D.showExtra({ kind: "suggested", from: now - 7 * 86400, to: now + 1 });
};

/// Monday's note: last week's best papers not opened. Seen once, the note goes for the week.
D.showMissed = function showMissed() {
  localStorage.setItem("dispatch.missedSeen", String(D.monday()));
  S.missed = 0;
  D.showExtra({ kind: "missed", ...lastWeek() });
};

D.dismissMissed = function dismissMissed() {
  localStorage.setItem("dispatch.missedSeen", String(D.monday()));
  S.missed = 0;
  D.draw();
};

D.showExtra = function showExtra(x) {
  if (S.view !== "research") D.showView("research");
  S.topic = null;
  S.later = false;
  S.extra = x;
  S.selected = null;
  S.picked = new Set();
  D.loadItems();
};

/// Pins (or unpins) a Search query as a Research chip.
D.pinSearch = async function pinSearch(query, on) {
  try {
    const pinned = await invoke("pin_search", { query, on });
    S.topics = { ...S.topics, pinned };
    D.toast(on ? `Pinned “${query}” to Research` : `Unpinned “${query}”`);
    if (!on && S.extra?.kind === "pin" && S.extra.query === query) D.setTopic(null);
    await D.loadCounts();
    drawListHead();
  } catch (e) { D.toast(`Couldn't pin: ${e}`); }
};

/// On today's queue.
D.isLater = (r) => !!r?.later_at && new Date(r.later_at).getTime() >= D.laterSince() * 1000;

D.toggleUpdates = function toggleUpdates() {
  S.hideUpdates = !S.hideUpdates;
  localStorage.setItem("dispatch.hideUpdates", S.hideUpdates ? "1" : "");
  D.toast(S.hideUpdates ? "Hiding updated papers (arXiv replacements)" : "Showing updated papers");
  D.loadCounts();
  D.loadItems();
};

/// `l`: the "Read later" list, for the selection (or the cursor). Not Saved (nothing goes in
/// dispatch.bib): it keeps papers as long as Settings says, or until you open them.
D.toggleLater = async function toggleLater(keys = D.targets()) {
  if (!keys.length) return;
  const rows = S.rows.filter((r) => keys.includes(r.key));
  const on = !rows.every(D.isLater);
  await setLater(keys, on);
  const what = keys.length === 1 ? "" : ` ${keys.length}`;
  D.toast(on ? `Read later${what} ⏱` : `Off the later list${what}`, () => setLater(keys, !on));
};

async function setLater(keys, on) {
  await invoke("set_later", { keys, on });
  D.markLater(keys, on);
  if (S.later) await D.loadItems();
  else { drawList(); drawPreview(); }
  D.loadCounts();
}

/// Keeps the rows on screen in step without reloading (triage uses it too).
D.markLater = function markLater(keys, on) {
  const set = new Set(keys);
  for (const r of S.rows) if (set.has(r.key)) r.later_at = on ? new Date().toISOString() : null;
};

D.showLater = function showLater(on = !S.later) {
  S.later = on;
  S.extra = null;
  S.selected = null;
  S.picked = new Set();
  D.loadItems();
};

/// An unread item that arrived before today (a leftover), greyed a little.
function earlier(r) {
  if (!S.settings?.dim_earlier || r.read || S.later || S.extra || !D.section()) return false;
  return new Date(r.first_seen).getTime() < D.midnight() * 1000;
}

/// `g`: the next topic (after the current one) that has unread items.
D.nextUnreadTopic = function nextUnreadTopic() {
  const section = D.section();
  if (!section) return;
  const counts = S.unread[section] || {};
  const ids = sectionTopics().map((t) => t.id);
  const from = ids.indexOf(S.topic);
  for (let step = 1; step <= ids.length; step++) {
    const id = ids[(from + step + ids.length) % ids.length];
    if ((counts[id] ?? 0) > 0) {
      if (id !== S.topic) D.setTopic(id);
      D.toast(`${sectionTopics().find((t) => t.id === id).name}: ${counts[id]} unread`);
      return;
    }
  }
  D.toast("Nothing unread in any topic.");
};

/// A `dispatch://paper/…` link for the paper under the cursor.
D.copyPaperLink = function copyPaperLink() {
  const key = D.targets()[0];
  if (!key?.startsWith("arxiv:")) return;
  D.settingsView.copy(`dispatch://paper/${key.slice(6)}`, "the paper's Dispatch link");
};

D.toggleForYou = function toggleForYou() {
  S.forYou = !S.forYou;
  localStorage.setItem("dispatch.forYou", S.forYou ? "1" : "");
  D.toast(S.forYou ? "Sorted for you" : "Sorted newest first");
  D.loadItems();
};

D.loadItems = async function loadItems() {
  if (!D.reading()) return;
  if (S.view === "search") {
    const q = S.search;
    const asked = `${q.mode}|${q.query}`;
    const wanted = q.query.trim();
    const [r, tags] = await Promise.all([
      q.mode === "notes" ? invoke("noted_items").then((rows) => {
        const words = wanted.toLowerCase().split(/\s+/).filter(Boolean);
        const hits = rows.filter((x) => words.every((w) => `${x.note} ${x.title} ${x.authors.join(" ")}`.toLowerCase().includes(w)));
        return { hits, total: hits.length };
      })
        : q.mode === "papers" && wanted
          ? invoke("search", { query: q.query, filter: searchFilter(q), limit: 300 })
          : { hits: [], total: 0 },
      invoke("all_tags"),
    ]);
    if (S.view !== "search" || asked !== `${q.mode}|${q.query}`) return; // a newer search is on its way
    S.rows = r.hits;
    S.tags = tags;
    q.total = r.total;
  } else if (S.view === "saved") {
    [S.rows, S.tags] = await Promise.all([invoke("saved_items", { tag: S.tag }), invoke("all_tags")]);
    if (S.tag && !S.tags.some(([t]) => t === S.tag)) { S.tag = null; return D.loadItems(); }
  } else if (S.extra) {
    const x = S.extra;
    const asked = x;
    const rows = x.kind === "pin" ? invoke("pinned_items", { query: x.query }) : invoke("suggested", { from: x.from, to: x.to });
    [S.rows, S.tags] = await Promise.all([rows, invoke("all_tags")]);
    if (S.extra !== asked) return;
  } else if (S.later) {
    [S.rows, S.tags] = await Promise.all([invoke("later_items", { section: D.section(), since: D.laterSince() }), invoke("all_tags")]);
  } else {
    const section = D.section();
    const rank = section === "research" && S.forYou;
    [S.rows, S.tags] = await Promise.all([
      invoke("items", { section, topic: S.topic, limit: 500, rank, hideReplaced: hideReplaced(section) }),
      invoke("all_tags")]);
    if (rank) S.rows = forYouOrder(S.rows);
  }
  if (!D.visibleRows().some((r) => r.key === S.selected)) S.selected = D.visibleRows()[0]?.key ?? null;
  D.draw();
  readSoon();
};

D.loadCounts = async function loadCounts() {
  for (const section of ["research", "news"]) {
    [S.unread[section], S.laterCount[section]] = await Promise.all([
      invoke("unread_counts", { section, hideReplaced: hideReplaced(section) }),
      invoke("later_items", { section, since: D.laterSince() }).then((rows) => rows.length)]);
  }
  S.savedCount = (await invoke("saved_items", { tag: null })).length;
  S.pinCounts = await invoke("pinned_counts").catch(() => []);
  S.missed = 0;
  if (new Date().getDay() === 1 && localStorage.getItem("dispatch.missedSeen") !== String(D.monday())) {
    const w = lastWeek();
    S.missed = (await invoke("suggested", { from: w.from, to: w.to }).catch(() => [])).length;
  }
  drawCounts();
  drawChips();
};

D.loadStatus = async function loadStatus() {
  S.status = await invoke("status");
  drawFooter();
};

D.visibleRows = function visibleRows() {
  if (S.view === "search") return S.rows;
  return D.digestOn() ? digestGroups(S.rows).flatMap((g) => g.rows) : S.rows;
};

const SINCE_DAYS = { week: 7, month: 31, year: 365 };

/// The Search tab looks somewhere narrower than everything.
function scoped() {
  const q = S.search;
  return !!(q.section || q.topic || q.saved || q.unread || q.since !== "any");
}

/// What the Search tab's scope controls ask the archive for (`store::SearchFilter`).
function searchFilter(q) {
  const days = SINCE_DAYS[q.since];
  return {
    section: q.section, topic: q.topic, saved_only: q.saved, unread_only: q.unread, sort: q.sort,
    since: days ? Math.floor(Date.now() / 1000) - days * 86400 : null,
  };
}

// ---- week digest -----------------------------------------------------------------

/// The "no new papers" line shows once per quiet update: from the first
/// time Research shows it until you leave the tab, then not again until the next quiet update
/// (remembered by that update's time, so a restart doesn't bring it back).
function quietNoteDue(ax) {
  const stamp = ax.last_ok || "";
  if (localStorage.getItem("dispatch.quietNote") !== stamp) {
    localStorage.setItem("dispatch.quietNote", stamp);
    S.quietNoteFor = stamp;
  }
  return S.quietNoteFor === stamp;
}

/// True when arXiv's latest update brought no new papers (weekend, holiday).
function quietDay() {
  const ax = S.status?.sources.find((x) => x.source === "arxiv");
  return !!ax && !ax.last_error && ax.last_count === 0;
}

/// The digest shows in Research when chosen with the "This week" chip, or automatically on a
/// quiet day if the setting is on.
D.digestOn = function digestOn() {
  if (D.section() === "news") return !S.later && !S.extra && S.newsDigest;
  if (D.section() !== "research" || S.later || S.extra) return false;
  if (S.digestChoice !== null) return S.digestChoice;
  return !!S.settings?.digest_auto && quietDay();
};

D.toggleDigest = function toggleDigest() {
  if (D.section() === "news") {
    S.newsDigest = !S.newsDigest;
    if (S.later) { S.later = false; S.newsDigest = true; D.loadItems(); return; }
    S.selected = null;
    S.picked = new Set();
    D.draw();
    return;
  }
  S.digestChoice = !D.digestOn();
  if (S.later) { S.later = false; S.digestChoice = true; D.loadItems(); return; }
  S.selected = null;
  S.picked = new Set();
  D.draw();
};

/// The past 7 days, grouped by topic (in topic order; a paper in several topics shows under
/// the first), unread first, then newest.
function digestGroups(rows) {
  // Research: the past week; News: the last 24 hours.
  const since = Date.now() - (D.section() === "news" ? 1 : 7) * 86400e3;
  const recent = rows.filter((r) => new Date(r.published || r.first_seen) >= since);
  const topics = sectionTopics().filter((t) => !S.topic || t.id === S.topic);
  const seen = new Set();
  const groups = [];
  const saved = recent.filter((r) => r.saved);
  if (saved.length && !S.topic) {
    saved.forEach((r) => seen.add(r.key));
    groups.push({ id: "saved", name: "★ Saved", rows: saved });
  }
  for (const t of topics) {
    const mine = recent.filter((r) => r.topics.includes(t.id) && !seen.has(r.key));
    mine.forEach((r) => seen.add(r.key));
    mine.sort((a, b) => (a.read - b.read) || (new Date(b.published || b.first_seen) - new Date(a.published || a.first_seen)));
    if (mine.length) groups.push({ id: t.id, name: t.name, rows: mine });
  }
  return groups;
}

// ---- read state --------------------------------------------------------------

// An item counts as read once it has stayed selected for a second, so skimming with j/k
// doesn't mark everything you pass.
let readTimer = null;
function readSoon() {
  clearTimeout(readTimer);
  const key = S.selected;
  const row = S.rows.find((r) => r.key === key);
  if (!row || row.read) return;
  readTimer = setTimeout(() => D.setRead(key, true), 1000);
}

D.setRead = async function setRead(key, read) {
  const row = S.rows.find((r) => r.key === key);
  if (!row || row.read === read) return;
  await invoke("set_read", { key, read });
  row.read = read;
  // The same story from other feeds goes with it.
  const copies = (row.also || []).filter((a) => a.read !== read).map((a) => a.key);
  if (copies.length) {
    await invoke("set_read_many", { keys: copies, read });
    for (const a of row.also) a.read = read;
  }
  const node = document.querySelector(`.row[data-key="${CSS.escape(key)}"]`);
  if (node) node.classList.toggle("read", read);
  D.loadCounts();
};

D.toggleRead = function toggleRead() {
  clearTimeout(readTimer);
  if (S.picked.size) {
    // Several: mark them all read, unless they already are; then unread.
    const rows = S.rows.filter((r) => S.picked.has(r.key));
    D.setReadMany(rows.map((r) => r.key), !rows.every((r) => r.read));
    return;
  }
  const row = S.rows.find((r) => r.key === S.selected);
  if (row) D.setRead(row.key, !row.read);
};

/// Marks the whole section (or topic) read, with Undo: the app returns exactly the items it
/// changed, and Undo marks only those unread again.
D.markAllRead = async function markAllRead() {
  const section = D.section();
  if (!section) return;
  if (S.extra) {
    const unread = S.rows.filter((r) => !r.read).map((r) => r.key);
    if (!unread.length) { D.toast("Nothing unread here."); return; }
    await D.setReadMany(unread, true);
    D.toast(`Marked ${unread.length} read`, () => D.setReadMany(unread, false));
    return;
  }
  const keys = await invoke("mark_all_read", { section, topic: S.topic });
  if (!keys.length) { D.toast("Nothing unread here."); return; }
  const set = new Set(keys);
  for (const r of S.rows) if (set.has(r.key)) r.read = true;
  drawList();
  drawPreview();
  D.loadCounts();
  D.toast(`Marked ${keys.length} read`, () => D.setReadMany(keys, false));
};

// ---- selection, toast, citations ------------------------------------------------

/// What an action applies to: the multi-selection if there is one, else the cursor.
D.targets = function targets() {
  if (S.picked.size) return [...S.picked];
  return S.selected ? [S.selected] : [];
};

D.selectAll = function selectAll() {
  S.picked = new Set(D.visibleRows().map((r) => r.key));
  drawList();
  drawPreview();
  D.keys?.drawStrip();
};

D.clearPicked = function clearPicked() {
  if (!S.picked.size) return false;
  S.picked = new Set();
  drawList();
  drawPreview();
  D.keys?.drawStrip();
  return true;
};

let toastTimer = null;
/// A short message above the strip; `undo` adds an Undo button.
D.toast = function toast(text, undo = null) {
  clearTimeout(toastTimer);
  const t = $("toast");
  // replaceChildren would show a null as the text "null", so leave it out.
  t.replaceChildren(el("span", { text }),
    ...(undo ? [el("button", { onclick: () => { t.hidden = true; undo(); } }, "Undo")] : []));
  t.hidden = false;
  toastTimer = setTimeout(() => { t.hidden = true; }, undo ? 8000 : 4000);
};

const FORMAT_NAME = { bibtex: "BibTeX", ris: "RIS" };

// ---- Add to Dispatch (in Saved's header, ⌘N) ---------------

/// ⌘N: Saved, with the cursor in the Add box.
D.focusAdd = function focusAdd() {
  if (S.view !== "saved") D.showView("saved");
  setTimeout(() => $("addbox").focus(), 0);
};

let adding = false;
D.addPaper = async function addPaper(input) {
  if (!input.trim() || adding) return;
  adding = true;
  D.toast("Adding…");
  try {
    const a = await invoke("add_paper_now", { input });
    $("addbox").value = "";
    D.toast(`${a.new ? "Added to Saved" : "Already here; saved"}: ${a.title}`);
    await D.loadItems();
    D.loadCounts();
    if (S.rows.some((r) => r.key === a.key)) D.select(a.key);
  } catch (e) { D.toast(`Couldn't add: ${e}`); }
  adding = false;
};

D.copyCitations = async function copyCitations(format = "bibtex") {
  const keys = D.targets();
  if (!keys.length) return;
  D.toast(`Getting ${FORMAT_NAME[format]}…`);
  try {
    const r = await invoke("copy_citations", { keys, format });
    D.toast(`Copied ${FORMAT_NAME[format]} for ${r.count} ${r.count === 1 ? "paper" : "papers"} (${r.origins})`);
  } catch (e) { D.toast(`Couldn't copy: ${e}`); }
};

D.exportCitations = async function exportCitations(format = "bibtex") {
  const keys = D.targets();
  if (!keys.length) return;
  try {
    const r = await invoke("export_citations", { keys, format });
    if (r.path) D.toast(`Saved ${r.count} to ${r.path.split("/").pop()} (${r.origins})`);
  } catch (e) { D.toast(`Couldn't export: ${e}`); }
};

D.openInZotero = async function openInZotero() {
  const keys = D.targets();
  if (!keys.length) return;
  if (!S.zotero) { D.toast("Zotero isn't installed in Applications. Copy BibTeX (b) and use Zotero's File → Import from Clipboard."); return; }
  D.toast("Handing over to Zotero…");
  try {
    const r = await invoke("open_in_zotero", { keys });
    D.toast(`Sent ${r.count} to Zotero: confirm the import there (${r.origins})`);
  } catch (e) { D.toast(`Couldn't open Zotero: ${e}`); }
};

/// Straight into the open Zotero app (its local port), no import dialog.
D.sendToZotero = async function sendToZotero() {
  const keys = D.targets();
  if (!keys.length) return;
  D.toast("Sending to Zotero…");
  try {
    D.toast(await invoke("send_to_zotero", { keys }));
  } catch (e) { D.toast(`${e}`); }
};

// ---- saved, notes, tags -----------------------------------------------------------

/// Send to Zotero works with the Zotero app installed, or with Zotero online set up (16b).
D.canSendZotero = () => !!(S.zotero || S.settings?.zotero_user);

/// Saves the target items, or unsaves them if they are all saved already. Undo puts it back.
D.toggleSaved = async function toggleSaved() {
  const keys = D.targets();
  if (!keys.length) return;
  const rows = S.rows.filter((r) => keys.includes(r.key));
  const save = !rows.every((r) => r.saved);
  await setSaved(keys, save);
  const what = keys.length === 1 ? "" : ` ${keys.length}`;
  D.toast(save ? `Saved${what} ★` : `Removed${what} from Saved`, () => setSaved(keys, !save));
};

async function setSaved(keys, saved) {
  await invoke("set_saved", { keys, saved });
  if (S.view === "saved") { await D.loadItems(); D.loadCounts(); return; }
  const set = new Set(keys);
  for (const r of S.rows) if (set.has(r.key)) r.saved = saved;
  drawList();
  drawPreview();
  D.loadCounts();
}

let noteTimer = null;
/// Notes save as you type (after a short pause) and when you leave the field.
function saveNote(key, text, now = false) {
  clearTimeout(noteTimer);
  const go = async () => {
    await invoke("set_note", { key, note: text });
    const row = S.rows.find((r) => r.key === key);
    if (!row) return;
    // A first note saves the paper (the app does it; mirror it here).
    const savedNow = !row.saved && !row.note && text.trim();
    row.note = text.trimEnd();
    if (savedNow) {
      row.saved = true;
      drawList();
      D.loadCounts();
      D.toast("Saved ★ with your note (it stays even if you remove it from Saved)");
    }
  };
  if (now) go(); else noteTimer = setTimeout(go, 700);
}

async function saveTags(key, text) {
  const tags = await invoke("set_tags", { key, tags: text.split(",") });
  const row = S.rows.find((r) => r.key === key);
  if (row) row.tags = tags;
  S.tags = await invoke("all_tags");
  drawList();
  if (S.view === "saved") drawChips();
}

D.addTag = async function addTag(tag) {
  const keys = D.targets();
  if (!keys.length || !tag.trim()) return;
  await invoke("add_tag", { keys, tag });
  await D.loadItems();
  D.toast(`Tagged ${keys.length} with “${tag.trim().toLowerCase()}”`);
};

D.focusNote = function focusNote() {
  const box = document.querySelector(".note-box");
  if (box) { box.focus(); box.setSelectionRange(box.value.length, box.value.length); }
};

D.setTag = function setTag(tag) {
  S.tag = tag;
  S.selected = null;
  S.picked = new Set();
  D.loadItems();
};

D.setReadMany = async function setReadMany(keys, read) {
  if (!keys.length) return;
  await invoke("set_read_many", { keys, read });
  const set = new Set(keys);
  for (const r of S.rows) if (set.has(r.key)) r.read = read;
  drawList();
  drawPreview();
  D.loadCounts();
};

// ---- search archive ------------------------------------------------------

const SEARCH_MODES = ["papers", "people", "tags", "notes"];
const SEARCH_PLACEHOLDER = {
  papers: "Search everything…", people: "Find a person…", tags: "Filter tags…", notes: "Search your notes…",
};

/// Search → People changed (followed someone, added a keyword): redraw its list.
D.peopleChanged = function peopleChanged() {
  if (S.view === "search" && S.search.mode === "people") drawList();
};

let searchTimer = null;
/// Searches as you type, after a short pause.
D.searchSoon = function searchSoon() {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => { S.selected = null; S.picked = new Set(); D.loadItems(); }, 160);
};

/// Opens Search with `query` (`4`, ⌘F, a menu click). From another tab it looks everywhere.
D.searchFor = function searchFor(query = "") {
  if (S.view !== "search") {
    Object.assign(S.search, { mode: "papers", section: null, topic: null, saved: false, unread: false, since: "any" });
    D.showView("search");
  }
  S.search.query = query;
  $("query").value = query;
  $("query").focus();
  D.loadItems();
};

/// `t`: Search → Tags, every tag with its count; typing filters them.
D.showTags = function showTags() {
  if (S.view === "search") { D.setSearch({ mode: "tags", query: "" }); $("query").value = ""; return; }
  Object.assign(S.search, { mode: "tags", query: "", section: null, topic: null, saved: false, unread: false, since: "any", person: null });
  D.showView("search");
};

/// `/`: Search, looking only where you were: this section
/// and topic, or your saved papers. The scope stays visible and changeable in the list header.
D.searchHere = function searchHere() {
  if (S.view === "search") { $("query").focus(); $("query").select(); return; }
  const scope = S.view === "saved"
    ? { section: null, topic: null, saved: true }
    : { section: D.section(), topic: S.topic, saved: false };
  Object.assign(S.search, { mode: "papers", query: "", unread: false, since: "any" }, scope);
  D.showView("search");
};

D.setSearch = function setSearch(change) {
  if (change.mode && change.mode !== S.search.mode) {
    change.person = null;
    $("query").placeholder = SEARCH_PLACEHOLDER[change.mode];
  }
  Object.assign(S.search, change);
  S.selected = null;
  S.picked = new Set();
  D.loadItems();
};

/// The menu-bar menu's "Show in Dispatch": the item in its section's full list, or found by
/// its key in Search if the list doesn't hold it (an old paper, or its topic is off).
D.openItem = async function openItem(key, section) {
  if (S.view === "topics" && !D.picker.canLeave()) return;
  if (S.view !== section) D.showView(section);
  S.topic = null;
  S.extra = null;
  S.later = false;
  S.digestChoice = false;
  S.selected = key;
  await D.loadItems();
  if (S.rows.some((r) => r.key === key)) D.select(key);
  else D.searchFor(key.replace(/^arxiv:/, ""));
};

/// A snippet from the index: matches sit between \u0002 and \u0003. Built with text nodes
/// and <mark>, never HTML.
function snippetNode(text) {
  const out = el("span", { class: "snippet" });
  for (const [i, part] of text.split(/[\u0002\u0003]/).entries()) {
    if (part) out.append(i % 2 ? el("mark", { text: part }) : part);
  }
  return out;
}

// ---- navigation --------------------------------------------------------------

/// A click: plain = select one; ⌘ = add/remove; ⇧ = range from the cursor.
D.click = function click(key, e) {
  if (e.metaKey || e.ctrlKey) {
    if (!S.picked.size && S.selected) S.picked.add(S.selected);
    if (S.picked.has(key)) S.picked.delete(key); else S.picked.add(key);
    if (S.picked.size === 1) { const only = [...S.picked][0]; S.picked = new Set(); D.select(only, false); return; }
    S.selected = key;
  } else if (e.shiftKey && S.selected) {
    const rows = D.visibleRows().map((r) => r.key);
    const [a, b] = [rows.indexOf(S.selected), rows.indexOf(key)].sort((x, y) => x - y);
    S.picked = new Set(rows.slice(a, b + 1));
    S.selected = key;
  } else {
    S.picked = new Set();
    D.select(key, false);
    return;
  }
  drawList();
  drawPreview();
  D.keys?.drawStrip();
};

D.select = function select(key, scroll = true) {
  if (S.mutedPending && key !== S.mutedPending) {
    S.mutedPending = null;
    S.selected = key;
    D.loadItems().then(() => D.select(key, scroll));
    return;
  }
  if (S.picked.size) { S.picked = new Set(); drawList(); D.keys?.drawStrip(); }
  S.selected = key;
  for (const n of document.querySelectorAll(".row")) {
    n.setAttribute("aria-current", String(n.dataset.key === key));
  }
  if (scroll) document.querySelector(".row[aria-current='true']")?.scrollIntoView({ block: "nearest" });
  drawPreview();
  readSoon();
};

D.move = function move(step) {
  const rows = D.visibleRows();
  if (!rows.length) return;
  const i = rows.findIndex((r) => r.key === S.selected);
  const next = rows[Math.max(0, Math.min(rows.length - 1, (i < 0 ? -1 : i) + step))];
  D.select(next.key);
};

D.showView = function showView(view) {
  if (D.triage?.active) D.triage.stop();
  if (S.view === view) return;
  if (S.view === "topics" && !D.picker.canLeave()) return;
  S.view = view;
  S.topic = null;
  S.selected = null;
  S.mutedPending = null;
  S.quietNoteFor = null; // the quiet-day line has been seen
  S.picked = new Set();
  S.digestChoice = null;
  S.later = false;
  S.extra = null;
  S.tag = null;
  $("query").value = view === "search" ? S.search.query : "";
  $("query").placeholder = SEARCH_PLACEHOLDER[S.search.mode];
  if (view === "topics") {
    D.picker.open();
    D.draw();
  } else if (view === "saved") {
    S.tag = null;
    D.loadItems();
  } else if (view === "settings") {
    D.draw();
    D.settingsView.open();
  } else if (view === "stats") {
    D.draw();
    D.statsView.open();
  } else if (view === "search") {
    D.loadItems();
    $("query").focus();
    $("query").select();
  } else {
    D.loadItems();
  }
};

D.setTopic = function setTopic(id) {
  S.topic = id;
  S.later = false;
  S.extra = null;
  S.selected = null;
  S.picked = new Set();
  D.loadItems();
};

/// `[` / `]`: the next chip in whatever the header shows (topics, Saved's tags, Search's modes).
D.cycleTopic = function cycleTopic(step) {
  const turn = (list, now) => list[(list.indexOf(now) + step + list.length) % list.length];
  if (S.view === "search") {
    D.setSearch({ mode: turn(SEARCH_MODES, S.search.mode) });
    return;
  }
  if (S.view === "saved") {
    D.setTag(turn([null, ...S.tags.map(([t]) => t)], S.tag));
    return;
  }
  const ids = [null, ...sectionTopics().map((t) => t.id)];
  const i = ids.indexOf(S.topic);
  D.setTopic(ids[(i + step + ids.length) % ids.length]);
};

D.openSelected = function openSelected(which = "link") {
  const r = S.rows.find((x) => x.key === S.selected);
  if (which === "pdf") { D.openPdf(r); return; }
  if (r?.link) D.openItemLink(r.key, r.link);
};

/// p: the PDF in the browser; with a PDF app chosen in Settings (or d, `download`), it is
/// downloaded once with a tidy name and opened in that app.
D.openPdf = async function openPdf(r, download = false) {
  if (!r || (!r.pdf && !r.key.startsWith("arxiv:"))) return;
  if (!download && !S.settings?.pdf_app) { if (r.pdf) D.openItemLink(r.key, r.pdf); return; }
  D.toast("Getting the PDF…");
  try {
    const name = await invoke("open_pdf", { key: r.key });
    D.toast(`Opened ${name}`);
    invoke("opened", { key: r.key }).then(() => markOpened(r.key, true)).catch(() => {});
  } catch (e) { D.toast(`Couldn't get the PDF: ${e}`); }
};

/// c / ⌘K: `\cite{key}` or `\bibitem`s for the picked papers, with the live .bib's keys.
D.copyLatex = async function copyLatex(kind = "cite") {
  const keys = D.targets().filter((k) => k.startsWith("arxiv:"));
  if (!keys.length) return;
  try {
    const text = await invoke("latex_snippet", { keys, kind });
    D.settingsView.copy(text, kind === "cite" ? text : (keys.length > 1 ? `${keys.length} \\bibitem entries` : "the \\bibitem"));
  } catch (e) { D.toast(`Couldn't copy: ${e}`); }
};

D.downloadPdf = function downloadPdf() {
  D.openPdf(S.rows.find((x) => x.key === S.selected), true);
};

/// Opens an item's page or PDF and records it for Stats: opening is what counts as read.
D.openItemLink = function openItemLink(key, url) {
  invoke("open_link", { url });
  invoke("opened", { key }).then(() => markOpened(key, true)).catch(() => {});
};

/// The preview's "Don't count": an accidental open goes back to seen.
D.unopen = async function unopen(key) {
  try {
    await invoke("unopened", { key });
    markOpened(key, false);
    D.toast("Not counted as read (still seen)");
  } catch (e) { D.toast(`Couldn't change it: ${e}`); }
};

function markOpened(key, opened) {
  const r = S.rows.find((x) => x.key === key);
  if (opened && D.isLater(r)) { r.later_at = null; D.loadCounts(); } // opening reads it: off the queue
  if (!r || r.opened === opened) return;
  r.opened = opened;
  if (S.selected === key && S.picked.size < 2) {
    const top = $("preview").scrollTop;
    drawPreview();
    $("preview").scrollTop = top;
  }
}

// ---- drawing (every visible state is derived from S here) ----------------

D.draw = function draw() {
  for (const b of document.querySelectorAll(".sections button")) {
    b.setAttribute("aria-selected", String(b.dataset.section === S.view));
  }
  const reading = D.reading();
  const triage = !!D.triage?.active;
  $("reader").hidden = !reading || triage;
  if (D.triage) { $("triage").hidden = !triage; if (triage) D.triage.draw(); }
  $("picker").hidden = S.view !== "topics";
  $("settings").hidden = S.view !== "settings";
  $("stats").hidden = S.view !== "stats";
  $("chips").hidden = !reading;
  $("query").hidden = S.view !== "search";
  $("addbox").hidden = S.view !== "saved";
  drawCounts();
  if (reading) {
    drawChips();
    chipEdges();
    drawList();
    drawPreview();
  } else if (S.view === "topics") {
    D.picker.draw();
  } else if (S.view === "stats") {
    D.statsView.draw();
  } else {
    D.settingsView.draw();
  }
  D.keys?.drawStrip();
};

function sectionTopics() {
  const section = D.section();
  if (!S.topics || !section) return [];
  return S.topics[section].filter((t) => t.enabled);
}

function drawCounts() {
  for (const section of ["research", "news"]) {
    const n = S.unread[section]?.[""] ?? 0;
    $(`count-${section}`).textContent = n ? String(n) : "";
  }
  $("count-saved").textContent = S.savedCount ? String(S.savedCount) : "";
}

/// Fades whichever end of the chip row is cut off, and keeps the pressed chip in view.
function chipEdges() {
  const c = $("chips");
  c.querySelector("[aria-pressed='true']")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  c.classList.toggle("more-left", c.scrollLeft > 2);
  c.classList.toggle("more-right", c.scrollLeft + c.clientWidth < c.scrollWidth - 2);
}
$("chips").addEventListener("scroll", chipEdges, { passive: true });
window.addEventListener("resize", chipEdges);
// A mouse wheel scrolls the chip row sideways (trackpads already do).
$("chips").addEventListener("wheel", (e) => {
  if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) { $("chips").scrollLeft += e.deltaY; e.preventDefault(); }
}, { passive: false });

function drawChips() {
  if (S.view === "search") {
    const q = S.search;
    const chip = (label, pressed, change, title) => el("button", {
      class: "chip", "aria-pressed": String(pressed), title: title || label, onclick: () => D.setSearch(change),
    }, label);
    $("chips").replaceChildren(
      chip("Papers", q.mode === "papers", { mode: "papers" }, "Everything ever fetched"),
      chip("People", q.mode === "people", { mode: "people" }, "Who you follow, watched keywords, any author"),
      chip("Tags", q.mode === "tags", { mode: "tags" }, "Your tags"),
      chip("Notes", q.mode === "notes", { mode: "notes" }, "Every paper you wrote a note on"));
    return;
  }
  if (S.view === "saved") {
    const chip = (tag, label, n) => el("button", {
      class: "chip", "aria-pressed": String(S.tag === tag), onclick: () => D.setTag(tag),
    }, label, n ? el("span", { class: "n", text: String(n) }) : null);
    $("chips").replaceChildren(chip(null, "All", 0), ...S.tags.map(([t, n]) => chip(t, `#${t}`, n)));
    return;
  }
  const section = D.section();
  if (!section) return;
  const counts = S.unread[section] || {};
  const chip = (id, name) => {
    const n = counts[id ?? ""] ?? 0;
    return el("button", {
      class: "chip",
      "aria-pressed": String(!S.later && !S.extra && S.topic === id),
      onclick: () => D.setTopic(id),
    }, name, n ? el("span", { class: "n", text: String(n) }) : null);
  };
  const digest = section === "research"
    ? el("button", { class: "chip digest", "aria-pressed": String(D.digestOn()), title: "The past week by topic, unread first",
      onclick: () => D.toggleDigest() }, "This week")
    : el("button", { class: "chip digest", "aria-pressed": String(D.digestOn()), title: "Today: the last 24 hours by topic, one line per story",
      onclick: () => D.toggleDigest() }, "Today");
  const queued = S.laterCount[section] || 0;
  const later = queued || S.later
    ? el("button", { class: "chip later", "aria-pressed": String(S.later), title: "Read later (l). Kept as long as Settings → Reading says, or until you open the paper.",
      onclick: () => D.showLater() }, "⏱ Later", queued ? el("span", { class: "n", text: String(queued) }) : null)
    : null;
  const extra = (x, label, title, n, cls = "") => el("button", {
    class: `chip ${cls}`, title,
    "aria-pressed": String(!!S.extra && S.extra.kind === x.kind && (x.kind !== "pin" || S.extra.query === x.query)),
    onclick: () => (x.kind === "suggested" ? D.showSuggested() : D.showExtra(x)),
  }, label, n ? el("span", { class: "n", text: String(n) }) : null);
  const research = section === "research";
  const pins = research ? S.pinCounts.map(([q, n]) => extra({ kind: "pin", query: q }, `🔎 ${q}`, `Pinned search: ${q} (unread matches in the last 30 days)`, n, "pin")) : [];
  const suggested = research
    ? extra({ kind: "suggested" }, "Suggested", "The best papers of the past 7 days you haven't opened, across your topics: keywords, people you follow, and papers like the ones you saved or opened")
    : null;
  // Suggested and This week come right after All. At the end of the row they
  // were scrolled out of sight once there were a few topics (the row has no scrollbar).
  $("chips").replaceChildren(...[later, chip(null, "All"), suggested, digest, ...pins, ...sectionTopics().map((t) => chip(t.id, t.name))].filter(Boolean));
}

/// The line above the list: why it might be thin today, and "Mark all read".
function drawListHead() {
  const head = $("list-head");
  const section = D.section();
  const s = S.status;
  let note = "";
  if (S.later) note = `Read later: ${laterKept()}, or until you open each one. l takes one off.`;
  else if (S.extra?.kind === "suggested")
    note = S.rows.length ? "Suggested: the best papers of the past 7 days you haven't opened, across all your topics."
      : "Nothing to suggest yet: save, note or open a few papers (or watch a keyword) and Dispatch learns what you like.";
  else if (S.extra?.kind === "missed") note = "What you missed last week: its best papers you didn't open.";
  else if (S.extra?.kind === "pin") note = `Pinned search “${S.extra.query}”: unread first, then newest.`;
  else if (section === "news" && D.digestOn()) note = "Today: the last 24 hours by topic, one line per story, unread first.";
  else if (section === "research" && s) {
    const ax = s.sources.find((x) => x.source === "arxiv");
    if (s.backfilling) note = "Fetching recent papers from arXiv (one category every 3 s)…";
    else if (ax?.last_error && !S.rows.length) note = `Couldn't reach arXiv: ${ax.last_error}. It will try again.`;
    else if (S.extra) { /* note set above */ }
    else if (D.digestOn())
      note = quietDay()
        ? "Quiet day: arXiv announced no new papers (weekend or holiday). Here's your past week by topic, unread first."
        : "This week by topic, unread first.";
    else if (ax && !ax.last_error && ax.last_count === 0 && S.rows.length && quietNoteDue(ax))
      note = "arXiv announced no new papers in its latest update (weekends and holidays). Showing the most recent.";
  }
  if (S.view === "search") {
    const q = S.search;
    const n = q.total;
    if (q.mode === "notes") {
      head.replaceChildren(el("span", { class: "muted-note", text: `${n} ${n === 1 ? "paper" : "papers"} with a note` }));
      return;
    }
    if (q.mode !== "papers") { head.replaceChildren(); return; }
    const scope = el("select", { class: "text small", title: "Where to look",
      onchange: (e) => D.setSearch({ section: e.target.value === "research" || e.target.value === "news" ? e.target.value : null,
        saved: e.target.value === "saved", topic: null }) },
    ...[["all", "Everywhere"], ["research", "Research"], ["news", "News"], ["saved", "★ Saved only"]].map(([v, label]) =>
      el("option", { value: v, selected: (q.saved ? "saved" : q.section ?? "all") === v }, label)));
    const inSection = q.section ? (S.topics?.[q.section] ?? []).filter((t) => t.enabled !== false) : [];
    const topic = inSection.length
      ? el("select", { class: "text small", title: "Only this topic", onchange: (e) => D.setSearch({ topic: e.target.value || null }) },
        el("option", { value: "", selected: !q.topic }, "All topics"),
        ...inSection.map((t) => el("option", { value: t.id, selected: q.topic === t.id }, t.name)))
      : null;
    const since = el("select", { class: "text small", title: "Published (or first seen) since",
      onchange: (e) => D.setSearch({ since: e.target.value }) },
    ...[["any", "Any time"], ["week", "Past week"], ["month", "Past month"], ["year", "Past year"]].map(([v, label]) =>
      el("option", { value: v, selected: q.since === v }, label)));
    const unread = el("button", { class: "btn small", "aria-pressed": String(q.unread), title: "Only items you haven't read",
      onclick: () => D.setSearch({ unread: !q.unread }) }, q.unread ? "Unread only" : "Read and unread");
    const pinned = (S.topics?.pinned ?? []).includes(q.query.trim().split(/\s+/).join(" "));
    const pin = q.query.trim()
      ? el("button", { class: "btn small", title: "A chip in Research with this search's unread count",
        onclick: () => D.pinSearch(q.query, !pinned) }, pinned ? "📌 Unpin" : "📌 Pin to Research")
      : null;
    head.replaceChildren(el("span", { class: "muted-note", text: !q.query.trim() ? ""
      : n > S.rows.length ? `${n} matches, showing the first ${S.rows.length}` : `${n} ${n === 1 ? "match" : "matches"}` }),
      el("span", { class: "head-tools" }, pin, scope, topic, since, unread,
        el("button", { class: "btn small", title: "Best match puts title and author matches first. Click to switch.",
          onclick: () => D.setSearch({ sort: q.sort === "best" ? "newest" : "best" }) },
        q.sort === "best" ? "Sort: best match" : "Sort: newest")));
    return;
  }
  if (S.view === "saved") {
    head.replaceChildren(el("span", { class: "muted-note",
      text: S.tag ? `${S.rows.length} saved with #${S.tag}` : `${S.rows.length} saved · s saves or removes · notes and tags in the preview` }));
    return;
  }
  const unread = S.rows.some((r) => !r.read);
  const sort = section === "research" && !D.digestOn() && !S.later && !S.extra
    ? el("button", { class: "btn small", title: "For you: watched keywords and people you follow first, then papers like the ones you saved, noted or tagged. Click to switch.",
      onclick: () => D.toggleForYou() }, S.forYou ? "Sort: for you" : "Sort: newest")
    : null;
  const updates = section === "research" && !S.later
    ? el("button", { class: "btn small", "aria-pressed": String(S.hideUpdates), title: "arXiv replacements: new versions of older papers (h)",
      onclick: () => D.toggleUpdates() }, S.hideUpdates ? "Updates hidden" : "Hide updates")
    : null;
  const triage = !S.later && unread
    ? el("button", { class: "btn small", title: "Go through the unread ones: keep, later or skip (e)", onclick: () => D.triage.start() }, "Triage")
    : null;
  const unpin = S.extra?.kind === "pin"
    ? el("button", { class: "btn small", onclick: () => D.pinSearch(S.extra.query, false) }, "Unpin")
    : null;
  const missed = section === "research" && !S.extra && !S.later && S.missed > 0
    ? el("span", { class: "missed-note" },
      el("button", { class: "linkish", onclick: () => D.showMissed() }, `You missed ${S.missed} good ${S.missed === 1 ? "paper" : "papers"} last week → show`),
      el("button", { class: "linkish", title: "Not this week", onclick: () => D.dismissMissed() }, " ×"))
    : null;
  head.replaceChildren(
    missed ?? el("span", { class: "note", text: note }),
    el("span", { class: "head-tools" }, unpin, triage, S.extra ? null : updates, sort,
      el("button", { class: "btn small", disabled: !unread, title: "Mark everything here read (with Undo)", onclick: () => D.markAllRead() }, "Mark all read")));
}

/// Search → People: the list of people (async: it may ask the archive for matching names).
async function drawPeople() {
  const q = S.search;
  const asked = q.query;
  const pick = (name) => { q.person = name; drawPeople(); drawPreview(); };
  const parts = await D.authors.peopleList(q.query, pick, q.person);
  if (S.view !== "search" || q.mode !== "people" || asked !== q.query) return;
  $("list").replaceChildren(...parts.filter(Boolean));
}

/// Search → Tags: every tag with its count; a click searches papers with it.
function drawTagList() {
  const want = S.search.query.trim().toLowerCase();
  const tags = S.tags.filter(([t]) => !want || t.includes(want.replace(/^#/, "")));
  $("list").replaceChildren(tags.length
    ? el("div", {}, ...tags.map(([t, n]) => el("button", {
      class: "row tag-row", onclick: () => { D.setSearch({ mode: "papers", section: null, saved: false }); D.searchFor(`tag:"${t}"`); },
    }, el("span", { class: "title", text: `#${t}` }), el("span", { class: "meta", text: `${n} ${n === 1 ? "item" : "items"}` }))))
    : el("div", { class: "empty", text: S.tags.length ? `No tag matches “${want}”.` : "No tags yet. Add them under a paper's note (comma-separated)." }));
}

function drawList() {
  drawListHead();
  const list = $("list");
  if (S.view === "search" && S.search.mode === "people") { drawPeople(); return; }
  if (S.view === "search" && S.search.mode === "tags") { drawTagList(); return; }
  const rows = D.visibleRows();
  if (!rows.length) {
    const section = D.section();
    if (S.view === "search" && S.search.mode === "notes") {
      list.replaceChildren(el("div", { class: "empty", text: S.search.query.trim()
        ? `No note mentions “${S.search.query.trim()}”.`
        : "No notes yet. Write one under any paper (n); the paper is saved with it." }));
      return;
    }
    if (S.view === "search") {
      list.replaceChildren(S.search.query.trim()
        ? el("div", { class: "empty", text: `Nothing in the archive matches “${S.search.query.trim()}”${scoped() ? " here. Widen the choices above the list." : "."}` })
        : searchHelp());
      return;
    }
    if (S.later) {
      list.replaceChildren(el("div", { class: "empty", text: `Nothing to read later. Press l on anything to put it here (kept ${laterKept()}).` }));
      return;
    }
    if (D.digestOn()) {
      list.replaceChildren(el("div", { class: "empty", text: section === "news" ? "No news in the last 24 hours for these topics." : "Nothing from the past week for these topics yet." }));
      return;
    }
    if (S.view === "saved") {
      list.replaceChildren(el("div", { class: "empty",
        text: "Nothing saved yet. Press s (or ☆ Save in the preview) on anything in Research or News to keep it here, with your notes and tags." }));
      return;
    }
    if (section === "news" && S.topics && !S.topics.news.some((t) => t.enabled && t.sources.some((f) => f.enabled))) {
      list.replaceChildren(el("div", { class: "empty",
        text: "No news feeds yet. Open Topics (⌘T) → News, add a topic and paste the address of any RSS or Atom feed; Test checks it before you save." }));
      return;
    }
    const never = !(section === "research" ? S.status?.research_last_ok : S.status?.news_last_ok);
    const msg = section === "research" && S.status?.backfilling
        ? "Fetching the most recent papers for your topics from arXiv. This takes a few seconds per category."
        : never
          ? "Nothing fetched yet. Press Refresh (or r) to fetch straight away."
          : section === "research"
            ? "No papers for this topic yet. If you just added it, recent papers are on their way; otherwise check its categories and keywords in Topics."
            : "No news for this topic yet.";
    list.replaceChildren(el("div", { class: "empty", text: msg }));
    return;
  }
  const names = S.view === "search"
    ? Object.fromEntries([...(S.topics?.research ?? []), ...(S.topics?.news ?? [])].map((t) => [t.id, t.name]))
    : Object.fromEntries(sectionTopics().map((t) => [t.id, t.name]));
  const out = [];
  let lastDay = null;
  // In the digest a heading starts each topic group; otherwise each day.
  const groupStart = new Map();
  const research0 = D.section() !== "news";
  const compact = D.digestOn() && !research0; // News "Today": one line per story
  if (D.digestOn()) {
    for (const g of digestGroups(rows.length ? S.rows.filter((r) => rows.includes(r)) : [])) {
      const unread = g.rows.filter((r) => !r.read).length;
      groupStart.set(g.rows[0].key, el("div", { class: "group" },
        el("span", { text: g.name }), el("span", { class: "n", text: `${g.rows.length} ${g.rows.length === 1 ? (research0 ? "paper" : "story") : (research0 ? "papers" : "stories")}${unread ? `, ${unread} unread` : ""}` })));
    }
  }
  for (const r of rows) {
    if (D.digestOn()) {
      if (groupStart.has(r.key)) out.push(groupStart.get(r.key));
    } else if (S.view === "search" && (S.search.sort === "best" || S.search.mode === "notes")) {
      // Best match: in score order, so no day headings.
    } else {
      // Saved: by the day it was saved; elsewhere by the day it was published.
      const d = new Date(S.view === "saved" ? r.saved_at : (r.published || r.first_seen));
      const day = d.toDateString();
      if (day !== lastDay) {
        out.push(el("div", { class: "day", text: (S.view === "saved" ? "Saved " : "") + dayLabel(d) }));
        lastDay = day;
      }
    }
    const research = r.section === "research";
    const followed = r.followed?.length ? `👤 ${r.followed.map((n) => n.split(/\s+/).pop()).join(", ")} · ` : "";
    const who = followed + (research
      ? (r.authors.length > 3 ? `${r.authors.slice(0, 3).join(", ")} +${r.authors.length - 3}` : r.authors.join(", "))
      : r.source_name);
    const tag = r.announce_type && r.announce_type !== "new"
      ? el("span", { class: `tag ${r.announce_type.split("-")[0]}`, text: announceLabel(r.announce_type), title: ANNOUNCE[r.announce_type] || "" })
      : null;
    // Published in a journal, and code or data linked in the abstract.
    const published = research && (r.journal_ref || r.doi)
      ? el("span", { class: "tag published", title: r.journal_ref || `DOI ${r.doi}`, text: "published" }) : null;
    const code = r.code?.length
      ? el("span", { class: "tag code", title: r.code.map((c) => c.url).join("\n"), text: r.code.some((c) => c.kind !== "Zenodo" && c.kind !== "Figshare" && c.kind !== "Dryad" && c.kind !== "OSF") ? "code" : "data" })
      : null;
    const topicNames = S.view === "saved" || S.view === "search"
      ? [research ? "Research" : "News", ...r.tags.map((t) => `#${t}`)].join(" · ")
      : [r.topics.map((t) => names[t]).filter(Boolean).join(" · "), ...r.tags.map((t) => `#${t}`)].filter(Boolean).join(" · ");
    const title = el("span", { class: "title" }, r.saved ? el("span", { class: "star", text: "★ " }) : null,
      D.isLater(r) && !S.later ? el("span", { class: "later-mark", text: "⏱ ", title: "Read later" }) : null, r.title);
    if (research) D.math(title);
    // How many other feeds have the same story.
    const also = r.also?.length ? el("span", { class: "tag also", title: r.also.map((a) => a.source_name).join(", "), text: `${r.also.length + 1} sources` }) : null;
    if (compact) {
      out.push(el("button", {
        class: `row compact${r.read ? " read" : ""}${S.picked.has(r.key) ? " picked" : ""}`,
        "data-key": r.key, "aria-current": String(r.key === S.selected), tabindex: "-1",
        onclick: (e) => D.click(r.key, e), ondblclick: () => D.openSelected(),
      }, el("span", { class: "title" }, r.title), el("span", { class: "meta" }, also, r.source_name)));
      continue;
    }
    out.push(el("button", {
      class: `row${r.read ? " read" : ""}${S.picked.has(r.key) ? " picked" : ""}${earlier(r) ? " earlier" : ""}`,
      "data-key": r.key,
      "aria-current": String(r.key === S.selected),
      tabindex: "-1",
      onclick: (e) => D.click(r.key, e),
      ondblclick: () => D.openSelected(),
    }, title, el("span", { class: "meta" }, tag, published, code, also, [who, topicNames].filter(Boolean).join(" · ")),
      S.view === "search" && S.search.mode === "notes" ? el("span", { class: "snippet", text: `✎ ${r.note}` })
        : S.view === "search" && r.snippet?.includes("\u0002") ? snippetNode(r.snippet) : null));
  }
  list.replaceChildren(...out);
}

function searchHelp() {
  const line = (code, what) => el("li", {}, el("code", { text: code }), ` ${what}`);
  return el("div", { class: "empty search-help" },
    el("p", { text: "Search everything Dispatch has ever fetched, offline: titles, authors, abstracts, your notes and tags, arXiv ids and categories. Items whose topic is switched off are included." }),
    el("ul", {},
      line("spin chain", "both words (each also matches longer words: qubit → qubits)"),
      line("\"quantum dot\"", "the exact phrase"),
      line("qubit OR qudit", "either word"),
      line("-review", "without this word"),
      line("au:smith  ti:graphene  abs:laser", "in the authors, title or abstract"),
      line("note:thesis  tag:to-read", "in your notes and tags"),
      line("cat:quant-ph  2410.12345", "a category, or an arXiv id")));
}

function tagOptions() {
  return el("datalist", { id: "tag-options" }, ...S.tags.map(([t]) => el("option", { value: t })));
}

function catName(id) {
  for (const g of S.catalogue?.groups ?? []) {
    const c = g.categories.find((x) => x.id === id);
    if (c) return c.name;
  }
  return null;
}

function drawMulti(pane) {
  const rows = S.rows.filter((r) => S.picked.has(r.key));
  const research = rows.every((r) => r.section === "research");
  const tagInput = el("input", { class: "text", placeholder: "Add a tag to all…", list: "tag-options" });
  tagInput.addEventListener("keydown", (e) => { if (e.key === "Enter") { D.addTag(tagInput.value); tagInput.value = ""; } });
  const b = (label, fn, key) => el("button", { onclick: fn, title: key || label }, label);
  pane.replaceChildren(...[
    el("h1", { text: `${rows.length} selected` }),
    el("div", { class: "who", text: `${rows.filter((r) => !r.read).length} unread · ⌘-click to add or remove, ⇧-click for a range, esc to clear` }),
    el("div", { class: "links" },
      b("Mark read", () => D.setReadMany(rows.map((r) => r.key), true)),
      b("Mark unread", () => D.setReadMany(rows.map((r) => r.key), false)),
      b("Clear selection", () => D.clearPicked(), "esc")),
    el("div", { class: "links" },
      b(rows.every((r) => r.saved) ? "Remove from Saved" : "☆ Save all", () => D.toggleSaved(), "s"),
      tagInput, tagOptions()),
    research ? el("h2", { class: "cite-head", text: "Cite" }) : null,
    research ? el("div", { class: "links" },
      b("Copy BibTeX", () => D.copyCitations("bibtex"), "b"),
      b("Export .bib…", () => D.exportCitations("bibtex")),
      b("Export .ris…", () => D.exportCitations("ris")),
      D.canSendZotero() ? b("Send to Zotero", () => D.sendToZotero()) : null,
      S.zotero ? b("Open in Zotero", () => D.openInZotero()) : null) : null,
    el("ul", { class: "picked-list" }, ...rows.slice(0, 50).map((r) => {
      const li = el("li", { text: r.title });
      if (research) D.math(li);
      return li;
    })),
  ].filter(Boolean));
}

function drawPreview() {
  const pane = $("preview");
  if (S.view === "search" && (S.search.mode === "people" || S.search.mode === "tags")) {
    const who = S.search.mode === "people" ? S.search.person : null;
    if (who && pane.dataset.person === who) return;
    pane.dataset.person = who ?? "";
    if (who) D.authors.show(who, pane);
    else pane.replaceChildren(el("div", { class: "empty", text: S.search.mode === "people"
      ? "Pick someone to see their papers, who they write with, and Follow."
      : "Click a tag to search the papers that carry it." }));
    return;
  }
  pane.dataset.person = "";
  if (S.picked.size > 1) { drawMulti(pane); return; }
  const r = S.rows.find((x) => x.key === S.selected);
  if (!r) { pane.replaceChildren(); return; }
  const research = r.section === "research";
  // `opened`: the page or PDF itself, counted in Stats (a DOI link is not).
  const button = (label, url, key, opened = true) => url && el("button", {
    title: key ? `${label} (${key})` : label,
    onclick: () => (opened ? D.openItemLink(r.key, url) : invoke("open_link", { url })),
  }, label);
  const facts = [];
  facts.push(el("span", { class: "fact", text: D.when(r.published || r.first_seen) }));
  if (research) {
    for (const c of r.categories.slice(0, 8)) {
      facts.push(el("span", { class: "fact fact-cat", text: c, title: catName(c) || c }));
    }
    if (r.announce_type && r.announce_type !== "new") {
      facts.push(el("span", { class: "fact replace", text: announceLabel(r.announce_type), title: ANNOUNCE[r.announce_type] || "" }));
    }
    if (r.version > 1) facts.push(el("span", { class: "fact", text: `v${r.version}` }));
    if (r.journal_ref || r.doi) {
      facts.push(el("span", { class: "fact published", title: r.doi ? `DOI ${r.doi}` : "", text: `Published${r.journal_ref ? `: ${r.journal_ref}` : " (has a DOI)"}` }));
    }
    for (const h of r.hints ?? []) facts.push(el("span", { class: "fact hint", text: h }));
  } else if (r.authors.length) {
    facts.push(el("span", { class: "fact", text: r.authors.join(", ") }));
  }
  const title = el("h1", {}, r.saved ? el("span", { class: "star", text: "★ " }) : null, r.title);
  // A story whose feed gives only a discussion link has no summary of its own.
  const abstract = el("div", { class: "abstract", text: r.summary
    || (r.discussion ? "No summary in the feed: open the story, or its discussion below." : "No summary in the feed.") });
  const note = el("textarea", { class: "text note-box", rows: "3", placeholder: "Your note (n)… saved as you type" });
  note.value = r.note || "";
  note.addEventListener("input", () => saveNote(r.key, note.value));
  note.addEventListener("blur", () => saveNote(r.key, note.value, true));
  const tags = el("input", { class: "text", value: r.tags.join(", "), placeholder: "Tags, comma-separated (e.g. thesis, to-read)", list: "tag-options" });
  tags.addEventListener("change", () => saveTags(r.key, tags.value));
  tags.addEventListener("keydown", (e) => { if (e.key === "Enter") tags.blur(); });
  if (research) { D.math(title); D.math(abstract); }
  // replaceChildren would show a null as the text "null", so leave the empty parts out.
  pane.replaceChildren(...[
    title,
    research && D.authors ? D.authors.authorLine(r) : el("div", { class: "who", text: research ? r.authors.join(", ") : r.source_name }),
    el("div", { class: "facts" }, ...facts),
    abstract,
    r.code?.length ? el("div", { class: "links code-links" },
      el("span", { class: "muted-note", text: "Code & data:" }),
      ...r.code.map((c) => el("button", { title: c.url, onclick: () => invoke("open_link", { url: c.url }) }, `↗ ${c.kind}`))) : null,
    el("div", { class: "links" },
      button(r.key.startsWith("doi:") ? "Journal page" : research ? "arXiv page" : "Open article", r.link, "o"),
      research && (r.pdf || r.key.startsWith("arxiv:")) ? el("button", { title: S.settings?.pdf_app ? `PDF in ${S.settings.pdf_app.split("/").pop().replace(/\.app$/, "")} (p)` : "PDF (p)", onclick: () => D.openPdf(r) }, "PDF") : button("PDF", r.pdf, "p"),
      r.doi && !r.key.startsWith("doi:") ? button("DOI", `https://doi.org/${r.doi}`, null, false) : null,
      el("button", { onclick: () => D.toggleRead(), title: "u" }, r.read ? "Mark unread" : "Mark read"),
      el("button", { class: r.saved ? "saved-btn" : "", onclick: () => D.toggleSaved(), title: "s" }, r.saved ? "★ Saved" : "☆ Save"),
      el("button", { class: D.isLater(r) ? "saved-btn" : "", onclick: () => D.toggleLater([r.key]), title: "Read later (l)" },
        D.isLater(r) ? "⏱ On Later" : "⏱ Later")),
    research ? null : newsExtras(r),
    r.why?.length ? el("p", { class: "why-line", text: `For you: ${r.why.join(" · ")}` }) : null,
    r.opened ? el("p", { class: "opened-line" }, "Read ✓ (you opened it) · ",
      el("button", { class: "linkish", title: "Opened by accident: Stats counts it as seen only", onclick: () => D.unopen(r.key) }, "Don't count"))
      : null,
    el("h2", { class: "cite-head", text: "Notes & tags" }),
    el("div", { class: "notes" }, note, tags, tagOptions()),
    research ? el("h2", { class: "cite-head", text: "Cite" }) : null,
    research ? el("div", { class: "links" },
      el("button", { title: "b", onclick: () => D.copyCitations("bibtex") }, "Copy BibTeX"),
      el("button", { title: "c · the key matches your live .bib file", onclick: () => D.copyLatex("cite") }, "Copy \\cite"),
      el("button", { title: "For a thebibliography list", onclick: () => D.copyLatex("bibitem") }, "Copy \\bibitem"),
      el("button", { onclick: () => D.copyCitations("ris") }, "Copy RIS"),
      el("button", { onclick: () => D.exportCitations("bibtex") }, "Export .bib…"),
      el("button", { onclick: () => D.exportCitations("ris") }, "Export .ris…"),
      el("button", { title: "A dispatch:// link to this paper, for notes, Raycast or Shortcuts", onclick: () => D.copyPaperLink() }, "Copy link"),
      D.canSendZotero() ? el("button", { title: "Into the open Zotero app, or Zotero online when the app is closed", onclick: () => D.sendToZotero() }, "Send to Zotero") : null,
      S.zotero ? el("button", { title: "Zotero imports a BibTeX file (works even if the direct way doesn't)", onclick: () => D.openInZotero() }, "Open in Zotero") : null) : null,
    research ? relatedBox(r) : null,
    research && (S.settings?.s2_on || S.settings?.s2_connected) ? scholarBox(r) : null,
  ].filter(Boolean));
  pane.scrollTop = 0;
}

/// "Related": papers like this one from the whole archive, filled in after the
/// preview is drawn (one local index query).
function relatedBox(r) {
  const box = el("div", { class: "related" });
  invoke("related", { key: r.key }).then((rows) => {
    if (S.selected !== r.key || !rows.length) return;
    box.replaceChildren(el("h2", { class: "cite-head", text: "Related" }),
      ...rows.map((x) => {
        const title = el("span", { class: "rtitle", text: x.title });
        D.math(title);
        return el("button", { class: "related-row", title: "Show this paper", onclick: () => D.openItem(x.key, "research") },
          title, el("span", { class: "meta", text: `${x.authors.slice(0, 2).join(", ")}${x.authors.length > 2 ? " et al." : ""} · ${D.when(x.published || x.first_seen)}${x.read ? "" : " · unread"}` }));
      }));
  }).catch(() => {});
  return box;
}

/// Under a news story: its discussion link, the other feeds with the same story,
/// and muting its feed for a while.
/// arXiv's announce types, in words ("replace cross" was unclear).
const ANNOUNCE = {
  cross: "Cross-listed: a new paper from another category that also lists this one",
  replace: "New version of a paper arXiv announced earlier",
  "replace-cross": "New version of a cross-listed paper from another category",
};
function announceLabel(t) {
  return { cross: "cross-list", replace: "new version", "replace-cross": "new version · cross-list" }[t] || t.replace("-", " ");
}

function newsExtras(r) {
  const parts = [];
  if (r.discussion) {
    const open = () => invoke("open_link", { url: r.discussion.url });
    parts.push(el("p", { class: "news-line" }, el("button", { class: "linkish", title: r.discussion.url, onclick: open }, "Discussion")));
  }
  if (r.also?.length) {
    parts.push(el("p", { class: "news-line" }, "Also in: ",
      ...r.also.flatMap((a, i) => [i ? " · " : "", el("button", { class: "linkish", title: a.link, onclick: () => D.openItemLink(a.key, a.link) }, a.source_name)])));
  }
  // Once muted, the same button unmutes, in case it was a slip.
  const now = new Date();
  const muted = (S.topics?.mute || []).find((m) => m.kind === "source" && m.value === r.source
    && (!m.until || new Date(m.until) > now));
  if (muted) {
    const until = muted.until
      ? `until ${new Date(muted.until).toLocaleDateString(undefined, { day: "numeric", month: "short" })}`
      : "until you unmute it";
    parts.push(el("p", { class: "news-line mute-line" },
      el("button", { class: "btn small", onclick: () => D.mute("source", r.source, r.source_name, "", false) }, `Unmute ${r.source_name}`),
      el("span", { class: "sub", text: ` · muted ${until}; its new stories are hidden` })));
  } else {
    const span = el("select", { class: "text small", title: "For how long" },
      ...[["1d", "for a day"], ["7d", "for a week"], ["30d", "for a month"], ["", "until I unmute it"]].map(([v, l]) => el("option", { value: v }, l)));
    span.value = "7d";
    parts.push(el("p", { class: "news-line mute-line" },
      el("button", { class: "btn small", onclick: () => D.mute("source", r.source, r.source_name, span.value) }, `Mute ${r.source_name}`), " ", span));
  }
  return el("div", { class: "news-extras" }, ...parts);
}

/// Mute (or unmute) a news source or word; its unread stories are marked read.
D.mute = async function mute(kind, value, label, span, on = true) {
  try {
    const mutes = await invoke("set_mute", { kind, value, label, span, on });
    S.topics = { ...S.topics, mute: mutes };
    D.toast(on ? `Muted ${label || value}${span ? ` ${({ "1d": "for a day", "7d": "for a week", "30d": "for a month" })[span] || ""}` : ""}` : `Unmuted ${label || value}`);
    // The story stays (with Unmute) until you move to another item, so the
    // message and the way back are seen; then the muted stories leave the list.
    if (on && S.selected) S.mutedPending = S.selected;
    else { S.mutedPending = null; await D.loadItems(); }
    await D.loadCounts();
    drawPreview(); // the story may still be showing (from Search): Mute becomes Unmute
  } catch (e) { D.toast(`Couldn't mute: ${e}`); }
};

/// Semantic Scholar's similar papers, citing papers and references.
/// Cached lists show at once; otherwise one click fetches all three (about 3 s), never by
/// itself. A paper Dispatch has opens in Dispatch; others open on arXiv or doi.org.
let scholarTab = "similar"; // kept while moving between papers
function scholarBox(r) {
  const box = el("div", { class: "related scholar" });
  let lists = null;
  let busy = false;
  let error = null;
  const tabs = [["similar", "Similar"], ["cited_by", "Cited by"], ["references", "References"]];
  const rowFor = (p) => {
    const title = el("span", { class: "rtitle", text: p.title });
    D.math(title);
    const who = `${p.authors.join(", ")}${p.more ? " et al." : ""}`;
    const meta = [who, p.year, p.citations ? `cited ${p.citations}×` : null, p.key ? "in Dispatch" : null].filter(Boolean).join(" · ");
    const url = p.arxiv ? `https://arxiv.org/abs/${p.arxiv}` : p.doi ? `https://doi.org/${p.doi}` : null;
    return el("button", {
      class: "related-row", title: p.key ? "Show this paper in Dispatch" : url ? `Open ${url}` : "No link known",
      onclick: () => (p.key ? D.openItem(p.key, "research") : url && invoke("open_link", { url })),
    }, title, el("span", { class: "meta", text: meta }));
  };
  const draw = () => {
    if (S.selected !== r.key) return;
    const head = el("div", { class: "scholar-head" }, el("h2", { class: "cite-head", text: "Semantic Scholar" }),
      lists ? el("span", { class: "chips-inline" }, ...tabs.map(([id, label]) => el("button", {
        class: "chip", "aria-pressed": String(scholarTab === id),
        onclick: () => { scholarTab = id; draw(); },
      }, label, el("span", { class: "n", text: String(lists[id].length) + (id === "cited_by" && lists.cited_more ? "+" : "") })))) : null);
    let body;
    const keyless = !S.settings?.s2_connected;
    if (busy) body = el("div", { class: "sub", text: keyless
      ? "Asking Semantic Scholar without a key (often a minute, and it may be busy)…"
      : "Asking Semantic Scholar (about 3 seconds)…" });
    else if (!lists) {
      body = el("div", {}, error ? el("div", { class: "warn-text", text: `⚠ ${error}` }) : null,
        el("button", { class: "btn", onclick: () => get(true) }, "Similar · Cited by · References"),
        keyless ? el("div", { class: "sub", text: "No key: slow, and often busy (Semantic Scholar's shared limit)." }) : null);
    } else {
      const list = lists[scholarTab];
      const empty = scholarTab === "similar" && lists.similar_error ? `No similar papers: ${lists.similar_error}`
        : { similar: "No similar papers yet.", cited_by: "Nobody has cited it yet (or Semantic Scholar hasn't seen it).", references: "No references known." }[scholarTab];
      const when = new Date(lists.fetched * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short" });
      body = el("div", {}, ...(list.length ? list.map(rowFor) : [el("div", { class: "sub", text: empty })]),
        el("div", { class: "sub scholar-foot" }, `Fetched ${when}${scholarTab === "similar" ? "" : " · most cited first"} · `,
          el("button", { class: "linkish", onclick: () => get(true) }, "fetch again")));
    }
    box.replaceChildren(head, body);
  };
  const get = async (fetch) => {
    if (fetch) { busy = true; error = null; draw(); }
    try { lists = (await invoke("s2_lists", { key: r.key, fetch })) || lists; }
    catch (e) { error = `${e}`; }
    busy = false;
    draw();
  };
  get(false);
  return box;
}

function drawFooter() {
  const s = S.status;
  if (!s) return;
  const failed = s.sources.filter((x) => x.last_error);
  $("sources").replaceChildren(
    el("span", { class: `dot ${failed.length ? "fail" : s.sources.length ? "ok" : ""}` }),
    failed.length
      ? `${failed.length} of ${s.sources.length} failed`
      : s.sources.length ? `${s.sources.length} ${s.sources.length === 1 ? "source" : "sources"} ok` : "No sources yet");
  $("last").textContent = s.refreshing || s.backfilling
    ? (s.backfilling ? "Fetching recent arXiv papers…" : "Refreshing…")
    : `arXiv ${s.research_last_ok ? D.when(s.research_last_ok) : "never"} · news ${s.news_last_ok ? D.when(s.news_last_ok) : "never"}`;
  $("last").title = `Next arXiv refresh ${D.when(s.research_next)}, next news refresh ${D.when(s.news_next)}`;
  $("refresh").disabled = s.refreshing || s.backfilling;
  if (D.reading()) drawListHead();
  if (!$("source-pop").hidden) drawSourcePop();
}

function drawSourcePop() {
  const s = S.status;
  $("source-pop").replaceChildren(...s.sources.map((src) => {
    const ok = !src.last_error;
    return el("div", { class: "srow" },
      el("span", { class: `dot ${ok ? "ok" : "fail"}` }),
      el("span", { text: `${src.name} · ${src.last_count ?? 0} items` }),
      el("span", { class: "when", text: src.last_ok ? D.when(src.last_ok) : "never" }),
      ok ? null : el("span", { class: "err", text: src.last_error }));
  }));
}

D.toggleSources = function toggleSources(show = $("source-pop").hidden) {
  $("source-pop").hidden = !show;
  if (show) drawSourcePop();
};

D.refresh = async function refresh() {
  await invoke("refresh_now");
  D.loadStatus();
};

// ---- theme -------------------------------------------------------------------

/// The look comes from the app (Palette's glass or Settings); the page only sets the
/// background's opacity from it.
D.applyLook = function applyLook(look) {
  if (look) D.applyTransparency(look.transparency);
};

D.applyTransparency = function applyTransparency(percent) {
  document.documentElement.style.setProperty("--opacity", `${100 - Math.max(0, Math.min(100, percent || 0))}%`);
};

function applyTheme(t) {
  const root = document.documentElement.style;
  for (const [k, v] of Object.entries(t.colours)) root.setProperty(`--${k}`, v);
  document.documentElement.style.colorScheme = t.mode;
}

// ---- divider: drag to resize the list, double-click to reset ------------------

(function divider() {
  const KEY = "dispatch.listWidth";
  const split = $("reader");
  const bar = $("divider");
  const apply = (px) => {
    const max = split.clientWidth - 320;
    const w = Math.max(260, Math.min(max, px));
    document.documentElement.style.setProperty("--list-width", `${w}px`);
    return w;
  };
  const saved = Number(localStorage.getItem(KEY));
  if (saved) requestAnimationFrame(() => apply(saved));
  bar.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    bar.setPointerCapture(e.pointerId);
    bar.classList.add("dragging");
    const left = split.getBoundingClientRect().left;
    const onMove = (m) => apply(m.clientX - left);
    const onUp = (u) => {
      bar.releasePointerCapture(u.pointerId);
      bar.classList.remove("dragging");
      localStorage.setItem(KEY, String(apply(u.clientX - left)));
      bar.removeEventListener("pointermove", onMove);
      bar.removeEventListener("pointerup", onUp);
    };
    bar.addEventListener("pointermove", onMove);
    bar.addEventListener("pointerup", onUp);
  });
  bar.addEventListener("dblclick", () => {
    localStorage.removeItem(KEY);
    document.documentElement.style.setProperty("--list-width", "42%");
  });
})();

// ---- wiring --------------------------------------------------------------------

for (const b of document.querySelectorAll(".sections button")) {
  b.addEventListener("click", () => D.showView(b.dataset.section));
}
$("refresh").addEventListener("click", () => D.refresh());
$("sources").addEventListener("click", (e) => { e.stopPropagation(); D.toggleSources(); });
document.addEventListener("click", (e) => {
  if (!$("source-pop").hidden && !$("source-pop").contains(e.target)) D.toggleSources(false);
});
$("query").addEventListener("input", () => {
  S.search.query = $("query").value;
  D.searchSoon();
});

listen("refreshing", () => D.loadStatus());
listen("refreshed", () => { D.loadStatus(); D.loadItems(); D.loadCounts(); });
listen("refresh-error", () => D.loadStatus());
listen("theme", (e) => applyTheme(e.payload));
listen("look", async (e) => {
  D.applyLook(e.payload);
  if (S.view === "settings") { await D.settingsView.load(); D.settingsView.draw(); }
});
// dispatch:// links. The app also keeps the last one until the page takes it,
// so a link that started the app is handled once the page has loaded.
listen("link", () => takeLink());
$("addbox").addEventListener("keydown", (e) => {
  if (e.key === "Enter") { e.preventDefault(); D.addPaper(e.target.value); }
});
listen("add-failed", (e) => D.toast(`Couldn't use that link: ${e.payload}`));
async function takeLink() {
  const link = await invoke("take_link");
  if (!link) return;
  if (link.kind === "search") D.searchFor(link.query);
  else if (link.kind === "paper") D.openItem(link.key, "research");
  else if (link.kind === "added") {
    // Add to Dispatch (a Shortcut): shown in Saved, where every added paper is.
    await D.openItem(link.key, "saved");
    D.toast(`Added to Saved: ${link.title}`);
  }
  else if (link.kind === "open" && link.view) {
    if (link.view === "search") D.searchFor(S.search.query); else D.showView(link.view);
  }
}
// From the menu-bar menu.
listen("show-view", (e) => { if (e.payload === "search") D.searchFor(S.search.query); else D.showView(e.payload); });
listen("show-sources", () => { if (!D.reading()) D.showView("research"); setTimeout(() => D.toggleSources(true), 50); });
listen("open-item", (e) => D.openItem(e.payload.key, e.payload.section));
listen("settings", async () => { await D.settingsView.load(); if (D.reading()) D.draw(); });
listen("exports", () => D.settingsView.exportsChanged());
listen("synced", async () => { if (S.view === "settings") { await D.settingsView.load(); D.settingsView.draw(); } });
listen("topics", async () => {
  S.topics = await invoke("get_topics");
  D.loadCounts();
  // A story just muted stays until you move on (D.select reloads then).
  if (S.view !== "topics" && !S.mutedPending) D.loadItems();
});

window.addEventListener("load", async () => {
  applyTheme(await invoke("current_theme"));
  [S.topics, S.catalogue, S.zotero] = await Promise.all([invoke("get_topics"), invoke("arxiv_catalogue"), invoke("has_zotero")]);
  await D.settingsView.load();
  await D.loadStatus();
  await D.loadCounts();
  await D.loadItems();
  takeLink();
  // Keep "today 13:30" and friends current while the window stays open.
  setInterval(() => drawFooter(), 60e3);
});
