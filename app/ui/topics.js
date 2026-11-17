// The topic picker (Topics tab, or t). Edits a draft copy of topics.json; Save (⌘S) sends
// it to the app, which checks it, writes it, retags everything and fetches only what's new.
//
// Typing in a field updates the draft without redrawing (so the field keeps focus); adding,
// removing or toggling redraws. The save bar is derived from `dirty()` on every draw.
"use strict";

(function () {
  const { el, $, invoke, state: S } = window.D;

  const P = {
    saved: null,      // JSON of the topics as last saved
    draft: null,      // the copy being edited
    sel: null,        // { section: "research" | "news", id }
    error: "",
    tests: {},        // feed url -> { state: "testing" | "ok" | "fail", text }
    menu: false,      // the "add research topic" chooser is open
  };

  const clone = (x) => JSON.parse(JSON.stringify(x));
  // Compared in a normal form, so an empty "only papers mentioning" map (absent in the
  // file, {} after an edit) never counts as a change.
  const norm = (topics) => JSON.stringify(topics, (k, v) =>
    k === "category_include" && v && !Object.keys(v).length ? undefined : v);
  const dirty = () => P.draft && norm(P.draft) !== P.saved;
  const list = (t) => t.split(",").map((s) => s.trim()).filter(Boolean);
  const words = (a) => (a || []).join(", ");

  function current() {
    if (!P.sel) return null;
    return P.draft[P.sel.section].find((t) => t.id === P.sel.id) || null;
  }

  function uniqueId(base) {
    const slug = (base || "topic").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "topic";
    const taken = new Set([...P.draft.research, ...P.draft.news].map((t) => t.id));
    let id = slug;
    for (let n = 2; taken.has(id); n++) id = `${slug}-${n}`;
    return id;
  }

  function catName(id) {
    for (const g of S.catalogue?.groups ?? []) {
      const c = g.categories.find((x) => x.id === id);
      if (c) return c.name;
    }
    return "";
  }

  // Edits that change structure: update, then redraw the picker.
  function change(fn) {
    fn();
    P.error = "";
    draw();
  }

  // Edits from typing: update only, then refresh the save bar.
  function typed(fn) {
    fn();
    P.error = "";
    drawSavebar();
    D.keys?.drawStrip();
  }

  // ---- left: topic list ------------------------------------------------------

  function drawList() {
    const row = (section, t) => el("div", {
      class: `prow${t.enabled ? "" : " off"}`,
      "aria-current": String(P.sel?.section === section && P.sel?.id === t.id),
      onclick: () => change(() => { P.sel = { section, id: t.id }; P.menu = false; }),
    },
      el("input", {
        type: "checkbox", class: "switch", checked: t.enabled, title: t.enabled ? "On" : "Off",
        onclick: (e) => e.stopPropagation(),
        onchange: (e) => change(() => { t.enabled = e.target.checked; }),
      }),
      el("span", { class: "name", text: t.name || "Untitled" }),
      el("span", { class: "sub", text: section === "research"
        ? String(t.categories.filter((c) => c.enabled).length)
        : String(t.sources.filter((f) => f.enabled).length) }));

    $("picker-list").replaceChildren(
      el("h2", { text: "Research" }),
      ...P.draft.research.map((t) => row("research", t)),
      el("button", { class: "add", onclick: () => change(() => { P.menu = true; P.sel = null; }) }, "+ Add research topic"),
      el("h2", { text: "News" }),
      ...P.draft.news.map((t) => row("news", t)),
      el("button", { class: "add", onclick: () => change(addNews) }, "+ Add news topic"),
    );
  }

  function addNews() {
    const t = { id: uniqueId("news"), name: "New topic", enabled: true, sources: [], include: [], exclude: [] };
    P.draft.news.push(t);
    P.sel = { section: "news", id: t.id };
  }

  function addResearch(group) {
    let t;
    if (group) {
      t = {
        id: uniqueId(group.id),
        name: group.name,
        enabled: true,
        categories: group.categories.map((c) => ({ id: c.id, enabled: true })),
        include: [], exclude: [],
      };
      const narrow = Object.fromEntries(group.categories.filter((c) => c.match?.length).map((c) => [c.id, c.match]));
      if (Object.keys(narrow).length) t.category_include = narrow;
    } else {
      t = { id: uniqueId("topic"), name: "New topic", enabled: true, categories: [], include: [], exclude: [] };
    }
    P.draft.research.push(t);
    P.sel = { section: "research", id: t.id };
    P.menu = false;
  }

  // ---- right: editor -------------------------------------------------------------

  function drawGroupMenu() {
    const have = new Set(P.draft.research.map((t) => t.name));
    return [
      el("h2", { text: "Add a research topic" }),
      el("p", { class: "field" }, el("small", {
        text: "Pick a group to start from: its arXiv categories come switched on, and you can fine-tune them next." })),
      el("div", { class: "menu" },
        el("button", { onclick: () => change(() => addResearch(null)) }, "Empty topic",
          el("span", { class: "sub", text: "add categories yourself" })),
        ...S.catalogue.groups.map((g) => el("button", {
          onclick: () => change(() => addResearch(g)),
        }, g.name, el("span", { class: "sub", text: (have.has(g.name) ? "already added · " : "") + g.categories.map((c) => c.id).join(", ") })))),
    ];
  }

  function keywordFields(t) {
    return [
      el("label", { class: "field" },
        el("span", { text: "Include: show only items that mention one of these (comma-separated)" }),
        el("input", { class: "text", value: words(t.include), placeholder: "e.g. adiabatic, quantum dot",
          oninput: (e) => typed(() => { t.include = list(e.target.value); }) })),
      el("label", { class: "field" },
        el("span", { text: "Exclude: hide items that mention any of these" }),
        el("input", { class: "text", value: words(t.exclude), placeholder: "e.g. optical fibre",
          oninput: (e) => typed(() => { t.exclude = list(e.target.value); }) })),
    ];
  }

  function titleRow(t, section) {
    return el("div", { class: "title-row" },
      el("input", { class: "text", value: t.name, "aria-label": "Topic name",
        oninput: (e) => typed(() => { t.name = e.target.value; drawList(); }) }),
      el("label", { class: "field", style: "margin:0; display:flex; gap:8px; align-items:center" },
        el("input", { type: "checkbox", class: "switch", checked: t.enabled,
          onchange: (e) => change(() => { t.enabled = e.target.checked; }) }),
        el("span", { text: t.enabled ? "On" : "Off" })),
      el("button", { class: "btn danger", onclick: () => change(() => {
        P.draft[section] = P.draft[section].filter((x) => x !== t);
        P.sel = null;
      }) }, "Delete"));
  }

  function drawResearch(t) {
    // Drawing never changes the draft (it used to add an empty map here, which made a saved
    // topic look unsaved). Edits create the map when they need it.
    const narrow = t.category_include || {};
    const setNarrow = (id, w) => {
      if (w.length) (t.category_include ||= {})[id] = w;
      else if (t.category_include) delete t.category_include[id];
    };
    const allOn = t.categories.length && t.categories.every((c) => c.enabled);
    const cats = t.categories.map((c) => el("div", { class: "cat" },
      el("input", { type: "checkbox", checked: c.enabled,
        onchange: (e) => change(() => { c.enabled = e.target.checked; }) }),
      el("code", { text: c.id }),
      el("span", { class: "cname", text: catName(c.id) || "(not in the catalogue)" }),
      el("button", { class: "x", title: "Remove", onclick: () => change(() => {
        t.categories = t.categories.filter((x) => x !== c);
        setNarrow(c.id, []);
      }) }, "×"),
      c.enabled && el("input", { class: "text narrow", value: words(narrow[c.id]),
        placeholder: "Only papers mentioning… (optional, comma-separated)",
        oninput: (e) => typed(() => {
          setNarrow(c.id, list(e.target.value));
        }) })));

    const addInput = el("input", { class: "text", list: "cat-options", placeholder: "Add a category: type a name or id, e.g. physics.optics" });
    const add = () => {
      const raw = addInput.value.trim();
      const id = raw.split(" ")[0];
      if (!/^[a-z][a-zA-Z0-9.-]*$/.test(id)) { P.error = `“${raw}” isn't an arXiv category id`; drawSavebar(); return; }
      if (t.categories.some((c) => c.id === id)) { addInput.value = ""; return; }
      change(() => t.categories.push({ id, enabled: true }));
    };
    addInput.addEventListener("keydown", (e) => { if (e.key === "Enter") add(); });
    const options = el("datalist", { id: "cat-options" },
      ...[...new Map((S.catalogue?.groups ?? []).flatMap((g) => g.categories.map((c) => [c.id, c]))).values()]
        .map((c) => el("option", { value: `${c.id} — ${c.name}` })));

    return [
      titleRow(t, "research"),
      el("h2", { text: "arXiv categories" }),
      el("label", { class: "cat" },
        el("input", { type: "checkbox", checked: allOn, onchange: (e) => change(() => {
          for (const c of t.categories) c.enabled = e.target.checked;
        }) }),
        el("span", { class: "cname", style: "grid-column: 2 / 5", text: "All categories in this topic" })),
      el("div", { class: "cats" }, ...cats),
      el("div", { class: "addrow" }, addInput, options, el("button", { class: "btn", onclick: add }, "Add")),
      el("h2", { text: "Keywords" }),
      ...keywordFields(t),
    ];
  }

  function drawNews(t) {
    const feeds = t.sources.map((f) => {
      const test = P.tests[f.url];
      return el("div", { class: "feed" },
        el("input", { type: "checkbox", checked: f.enabled, onchange: (e) => change(() => { f.enabled = e.target.checked; }) }),
        el("input", { class: "text", value: f.name, placeholder: "Name", oninput: (e) => typed(() => { f.name = e.target.value; }) }),
        el("input", { class: "text", value: f.url, placeholder: "https://…/feed", spellcheck: "false",
          oninput: (e) => typed(() => { f.url = e.target.value.trim(); }) }),
        el("button", { class: "btn", onclick: () => testFeed(f) }, test?.state === "testing" ? "Testing…" : "Test"),
        el("button", { class: "x", title: "Remove", onclick: () => change(() => { t.sources = t.sources.filter((x) => x !== f); }) }, "×"),
        test && test.state !== "testing" ? el("div", { class: `result ${test.state}`, text: test.text }) : null);
    });
    return [
      titleRow(t, "news"),
      el("h2", { text: "Feeds" }),
      el("div", { class: "feeds" }, ...feeds),
      el("div", { class: "addrow" }, el("button", { class: "btn", onclick: () => change(() => {
        t.sources.push({ name: "", url: "", enabled: true });
      }) }, "+ Add feed"), el("small", { class: "field", style: "margin:0;align-self:center",
        text: "Tip: try the site's address with /feed, /rss or /rss.xml, then press Test." })),
      el("h2", { text: "Keywords" }),
      ...keywordFields(t),
    ];
  }

  async function testFeed(f) {
    const url = f.url;
    P.tests[url] = { state: "testing" };
    draw();
    try {
      const r = await invoke("test_feed", { url });
      P.tests[url] = r.items
        ? { state: "ok", text: `Works: ${r.items} items. Latest: ${r.first ?? ""}` }
        : { state: "fail", text: "It's a feed, but it has no items right now." };
    } catch (e) {
      P.tests[url] = { state: "fail", text: `Doesn't work: ${e}` };
    }
    draw();
  }

  function drawEditor() {
    const pane = $("picker-edit");
    if (P.menu) { pane.replaceChildren(...drawGroupMenu()); return; }
    const t = current();
    if (!t) {
      pane.replaceChildren(el("div", { class: "empty",
        text: "Choose a topic on the left to edit it, or add one. Switch a topic off to hide its items without losing them." }));
      return;
    }
    pane.replaceChildren(...(P.sel.section === "research" ? drawResearch(t) : drawNews(t)));
  }

  function drawSavebar() {
    const bar = $("savebar");
    if (P.error) {
      bar.replaceChildren(el("span", { class: "msg err", text: P.error }),
        ...(dirty() ? [el("button", { class: "btn", onclick: discard }, "Discard")] : []));
      return;
    }
    if (!dirty()) {
      bar.replaceChildren(el("span", { class: "msg", text: "Changes are saved to topics.json. New categories or feeds are fetched as soon as you save." }));
      return;
    }
    bar.replaceChildren(
      el("span", { class: "msg", text: "Unsaved changes" }),
      el("button", { class: "btn", onclick: discard }, "Discard"),
      el("button", { class: "btn primary", onclick: save }, "Save  ⌘S"));
  }

  function draw() {
    if (!P.draft) return;
    drawList();
    drawEditor();
    drawSavebar();
    D.keys?.drawStrip();
  }

  async function save() {
    if (!dirty()) return;
    // Drop empty feed rows quietly; they're unfinished, not errors.
    for (const t of P.draft.news) t.sources = t.sources.filter((f) => f.name.trim() || f.url.trim());
    for (const t of P.draft.research) {
      if (t.category_include && !Object.keys(t.category_include).length) delete t.category_include;
    }
    try {
      await invoke("save_topics", { topics: P.draft });
      S.topics = clone(P.draft);
      P.saved = norm(P.draft);
      P.error = "";
    } catch (e) {
      P.error = String(e);
    }
    draw();
  }

  function discard() {
    P.draft = JSON.parse(P.saved);
    if (P.sel && !current()) P.sel = null;
    P.error = "";
    draw();
  }

  D.picker = {
    open() {
      P.saved = norm(S.topics);
      P.draft = clone(S.topics);
      P.sel = P.sel && current() ? P.sel : null;
      P.menu = false;
      P.error = "";
    },
    draw,
    save,
    dirty,
    /// Leaving with unsaved changes is refused once, with a note; a second try discards.
    canLeave() {
      if (!dirty()) return true;
      if (P.error === "unsaved") { discard(); return true; }
      P.error = "unsaved";
      $("savebar").replaceChildren(
        el("span", { class: "msg err", text: "You have unsaved changes: Save, or switch tabs again to discard them." }),
        el("button", { class: "btn", onclick: discard }, "Discard"),
        el("button", { class: "btn primary", onclick: save }, "Save  ⌘S"));
      return false;
    },
  };
})();
