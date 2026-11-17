// Settings (the Settings tab, or ⌘,). Each change saves at once, as in macOS's own settings
// windows: the app clamps the values, writes settings.json and adds or removes the login item.
"use strict";

(function () {
  const { el, $, invoke, state: S } = window.D;

  let view = null; // { settings, login_available, data_dir, version }
  let storage = null; // { bytes, research, news, oldest }
  let exports = {}; // { bib, bib_ok, notes, notes_ok }
  let pdfApps = []; // known PDF apps installed on this Mac

  async function load() {
    [view, storage, exports, pdfApps] = await Promise.all([invoke("get_settings"), invoke("storage_info").catch(() => null),
      invoke("export_status").catch(() => ({})), invoke("pdf_apps").catch(() => [])]);
    S.settings = view.settings;
    D.applyLook(view.look);
  }

  async function save(change) {
    const next = { ...view.settings, ...change };
    try {
      view = await invoke("save_settings", { settings: next });
      S.settings = view.settings;
      D.toast("Saved");
    } catch (e) {
      D.toast(`Couldn't save: ${e}`);
    }
    draw();
    D.loadStatus();
  }

  function select(value, options, onchange) {
    return el("select", { class: "text", onchange: (e) => onchange(Number(e.target.value)) },
      ...options.map(([v, label]) => el("option", { value: String(v), selected: v === value }, label)));
  }

  function row(title, hint, control) {
    return el("div", { class: "setting" },
      el("div", {}, el("div", { class: "stitle", text: title }),
        hint instanceof Node ? el("small", {}, hint) : hint ? el("small", { text: hint }) : null),
      control);
  }

  function toggle(checked, onchange, disabled = false) {
    return el("input", { type: "checkbox", class: "switch", checked, disabled,
      onchange: (e) => onchange(e.target.checked) });
  }

  /// Moves the look live while dragging (transparency); saves when released.
  function slider(key, value, max, disabled) {
    const input = el("input", { type: "range", min: "0", max: String(max), step: "5", value: String(value), class: "range", disabled });
    if (key === "transparency") input.addEventListener("input", () => D.applyTransparency(Number(input.value)));
    input.addEventListener("change", () => save({ [key]: Number(input.value) }));
    return input;
  }

  /// Settings → Look: from Palette (`palette glass dispatch`) or these controls.
  function lookRows(s) {
    const look = view.look;
    const pal = look.by_palette;
    const fromPalette = pal ? " Set by Palette." : "";
    const KIND = { solid: "Solid", blurred: "Blurred", clear: "Clear" };
    // Make it obvious when Palette is in charge of the glass.
    const banner = pal ? el("div", { class: "palette-banner" },
      el("strong", { text: `🎨 Glass set by Palette: ${KIND[look.kind] ?? look.kind}` }),
      el("span", { text: ` · ${look.transparency} % see-through${look.blur ? ` · blur ${look.blur}` : ""}. ` }),
      el("span", { text: "Change it in Palette → Apps → Dispatch → Glass, or choose “Yours” there to use the sliders below." }))
      : null;
    const rows = [
      banner,
      row("Transparency", `${look.transparency}%: how much of the desktop shows through the window. 0 is solid.${fromPalette}`,
        slider("transparency", look.transparency, 100, pal)),
      row("Blur", look.blur
        ? `${look.blur}: how strongly the desktop behind is blurred (like Ghostty's background-blur; 20 is Ghostty's default). 0 uses the Frost style below.${fromPalette}`
        : `Off: the Frost style below does the blurring. Above 0, a blur of that strength replaces it (like Ghostty's background-blur).${fromPalette}`,
        slider("blur", look.blur, 100, pal)),
      view.blur_problem ? el("p", { class: "warn-line", text: `⚠ Blur: ${view.blur_problem}.` }) : null,
      row("Frost style", look.material === null
        ? "Palette chose clear glass: see-through with no blur."
        : "Apple's frosted materials, used when Blur is 0 (or if the blur ever stops working). Shows when Transparency is above 0.",
        el("select", { class: "text", disabled: look.material === null || (look.blur > 0 && !view.blur_problem), onchange: (e) => save({ frost: e.target.value }) },
          ...[["window", "Window: light blur"], ["sidebar", "Sidebar: more colour through"], ["popover", "Pop-over: milky"], ["hud", "HUD: heaviest, dark"]]
            .map(([v, label]) => el("option", { value: v, selected: v === s.frost }, label)))),
      row("Solid when not in front",
        "The glass switches off while another app is in front, so a Dispatch window left open in a tile costs no GPU for it. Off: always glass.",
        toggle(s.solid_in_background, (v) => save({ solid_in_background: v }))),
      row("Colours", view.theme_file
        ? `From Palette's ${view.theme_file}${view.theme_file === "dispatch.json" ? " (Dispatch's own file)" : " (Palette has no Dispatch file yet: update Palette)"}.`
        : "Built-in Tokyo Night (Palette isn't installed or has no current theme).", null),
    ];
    return rows;
  }

  // ---- dispatch:// links ----

  const LINKS = [
    ["dispatch://search?q=spin%20chain", "Search for words (use %20 for spaces)"],
    ["dispatch://paper/2410.01234", "Show a paper by its arXiv id"],
    ["dispatch://open/research", "Open a tab: research, news, saved, search or stats"],
    ["dispatch://refresh", "Refresh now, without opening the window"],
    ["dispatch://add?q=2410.01234", "Save a paper: an arXiv link or id, or a DOI"],
  ];

  async function copy(text, what) {
    try { await invoke("copy_text", { text }); D.toast(`Copied ${what}`); } catch (e) { D.toast(`Couldn't copy: ${e}`); }
  }

  function linkRows() {
    return [
      el("h2", { text: "Links" }),
      el("p", { class: "about tight", text: "Links that open Dispatch from anywhere: Raycast, Shortcuts (Open URLs), SketchyBar, a note, or Terminal with open \"…\". Copy one and change the words or id. A paper's own link is in its preview (Copy link) and ⌘K." }),
      ...LINKS.map(([link, what]) => row(what, link, el("div", { class: "row-tools" },
        el("button", { class: "btn", title: "Copy the link", onclick: () => copy(link, "the link") }, "Copy"),
        el("button", { class: "btn", title: "For Terminal or a shell script", onclick: () => copy(`open "${link}"`, "the Terminal command") }, "Copy for Terminal")))),
    ];
  }

  // ---- research workflow ----

  // The browser, the known PDF apps that are installed, the chosen one if it isn't
  // among them (picked with "Other app…", shown by its name), then "Other app…".
  async function chooseDocuments() {
    try {
      const msg = await invoke("choose_documents");
      if (!msg) return;
      D.toast(msg);
      await load();
      draw();
    } catch (e) { D.toast(`Couldn't change the folder: ${e}`); }
  }

  const OTHER = "__other__";
  const appLabel = (a) => (a.startsWith("/") ? a.split("/").pop().replace(/\.app$/, "") : a);
  function pdfAppOptions(chosen) {
    const apps = [["", "Browser"], ...pdfApps.map((a) => [a, a])];
    if (chosen && !apps.some(([v]) => v === chosen)) apps.push([chosen, appLabel(chosen)]);
    return [...apps, [OTHER, "Other app…"]];
  }
  async function choosePdfApp(value, select) {
    if (value !== OTHER) { save({ pdf_app: value }); return; }
    const path = await invoke("pick_app").catch((e) => { D.toast(`Couldn't choose: ${e}`); return null; });
    if (path) save({ pdf_app: path });
    else select.value = view.settings.pdf_app; // cancelled: show the current choice again
  }

  async function choose(command, args, key) {
    try {
      const path = await invoke(command, args);
      if (path) save({ [key]: path });
    } catch (e) { D.toast(`Couldn't choose: ${e}`); }
  }

  function status(text, ok) {
    return text ? el("span", { class: ok ? "" : "warn-text", text: ` ${ok ? "" : "⚠ "}${text}.` }) : null;
  }

  async function readingLog() {
    try { D.toast(await invoke("reading_log")); } catch (e) { D.toast(`Couldn't make the log: ${e}`); }
  }

  function workflowRows(s) {
    const tags = view.tags || [];
    const off = (key) => el("button", { class: "btn", onclick: () => save({ [key]: "" }) }, "Turn off");
    return [
      el("h2", { text: "Research workflow" }),
      row("Dispatch documents",
        el("span", {}, s.docs_dir
          ? `${s.docs_dir}: your .bib file and downloaded PDFs. Move… takes Dispatch's files there with it (nothing else in the folder is touched).`
          : "One folder for the .bib file and the PDFs Dispatch downloads, for LaTeX or Overleaf (Git). Until you choose one, PDFs go to Downloads."),
        el("button", { class: "btn", onclick: chooseDocuments }, s.docs_dir ? "Move…" : "Choose…")),
      row(".bib file",
        el("span", {}, !s.docs_dir
          ? "Choose the Dispatch documents folder first."
          : s.bib_on
            ? `${s.bib_name} in that folder holds your saved papers (★): saving adds one and unsaving takes it out, within a few seconds. Read later doesn't add to it. Never replaces a .bib of yours.`
            : `Off. On: ${s.bib_name} in that folder, kept up to date with your saved papers (turning it off leaves the file there).`,
          status(s.bib_on && s.docs_dir && exports.bib, exports.bib_ok)),
        toggle(s.bib_on, (v) => save({ bib_on: v }), !s.docs_dir)),
      s.bib_on && s.docs_dir ? row("Papers in it", "Every saved paper, or only those with one tag.",
        el("select", { class: "text", onchange: (e) => save({ bib_tag: e.target.value }) },
          el("option", { value: "", selected: !s.bib_tag }, "All saved papers"),
          ...tags.map((t) => el("option", { value: t, selected: t === s.bib_tag }, `Tag: ${t}`)))) : null,
      row("Citation keys", "Used in the .bib file and when copying \\cite{…} (c) or a \\bibitem (⌘K).",
        el("select", { class: "text", onchange: (e) => save({ cite_keys: e.target.value }) },
          ...[["author", "smith2024adiabatic"], ["arxiv", "2410.01234 (arXiv id)"]]
            .map(([v, label]) => el("option", { value: v, selected: v === s.cite_keys }, label)))),
      row("Notes folder (Obsidian)",
        el("span", {}, s.notes_dir
          ? `One Markdown note per saved paper in ${s.notes_dir}. A note you edit there is left alone from then on.`
          : "One Markdown note per saved paper (abstract, tags, your note) in a folder of your Obsidian vault. Notes you edit are never overwritten.",
          status(s.notes_dir && exports.notes, exports.notes_ok)),
        el("div", { class: "row-tools" },
          el("button", { class: "btn", onclick: () => choose("pick_folder", { title: "Choose a folder for paper notes (e.g. in your Obsidian vault)" }, "notes_dir") }, s.notes_dir ? "Move…" : "Choose…"),
          s.notes_dir ? off("notes_dir") : null)),
      row("Reading log", s.notes_dir
        ? "This week's notes as one Markdown file: copied, and written to the notes folder."
        : "This week's notes as one Markdown file, copied for pasting anywhere.",
        el("button", { class: "btn", onclick: readingLog }, "This week's log")),
      row("PDF app", `Where p opens a paper's PDF. The browser shows it online; an app needs the file on disk, so Dispatch downloads it once (2024 Smith - Title.pdf) into ${s.docs_dir ? "Dispatch documents" : "Downloads"} and opens it from there. d always downloads.`,
        el("select", { class: "text", onchange: (e) => choosePdfApp(e.target.value, e.target) },
          ...pdfAppOptions(s.pdf_app).map(([v, label]) => el("option", { value: v, selected: v === s.pdf_app }, label)))),
      ...zoteroRows(s),
      row("Deadlines", "Conferences and journals, in deadlines.json: shown in the menu bar and in Stats (the next 90 days).",
        el("button", { class: "btn", onclick: () => invoke("edit_deadlines").catch((e) => D.toast(`${e}`)) }, "Edit deadlines…")),
    ];
  }

  // ---- Zotero online ----

  let collections = null; // [{ key, path }] once asked

  async function zoteroConnect(input) {
    const key = input.value.trim();
    if (!key) { D.toast("Paste your Zotero API key first"); return; }
    D.toast("Checking the key with zotero.org…");
    try {
      const name = await invoke("zotero_connect", { key });
      input.value = "";
      collections = null;
      D.toast(`Connected to Zotero as ${name}`);
      await load();
    } catch (e) { D.toast(`Couldn't connect: ${e}`); }
    draw();
  }

  async function zoteroDisconnect() {
    try { await invoke("zotero_disconnect"); D.toast("Zotero online forgotten (key removed from the Keychain)"); } catch (e) { D.toast(`${e}`); }
    collections = null;
    await load();
    draw();
  }

  async function loadCollections() {
    try { collections = await invoke("zotero_collections"); } catch (e) { collections = []; D.toast(`Couldn't list collections: ${e}`); }
    draw();
  }

  // ---- news: mutes ----

  function newsRows() {
    const now = Date.now();
    const mutes = (S.topics?.mute || []).filter((m) => !m.until || new Date(m.until) > now);
    const word = el("input", { class: "text", placeholder: "A word or phrase, e.g. crypto", autocomplete: "off", spellcheck: "false" });
    const span = el("select", { class: "text small" },
      ...[["1d", "for a day"], ["7d", "for a week"], ["30d", "for a month"], ["", "until I unmute it"]].map(([v, l]) => el("option", { value: v }, l)));
    span.value = "7d";
    const add = async () => {
      const w = word.value.trim();
      if (!w) { D.toast("Type a word first"); return; }
      await D.mute("word", w, w, span.value);
      draw();
    };
    word.addEventListener("keydown", (e) => { if (e.key === "Enter") add(); });
    const until = (m) => (m.until ? `until ${new Date(m.until).toLocaleDateString(undefined, { day: "numeric", month: "short" })}` : "until you unmute it");
    return [
      el("h2", { text: "News" }),
      row("Mute a word", "News stories with this word or phrase in the title are hidden and marked read (whole words, any case). A source is muted from its story's preview.",
        el("div", { class: "row-tools" }, word, span, el("button", { class: "btn", onclick: add }, "Mute"))),
      ...mutes.map((m) => row(m.kind === "source" ? `Source: ${m.label || m.value}` : `Word: ${m.label || m.value}`, `Muted ${until(m)}.`,
        el("button", { class: "btn", onclick: async () => { await D.mute(m.kind, m.value, m.label, "", false); draw(); } }, "Unmute"))),
    ];
  }

  // ---- discovery: Semantic Scholar ----

  function discoveryRows(s) {
    const head = el("h2", { text: "Discovery" });
    const on = s.s2_on || s.s2_connected;
    const rows = [head, row("Semantic Scholar",
      el("span", {}, "Similar papers, references and who cites a paper, in its preview when you ask (nothing is fetched until you click). Dispatch sends only the paper's arXiv id or DOI, to api.semanticscholar.org. Free, no account needed.",
        on && !s.s2_connected ? el("span", { class: "warn-text", text: " ⚠ Without a key it's often busy: a list can take a minute, or fail with \u201cbusy, try again\u201d." }) : null),
      toggle(on, (v) => save({ s2_on: v })))];
    if (!on) return rows;
    if (!s.s2_connected) {
      const input = el("input", { type: "password", class: "text", placeholder: "API key (optional)", autocomplete: "off", spellcheck: "false",
        onkeydown: (e) => { if (e.key === "Enter") s2Connect(input); } });
      rows.push(row("API key (optional)",
        el("span", {}, "Without a key, Dispatch shares Semantic Scholar's public limit with everyone else. A personal key (",
          el("button", { class: "linkish", onclick: () => invoke("open_link", { url: "https://www.semanticscholar.org/product/api#api-key-form" }) }, "apply here"),
          ") makes it faster; it's kept in the macOS Keychain."),
        el("div", { class: "row-tools" }, input, el("button", { class: "btn", onclick: () => s2Connect(input) }, "Connect"))));
    } else {
      rows.push(row("API key", "Connected with your key: lists come in about 3 s.",
        el("button", { class: "btn", onclick: s2Disconnect }, "Disconnect")));
    }
    return rows;
  }

  async function s2Connect(input) {
    const key = input.value.trim();
    if (!key) { D.toast("Paste your Semantic Scholar key first"); return; }
    D.toast("Checking the key with Semantic Scholar…");
    try {
      await invoke("s2_connect", { key });
      input.value = "";
      D.toast("Connected to Semantic Scholar");
      await load();
    } catch (e) { D.toast(`Couldn't connect: ${e}`); }
    draw();
  }

  async function s2Disconnect() {
    try { await invoke("s2_disconnect"); D.toast("Key removed from the Keychain; Semantic Scholar goes on without a key (turn it off above)"); } catch (e) { D.toast(`${e}`); }
    await load();
    draw();
  }

  function zoteroRows(s) {
    if (!s.zotero_user) {
      const input = el("input", { type: "password", class: "text", placeholder: "API key from zotero.org", autocomplete: "off", spellcheck: "false",
        onkeydown: (e) => { if (e.key === "Enter") zoteroConnect(input); } });
      return [row("Zotero online",
        el("span", {}, "For Send to Zotero when the Zotero app isn't open. Make a key with \"Allow library access\" and \"Allow write access\" ",
          el("button", { class: "linkish", onclick: () => invoke("open_link", { url: "https://www.zotero.org/settings/keys/new" }) }, "on zotero.org"),
          ". It's kept in the macOS Keychain; Dispatch then contacts api.zotero.org only when you send."),
        el("div", { class: "row-tools" }, input, el("button", { class: "btn", onclick: () => zoteroConnect(input) }, "Connect")))];
    }
    if (collections === null) loadCollections();
    const list = collections || [];
    return [
      row("Zotero online", `Connected as ${s.zotero_name || s.zotero_user}. Send to Zotero uses the Zotero app when it's open, otherwise this.`,
        el("button", { class: "btn", onclick: zoteroDisconnect }, "Disconnect")),
      row("Collection", collections === null ? "Loading your collections…" : "Where papers sent online go.",
        el("select", { class: "text", onchange: (e) => {
          const c = list.find((x) => x.key === e.target.value);
          save({ zotero_collection: c?.key ?? "", zotero_collection_name: c?.path ?? "" });
        } },
        el("option", { value: "", selected: !s.zotero_collection }, "My Library (no collection)"),
        ...(list.some((c) => c.key === s.zotero_collection) || !s.zotero_collection ? [] : [el("option", { value: s.zotero_collection, selected: true }, s.zotero_collection_name)]),
        ...list.map((c) => el("option", { value: c.key, selected: c.key === s.zotero_collection }, c.path)))),
    ];
  }

  // ---- backups ----

  async function backupNow() {
    try {
      await invoke("backup_now");
      D.toast("Backed up (the newest five are kept)");
    } catch (e) { D.toast(`Couldn't back up: ${e}`); }
    if (S.view === "settings") { await load(); draw(); }
  }

  async function exportBackup() {
    try {
      const path = await invoke("export_backup");
      if (path) D.toast(`Exported to ${path}`);
    } catch (e) { D.toast(`Couldn't export: ${e}`); }
  }

  async function restore(name) {
    const when = name.replace("_", " ").replace(/^(\d{4}-\d\d-\d\d) (\d\d)(\d\d).*/, "$1 $2:$3");
    try {
      const before = await invoke("restore_backup", { name, when });
      if (!before) return;
      D.toast(`Restored ${when}. Your state from just before is backup ${before}.`);
      await D.loadCounts();
    } catch (e) { D.toast(`Couldn't restore: ${e}`); }
    await load();
    draw();
  }

  function backupRows() {
    const list = view.backups || [];
    const label = (b) => {
      const mb = (b.bytes / 1e6).toFixed(1);
      return `${b.at ? D.when(b.at) : b.name} · ${mb} MB`;
    };
    return [
      row("Backups", list.length
        ? `One a day by itself, newest five kept, in ${view.data_dir}/backups. Latest: ${label(list[0])}.`
        : "One a day by itself (database, topics, settings), the newest five kept. None yet.",
        el("div", { class: "row-tools" },
          el("button", { class: "btn", onclick: backupNow }, "Back up now"),
          el("button", { class: "btn", title: "Write a backup folder somewhere you choose (kept until you delete it)", onclick: exportBackup }, "Export…"))),
      list.length ? row("Restore", "Puts back papers, read state, saved items, notes, tags and topics. Settings stay as they are.",
        el("select", { class: "text", onchange: (e) => { if (e.target.value) restore(e.target.value); e.target.value = ""; } },
          el("option", { value: "" }, "Choose a backup…"),
          ...list.map((b) => el("option", { value: b.name }, label(b))))) : null,
    ];
  }

  function syncLine() {
    const st = view.sync;
    if (st?.error) return `Problem: ${st.error}`;
    const r = st?.last;
    if (!r) return "Not yet. It runs within half a minute, then whenever something changes.";
    if (r.skipped) return `${D.when(r.at)}: ${r.skipped}`;
    const bits = [`${D.when(r.at)}`];
    bits.push(r.pulled ? `${r.pulled} changes from your other Mac` : "up to date");
    if (r.topics_updated) bits.push("topics updated");
    if (r.conflicts) bits.push(`${r.conflicts} iCloud conflict ${r.conflicts === 1 ? "copy" : "copies"} merged`);
    return bits.join(" · ");
  }

  /// The sync check: kept across launches, unlike the line above.
  function syncCheck() {
    const st = view.sync || {};
    const merged = st.last_merge ? `Last merged ${D.when(st.last_merge)}` : "Not merged yet on this Mac";
    const n = st.conflicts_total || 0;
    return `${merged}. ${n ? `${n} iCloud conflict ${n === 1 ? "copy" : "copies"} resolved so far (the copies are in sync-conflicts/ in the data folder).` : "No iCloud conflicts so far."}`;
  }

  async function syncNow() {
    try {
      await invoke("sync_now");
      D.toast("Synced with iCloud");
    } catch (e) {
      D.toast(`Sync failed: ${e}`);
    }
    await load();
    draw();
  }

  function storageLine() {
    if (!storage) return "";
    const mb = storage.bytes / 1e6;
    const size = mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb.toFixed(mb < 10 ? 1 : 0)} MB`;
    const since = storage.oldest ? ` since ${new Date(storage.oldest).toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" })}` : "";
    return `${size}: ${storage.research.toLocaleString()} papers and ${storage.news.toLocaleString()} news items${since} (about 5 KB a paper, search index included).`;
  }

  async function cleanUp() {
    D.toast("Cleaning up…");
    try {
      const n = await invoke("prune_now");
      D.toast(n ? `Removed ${n.toLocaleString()} old items; space given back.` : "Nothing old enough to remove.");
    } catch (e) { D.toast(`Couldn't clean up: ${e}`); }
    await load();
    draw();
  }

  /// Stats count from now on; Undo puts the previous start back. Nothing is deleted.
  async function resetStats() {
    try {
      const before = await invoke("reset_stats", { backTo: null });
      const redraw = () => { if (S.view === "stats") D.statsView.open(); };
      D.toast("Stats now count from today. Unread marks, saved papers and notes are untouched.", async () => {
        await invoke("reset_stats", { backTo: before });
        redraw();
      });
      redraw();
    } catch (e) { D.toast(`Couldn't reset: ${e}`); }
  }

  /// Stats: the optional weekly goal and the Habit Tracker link.
  function statsRows(s) {
    const goals = [0, 5, 10, 20, 30, 50];
    if (!goals.includes(s.weekly_goal)) goals.push(s.weekly_goal); // a hand-edited value
    return [
      row("Weekly goal", "Papers to read each week, Monday to Sunday. Shown in Stats and the menu-bar menu; no reminders.",
        select(s.weekly_goal, goals.map((g) => [g, g ? `${g} papers` : "Off"]), (v) => save({ weekly_goal: v }))),
      row("Habit Tracker link",
        `Keeps ${view.data_dir}/reading-log.json up to date (papers read, seen, saved and noted per day, last 400 days), so a habit in the Habit Tracker can fill itself from Dispatch.`,
        toggle(s.habit_export, (v) => save({ habit_export: v }))),
      row("Reset stats", "Stats count from today. Read means a paper's page or PDF was opened. Nothing is deleted, and Undo brings the old numbers back.",
        el("button", { class: "btn", onclick: resetStats }, "Reset")),
    ];
  }

  // ---- menu bar ----
  const SYMBOLS = { ctrl: "⌃", alt: "⌥", shift: "⇧", cmd: "⌘" };
  const pretty = (combo) => (combo
    ? combo.split("+").map((p) => SYMBOLS[p] || (p.length === 1 ? p.toUpperCase() : p[0].toUpperCase() + p.slice(1))).join("")
    : "None");

  /// `KeyT` → `t`, `Digit5` → `5`, `ArrowUp` → `up`, `F5` → `f5` (as in Palette).
  function keyName(code) {
    if (/^Key[A-Z]$/.test(code)) return code.slice(3).toLowerCase();
    if (/^Digit\d$/.test(code)) return code.slice(5);
    if (/^F\d{1,2}$/.test(code)) return code.toLowerCase();
    const named = { Space: "space", ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Tab: "tab", Enter: "enter" };
    return named[code] || null;
  }

  let recording = false; // the shortcut button is listening
  async function setHotkey(combo) {
    try {
      const now = await invoke("set_hotkey", { combo });
      D.toast(now ? `Search from anywhere: ${pretty(now)}` : "No shortcut");
    } catch (e) { D.toast(String(e)); }
    await load();
    draw();
  }
  // Capture phase, so Dispatch's own shortcuts don't see the keys while recording.
  document.addEventListener("keydown", (e) => {
    if (!recording) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.key === "Escape") { recording = false; draw(); return; }
    const key = keyName(e.code);
    if (!key) return; // a modifier on its own: keep waiting
    const combo = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift", e.metaKey && "cmd", key].filter(Boolean).join("+");
    recording = false;
    setHotkey(combo);
  }, true);

  function menuBarRows(s) {
    const active = view.hotkey_active;
    const keyBtn = el("button", { class: "btn" + (recording ? " recording" : ""), onclick: (e) => {
      recording = true;
      e.currentTarget.textContent = "Press keys… (Esc cancels)";
    } }, recording ? "Press keys… (Esc cancels)" : pretty(active));
    const hint = active
      ? "Opens Dispatch on Search from any app. Click the keys to change them, then press the new ones."
      : s.hotkey ? `${pretty(s.hotkey)} couldn't be used (another app may have it). Click to choose others.` : "Off.";
    return [
      el("h2", { text: "Menu bar" }),
      row("Beside the icon", "What Dispatch's menu-bar icon shows. Unread counts only grow, so the others are calmer; "
        + "the dot means new papers since you last opened the window.",
        el("select", { class: "text", onchange: (e) => save({ menu_bar_count: e.target.value }) },
          ...[["off", "Nothing"], ["dot", "A dot for new papers"], ["suggested", "Suggested papers left"],
            ["foryou", "Today's papers by people and keywords you follow"], ["goal", "Weekly goal (papers read / goal)"],
            ["later", "On the Read later list"], ["research", "Unread papers"], ["all", "Unread papers and news"]]
            .map(([v, label]) => el("option", { value: v, selected: v === s.menu_bar_count }, label)))),
      row("Red dot when a source fails", "The icon turns red while a feed's last fetch failed; the menu's Sources list can retry it.",
        toggle(s.menu_bar_alert, (v) => save({ menu_bar_alert: v }))),
      row("Search from anywhere", hint,
        el("div", { class: "row-tools" }, keyBtn,
          active ? el("button", { class: "btn", onclick: () => setHotkey("") }, "Turn off") : null,
          active !== "ctrl+alt+s" ? el("button", { class: "btn", onclick: () => setHotkey("ctrl+alt+s") }, "Use ⌃⌥S") : null)),
      ...barRows(s),
    ];
  }

  /// status.json and the SketchyBar pill (Palette → Bar → add the Dispatch item).
  function barRows(s) {
    const on = s.status_file;
    const chosen = s.bar_metrics || [];
    const flip = (id, v) => {
      // Keep Settings' order, so the pill reads the same way as the list.
      const next = view.bar_metrics.map(([m]) => m).filter((m) => (m === id ? v : chosen.includes(m)));
      save({ bar_metrics: next });
    };
    const rows = [row("Status for SketchyBar",
      "Writes status.json in the data folder for Palette's Dispatch bar item (Palette → Bar). Off: the pill hides.",
      toggle(on, (v) => save({ status_file: v })))];
    if (on) {
      rows.push(row("The pill shows", view.bar_preview ? `Now: ${view.bar_preview}` : "Nothing to show yet.",
        el("div", { class: "checks" }, ...view.bar_metrics.map(([id, label]) =>
          el("label", { class: "check" }, el("input", { type: "checkbox", checked: chosen.includes(id),
            onchange: (e) => flip(id, e.target.checked) }), " " + label)))));
    }
    return rows;
  }

  // ---- data health, export, network log ----
  let health = null; // null = not checked yet, "busy", or the result
  let netlog = null;
  async function checkHealth() {
    health = "busy";
    draw();
    try { health = await invoke("data_health"); } catch (e) { health = null; D.toast(`Couldn't check: ${e}`); }
    draw();
  }
  async function compact() {
    D.toast("Compacting…");
    try {
      const freed = await invoke("compact_now");
      D.toast(freed > 1e6 ? `Compacted: ${(freed / 1e6).toFixed(1)} MB given back.` : "Compacted; it was already tidy.");
    } catch (e) { D.toast(`Couldn't compact: ${e}`); }
    await load();
    draw();
  }
  async function exportAll() {
    try {
      const dir = await invoke("export_everything");
      if (dir) D.toast(`Exported to ${dir}`);
    } catch (e) { D.toast(`Couldn't export: ${e}`); }
  }
  async function showNetlog() {
    netlog = netlog ? null : await invoke("network_log").catch(() => []);
    draw();
  }

  function healthDetails() {
    if (health === "busy") return [el("p", { class: "health-line", text: "Checking…" })];
    if (!health) return [];
    const out = [];
    const ok = health.integrity === "ok";
    out.push(el("p", { class: "health-line" + (ok ? "" : " bad"), text: ok ? "Database: no problems found (SQLite's integrity check)." : `Database problem: ${health.integrity}. Restore a backup below.` }));
    for (const src of health.sources) {
      const bits = [`${src.items.toLocaleString()} items`];
      bits.push(src.newest ? `newest ${D.when(src.newest)}` : "nothing yet");
      if (src.error) bits.push(`last fetch failed: ${src.error}`);
      else if (src.quiet) bits.push(src.section === "research" ? "nothing new for 4+ days" : "nothing new for a week: check the feed in Topics");
      if (src.unfollowed) bits.push("no longer in your topics");
      out.push(el("p", { class: "health-line" + (src.error || src.quiet ? " warn" : "") },
        el("b", { text: src.name }), ` · ${bits.join(" · ")}`));
    }
    const few = (list) => `${list.slice(0, 5).join(", ")}${list.length > 5 ? "…" : ""}`;
    if (health.orphaned.length) out.push(el("p", { class: "health-line warn", text: `${health.orphaned.length} saves or notes for papers this Mac doesn't have (from sync): ${few(health.orphaned)}. They stay; nothing is lost.` }));
    if (health.orphaned_notes.length) out.push(el("p", { class: "health-line warn", text: `${health.orphaned_notes.length} Obsidian notes for papers no longer saved (Dispatch leaves them): ${few(health.orphaned_notes)}.` }));
    if (!health.orphaned.length && !health.orphaned_notes.length) out.push(el("p", { class: "health-line", text: "Notes: every note belongs to a paper." }));
    return out;
  }

  function netlogDetails() {
    if (!netlog) return [];
    if (!netlog.length) return [el("p", { class: "health-line", text: "Nothing logged yet (the log starts with this version)." })];
    return netlog.map((h) => el("p", { class: "health-line" + (h.failed ? " warn" : "") },
      el("b", { text: h.host }), ` · ${h.why} · ${h.count} ${h.count === 1 ? "request" : "requests"}${h.failed ? ` (${h.failed} failed)` : ""} · last ${D.when(h.last)}`));
  }

  function healthRows() {
    return [
      row("Data health", "Items per source, quiet feeds, notes without a paper, and SQLite's own check of the database. Read only.",
        el("div", { class: "row-tools" },
          el("button", { class: "btn", onclick: checkHealth }, health && health !== "busy" ? "Check again" : "Check"),
          el("button", { class: "btn", onclick: compact }, "Compact database"))),
      ...healthDetails(),
      row("Export everything", "Plain files any program reads: dispatch.json (every paper and story with your saves, notes, tags and read state, plus topics and settings) and saved.csv (saved papers, for a spreadsheet).",
        el("button", { class: "btn", onclick: exportAll }, "Export…")),
      row("Network log", "Every host Dispatch contacted in the last 7 days, how often and why. Kept on this Mac only: host names, never addresses or keys.",
        el("button", { class: "btn", onclick: showNetlog }, netlog ? "Hide" : "Show")),
      ...netlogDetails(),
    ];
  }

  function draw() {
    const pane = $("settings");
    if (!view) { pane.replaceChildren(); return; }
    const s = view.settings;
    const hours = [1, 2, 3, 4, 6, 8, 12, 24].map((h) => [h, h === 1 ? "every hour" : `every ${h} hours`]);
    const days = [1, 2, 3, 4, 5, 6, 7].map((d) => [d, d === 1 ? "every day" : `every ${d} days`]);
    pane.replaceChildren(el("div", { class: "settings-body" },
      el("h2", { text: "Refresh" }),
      row("News", "How often news feeds are fetched while Dispatch runs (at least hourly).",
        select(s.news_every_hours, hours, (v) => save({ news_every_hours: v }))),
      row("arXiv", `At ${new Date(Date.UTC(2026, 0, 5, 5, 30)).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })} your time (05:30 UTC), just after arXiv publishes. Its feed changes once a day.`,
        select(s.arxiv_every_days, days, (v) => save({ arxiv_every_days: v }))),

      el("h2", { text: "Look" }),
      ...lookRows(s),

      ...menuBarRows(s),

      el("h2", { text: "Reading" }),
      row("Hide old read papers",
        "Research and News leave out read papers older than this. Saved papers always stay; everything is still in Search.",
        select(s.hide_read_after_days, [[0, "Never"], [3, "after 3 days"], [7, "after a week"], [14, "after 2 weeks"], [30, "after a month"]],
          (v) => save({ hide_read_after_days: v }))),
      row("Dim papers from earlier days",
        "Unread papers that arrived before today are greyed a little, so today's stand out.",
        toggle(s.dim_earlier, (v) => save({ dim_earlier: v }))),
      row("Keep Read later for",
        "How long a paper stays on the ⏱ Later list (l). It also leaves when you open its page or PDF. Later never adds to dispatch.bib: only Save (s) does.",
        select(s.later_days, [[1, "Until midnight"], [3, "3 days"], [7, "A week"], [14, "2 weeks"], [30, "A month"], [0, "Until I open it"]],
          (v) => save({ later_days: v }))),
      row("Week digest on quiet days",
        "When arXiv announced nothing (weekends, holidays), Research opens on the past week by topic, unread first.",
        toggle(s.digest_auto, (v) => save({ digest_auto: v }))),
      ...statsRows(s),

      ...newsRows(),
      ...workflowRows(s),
      ...discoveryRows(s),

      ...(D.authors ? D.authors.settingsSection(row, toggle, s, save) : []),

      el("h2", { text: "Sync" }),
      row("Sync with iCloud Drive",
        view.icloud_available
          ? "Your topics, saved items, notes, tags and read state, through one file in iCloud Drive → Dispatch. Turn it on on each Mac. Fetched papers and these settings stay on each Mac."
          : "iCloud Drive isn't turned on for this Mac (System Settings → Apple Account → iCloud → iCloud Drive).",
        toggle(s.icloud_sync, (v) => save({ icloud_sync: v }), !view.icloud_available)),
      s.icloud_sync ? row("Last sync", syncLine(), el("button", { class: "btn", onclick: syncNow }, "Sync now")) : null,
      s.icloud_sync ? row("Sync check", syncCheck(), null) : null,

      el("h2", { text: "Start-up" }),
      row("Launch at login",
        view.login_available
          ? "Starts in the menu bar only (no window) when you log in. Remove it here or in System Settings → General → Login Items."
          : "Available in the installed app (Applications), not in a development run.",
        toggle(s.launch_at_login, (v) => save({ launch_at_login: v }), !view.login_available)),

      ...linkRows(),

      el("h2", { text: "Data" }),
      row("Size", storageLine(), null),
      row("Archive limit",
        "Removes items older than this, unless saved, noted or tagged, and gives the space back. Checked once a day.",
        el("div", { class: "row-tools" },
          select(s.archive_months, [[0, "Keep everything"], [6, "6 months"], [12, "1 year"], [24, "2 years"]],
            (v) => save({ archive_months: v })),
          s.archive_months ? el("button", { class: "btn", onclick: cleanUp }, "Clean up now") : null)),
      ...backupRows(),
      ...healthRows(),
      row("Folder", view.data_dir,
        el("button", { class: "btn", onclick: () => invoke("reveal_data_folder") }, "Show in Finder")),
      el("p", { class: "about" },
        `Dispatch ${view.version}. Everything stays on this Mac: it only contacts the feeds you `
        + "follow, arXiv, doi.org when you copy a citation, and Zotero or Semantic Scholar if you connected them. "
        + "Network log (above) lists every host it contacted this week. No accounts, no tracking."),
    ));
  }

  D.settingsView = {
    async open() { await load(); draw(); },
    draw,
    load,
    resetStats,
    backupNow,
    copy,
    readingLog,
    async exportsChanged() {
      if (S.view !== "settings") return;
      exports = await invoke("export_status").catch(() => ({}));
      draw();
    },
  };
})();
