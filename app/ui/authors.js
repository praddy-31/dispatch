// Following: the author card (click an author's name in a paper's preview, or a
// person in Search → People), the People list in Search, and the notification switches in
// Settings. Followed authors and watched keywords live in topics.json under `follow` (the app
// keeps them when the topic picker saves).
"use strict";

(function () {
  const { el, $, invoke, state: S } = window.D;

  const follow = () => S.topics?.follow ?? { authors: [], keywords: [] };
  const fold = (s) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/ß/g, "ss").toLowerCase();
  const surname = (name) => {
    const s = name.includes(",") ? name.split(",")[0] : name.trim().split(/\s+/).pop();
    return s.replace(/[^\p{L}\p{N}-]/gu, "");
  };

  async function setFollow(next) {
    try {
      const saved = await invoke("set_follow", { follow: next });
      S.topics = { ...S.topics, follow: saved };
      return saved;
    } catch (e) {
      D.toast(`Couldn't save: ${e}`);
      return null;
    }
  }

  async function setFollowed(name, on) {
    try {
      S.topics = { ...S.topics, follow: await invoke("follow_author", { name, on }) };
      D.toast(on ? `Following ${name}` : `Stopped following ${name}`);
    } catch (e) {
      D.toast(`Couldn't save: ${e}`);
    }
  }

  /// Whether a name as a paper spells it belongs to someone followed (for highlighting; the
  /// app does the exact matching).
  function isFollowedName(name, row) {
    const s = fold(surname(name));
    return (row?.followed ?? follow().authors).some((f) => fold(surname(f)) === s);
  }

  /// The authors line of a research preview: each name opens its card.
  function authorLine(row) {
    const line = el("div", { class: "who authors" });
    row.authors.forEach((a, i) => {
      if (i) line.append(", ");
      line.append(el("button", {
        class: `author${isFollowedName(a, row) ? " followed" : ""}`,
        title: `${a}: papers, co-authors, follow`,
        onclick: () => open(a),
      }, isFollowedName(a, row) ? `👤 ${a}` : a));
    });
    return line;
  }

  async function fetchCard(name) {
    try {
      return await invoke("author_card", { name });
    } catch (e) {
      D.toast(`Couldn't open ${name}: ${e}`);
      return null;
    }
  }

  /// The card as a pop-up (from a paper's preview).
  async function open(name) {
    const c = await fetchCard(name);
    if (!c) return;
    $("overlay").replaceChildren(el("div", { class: "sheet author-card", onclick: (e) => e.stopPropagation() },
      ...cardParts(c, () => open(c.name), () => D.keys.closeOverlay())));
    $("overlay").hidden = false;
  }

  /// The card in the preview pane (Search → People).
  async function show(name, pane) {
    const c = await fetchCard(name);
    if (!c) return;
    pane.replaceChildren(el("div", { class: "author-card in-pane" },
      ...cardParts(c, () => show(c.name, pane), () => {})));
    pane.scrollTop = 0;
  }

  /// `again` redraws the card where it is; `leave` closes it before going elsewhere.
  function cardParts(c, again, leave) {
    const followBtn = el("button", {
      class: c.followed ? "btn primary" : "btn",
      onclick: async () => { await setFollowed(c.name, !c.followed); again(); D.peopleChanged?.(); },
    }, c.followed ? "✓ Following" : "Follow");
    const arxivBtn = el("button", { class: "btn", title: "One request to arXiv's search for this name's recent papers (useful for a common name, or someone outside your categories)" },
      "Search arXiv for recent papers");
    arxivBtn.addEventListener("click", async () => {
      arxivBtn.disabled = true;
      arxivBtn.textContent = "Asking arXiv… (can take a minute)";
      try {
        const r = await invoke("author_arxiv", { name: c.name });
        D.toast(`arXiv: ${r.found} recent ${r.found === 1 ? "paper" : "papers"} by ${c.name}, ${r.new} new to Dispatch`);
        if (arxivBtn.isConnected) again();
      } catch (e) {
        D.toast(`arXiv search failed: ${e}`);
        arxivBtn.disabled = false;
        arxivBtn.textContent = "Search arXiv for recent papers";
      }
    });
    const papers = c.papers.map((r) => {
      const t = el("span", { class: "t" }, r.title);
      if (r.section === "research") D.math(t);
      return el("button", {
        onclick: () => { leave(); D.openItem(r.key, r.section); },
      }, t, el("span", { class: "sub", text: D.when(r.published || r.first_seen) }));
    });
    const co = c.coauthors.map(([n, k]) => el("button", { class: "chip", title: `${k} papers together`, onclick: () => open(n) },
      n, el("span", { class: "n", text: String(k) })));
    const n = c.papers.length;
    return [
      el("div", { class: "card-head" }, el("h2", { text: c.name }), followBtn),
      el("p", { class: "muted-note", text: `${n} ${n === 1 ? "paper" : "papers"} in Dispatch`
        + (c.followed ? " · their new papers are marked 👤 and can notify you (Settings)" : "") }),
      el("div", { class: "links" }, arxivBtn,
        el("button", { class: "btn", onclick: () => { leave(); D.setSearch({ mode: "papers" }); D.searchFor(`au:"${surname(c.name)}"`); } }, "Their papers in Search")),
      co.length ? el("h3", { text: "Often writes with" }) : null,
      co.length ? el("div", { class: "chips-wrap" }, ...co) : null,
      el("h3", { text: "Papers" }),
      n ? el("div", { class: "menu" }, ...papers)
        : el("div", { class: "none", text: "None in Dispatch yet. “Search arXiv” fetches their recent papers." }),
    ];
  }

  /// Search → People, the list side: who you follow, watched keywords, and (when something is
  /// typed) authors in the archive whose names match. `pick(name)` shows a card.
  async function peopleList(query, pick, picked) {
    const f = follow();
    const q = query.trim();
    const matches = q.length >= 2 ? await invoke("find_authors", { query: q }).catch(() => []) : [];
    const person = (name, sub, followed) => el("button", {
      class: "row person-row", "aria-current": String(picked === name), onclick: () => pick(name),
    }, el("span", { class: "title", text: followed ? `👤 ${name}` : name }), sub ? el("span", { class: "meta", text: sub }) : null);
    const add = el("input", { class: "text", placeholder: "Follow someone: type a name, ↵", spellcheck: "false" });
    add.addEventListener("keydown", async (e) => {
      e.stopPropagation();
      if (e.key === "Enter" && add.value.trim()) { await setFollowed(add.value.trim(), true); D.peopleChanged?.(); }
      if (e.key === "Escape") add.blur();
    });
    const words = el("input", { class: "text", value: f.keywords.join(", "), placeholder: "e.g. superradiance, exceptional point", spellcheck: "false" });
    words.addEventListener("keydown", (e) => { e.stopPropagation(); if (e.key === "Enter" || e.key === "Escape") words.blur(); });
    words.addEventListener("change", async () => {
      if (await setFollow({ ...follow(), keywords: words.value.split(",") })) D.toast("Watched keywords saved");
    });
    const shown = f.authors.filter((a) => !q || fold(a).includes(fold(q)));
    return [
      el("div", { class: "day", text: `Following (${f.authors.length})` }),
      ...shown.map((a) => person(a, "", true)),
      f.authors.length ? null : el("div", { class: "empty small", text: "Nobody yet. Click an author's name in any paper, or type a name below." }),
      el("div", { class: "people-tools" }, add,
        el("label", { class: "stitle", text: "Watched keywords" }), words,
        el("small", { text: "Matched by name: same surname, and the same first name or initial (H. Weller = Hanna J. Weller); for a common name write the full first name. Keywords are looked for in new papers' titles and abstracts. Notifications: Settings." })),
      q.length >= 2 ? el("div", { class: "day", text: matches.length ? `Authors matching “${q}”` : `No author in the archive matches “${q}”` }) : null,
      ...matches.map(([name, n]) => person(name, `${n} ${n === 1 ? "paper" : "papers"} in Dispatch`, isFollowedName(name))),
    ];
  }

  /// Settings → Notifications (who and what to follow is in Search → People).
  function settingsSection(row, toggle, s, save) {
    const test = el("button", { class: "btn", onclick: async () => {
      try { await invoke("test_notification"); D.toast("Sent. If nothing appeared, allow Dispatch in System Settings → Notifications."); }
      catch (e) { D.toast(`Couldn't notify: ${e}`); } } }, "Send a test");
    const goPeople = el("button", { class: "btn", onclick: () => { D.setSearch({ mode: "people" }); D.searchFor(""); } }, "Search → People");
    return [
      el("h2", { text: "Notifications", id: "following" }),
      row("Who and what you follow", "People and watched keywords are in Search → People.", goPeople),
      row("New paper by someone you follow", "One notification per refresh sums up everything below that's switched on.",
        toggle(s.notify_authors, (v) => save({ notify_authors: v }))),
      row("Watched keyword in a new paper", null, toggle(s.notify_keywords, (v) => save({ notify_keywords: v }))),
      row("A saved paper gets a new version or a DOI", null, toggle(s.notify_updates, (v) => save({ notify_updates: v }))),
      row("Test", "macOS asks once whether Dispatch may show notifications.", test),
    ];
  }

  D.authors = { open, show, peopleList, authorLine, settingsSection, isFollowedName };
})();
