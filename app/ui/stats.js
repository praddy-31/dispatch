// Stats: how you read, from Dispatch's own store (the `stats` command). A range
// switch (4 weeks by day, 3 months or a year by week) and sections that fold; which ones are
// folded is remembered in localStorage. Charts are plain divs and inline SVG, no library.
// Names and titles come from feeds, so they are only ever set as text.
"use strict";

(function () {
  const { el, $, invoke, state: S } = window.D;
  const SVG = "http://www.w3.org/2000/svg";
  const FOLD_KEY = "dispatch.stats.folded";
  const RANGES = [["4w", "4 weeks"], ["3m", "3 months"], ["1y", "1 year"]];

  let range = "4w";
  let data = null;
  let problem = null;
  let deadlines = { upcoming: [], error: null };
  let folded = new Set(JSON.parse(localStorage.getItem(FOLD_KEY) || "[]"));

  async function load() {
    try {
      [data, deadlines] = await Promise.all([invoke("stats", { range }),
        invoke("deadlines").catch(() => ({ upcoming: [], error: null }))]);
      problem = null;
    } catch (e) {
      problem = String(e);
    }
  }

  function setRange(r) {
    if (r === range) return;
    range = r;
    load().then(draw);
  }

  function toggleFold(id) {
    if (folded.has(id)) folded.delete(id); else folded.add(id);
    localStorage.setItem(FOLD_KEY, JSON.stringify([...folded]));
    draw();
  }

  // ---- small pieces ---------------------------------------------------------

  const pct = (r) => `${Math.round(r * 100)}%`;
  const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
  const short = (iso) => new Date(`${iso}T00:00:00`).toLocaleDateString([], { day: "numeric", month: "short" });

  function section(id, title, ...body) {
    const open = !folded.has(id);
    return el("section", { class: `stats-sec${open ? "" : " folded"}` },
      el("h2", {},
        el("button", { class: "fold", "aria-expanded": String(open), onclick: () => toggleFold(id) },
          el("span", { class: "chev", text: open ? "▾" : "▸" }), title)),
      open ? el("div", { class: "stats-in" }, ...body) : null);
  }

  /// A thin bar showing a share (0–1).
  function rateBar(r) {
    const bar = el("span", { class: "rate-bar" }, el("span", {}));
    bar.firstChild.style.width = `${Math.max(0, Math.min(1, r)) * 100}%`;
    return el("span", { class: "rate" }, bar, el("span", { class: "rate-n", text: pct(r) }));
  }

  function table(head, rows) {
    return el("table", { class: "stats-table" },
      el("thead", {}, el("tr", {}, ...head.map((h, i) => el("th", { class: i ? "num" : "", text: h })))),
      el("tbody", {}, ...rows));
  }

  const td = (v, cls = "num") => el("td", { class: cls }, v);

  // ---- charts ----------------------------------------------------------------

  /// Columns of papers read (and news on top, paler), one per bucket.
  function readingChart(buckets) {
    const max = Math.max(1, ...buckets.map((b) => b.read + b.news_read));
    const cols = buckets.map((b) => {
      const col = el("div", { class: "col", title:
        `${data.bucket === "day" ? short(b.date) : `Week of ${short(b.date)}`}: ${plural(b.read, "paper")}, ${b.news_read} news read · ${b.seen} seen · ${b.saved} saved · ${plural(b.noted, "note")}` });
      const news = el("span", { class: "seg news" });
      const papers = el("span", { class: "seg papers" });
      news.style.height = `${(b.news_read / max) * 100}%`;
      papers.style.height = `${(b.read / max) * 100}%`;
      col.append(news, papers);
      return col;
    });
    return el("div", { class: "chart" },
      el("div", { class: "bars" }, ...cols),
      axis(buckets),
      el("div", { class: "legend" },
        el("span", { class: "key papers" }), "papers read ",
        el("span", { class: "key news" }), "news read"));
  }

  function axis(buckets) {
    const first = buckets[0]?.date;
    const mid = buckets[Math.floor(buckets.length / 2)]?.date;
    return el("div", { class: "axis" },
      el("span", { text: first ? short(first) : "" }),
      el("span", { text: mid ? short(mid) : "" }),
      el("span", { text: data.bucket === "day" ? "today" : "this week" }));
  }

  function svg(tag, attrs) {
    const n = document.createElementNS(SVG, tag);
    for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, String(v));
    return n;
  }

  /// Two lines: research and news waiting at the end of each bucket.
  function backlogChart(points) {
    const W = 600, H = 120;
    const max = Math.max(1, ...points.map((p) => Math.max(p.research, p.news)));
    const x = (i) => (points.length < 2 ? W / 2 : (i / (points.length - 1)) * W);
    const y = (v) => H - 4 - (v / max) * (H - 10);
    const chart = svg("svg", { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: "none", class: "lines", role: "img",
      "aria-label": "Unread items waiting over time" });
    for (const f of [0.5, 1]) chart.append(svg("line", { x1: 0, x2: W, y1: y(max * f), y2: y(max * f), class: "grid" }));
    for (const [field, cls] of [["news", "news"], ["research", "research"]]) {
      const d = points.map((p, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(p[field]).toFixed(1)}`).join(" ");
      chart.append(svg("path", { d, class: `line ${cls}` }));
    }
    const first = points[0], last = points[points.length - 1];
    return el("div", { class: "chart" },
      el("div", { class: "lines-box" }, chart, el("span", { class: "ymax", text: String(max) })),
      axis(points),
      el("div", { class: "legend" },
        el("span", { class: "key research" }), `papers ${first?.research ?? 0} → ${last?.research ?? 0} `,
        el("span", { class: "key newsline" }), `news ${first?.news ?? 0} → ${last?.news ?? 0}`));
  }

  // ---- sections ----------------------------------------------------------------

  function readingSection() {
    const r = data.reading;
    const t = r.totals;
    const streak = r.streak > 1
      ? `${r.streak} days in a row with something read${r.streak_today ? "" : " (up to yesterday)"}.`
      : r.streak === 1 ? (r.streak_today ? "You read something today." : "You read something yesterday.") : null;
    return section("reading", "Reading over time",
      el("div", { class: "totals" },
        big(t.read, "papers read"), big(t.news_read, "news read"), big(t.seen, "seen"),
        big(t.saved, "saved"), big(t.noted, "notes")),
      streak ? el("p", { class: "calm", text: streak }) : null,
      readingChart(r.buckets),
      el("p", { class: "fine" },
        "Read = you opened its page or PDF (the preview's “Don't count” takes one back). Seen = shown in the preview. Notes count on the day they were last edited.",
        data.since ? ` Counting since ${short(data.since)}.` : "",
        " ",
        el("button", { class: "linkish", onclick: () => D.settingsView.resetStats() }, "Reset stats…")));
  }

  function big(n, label) {
    return el("div", { class: "big" }, el("b", { text: String(n) }), el("span", { text: label }));
  }

  function goalSection() {
    const g = data.goal;
    if (!g.target) {
      return section("goal", "Weekly goal",
        el("p", { class: "calm" }, "No weekly goal set. ",
          el("button", { class: "linkish", onclick: () => D.showView("settings") }, "Set one in Settings → Reading"),
          " if you'd like one."));
    }
    const bar = el("div", { class: "goal-bar" }, el("span", {}));
    bar.firstChild.style.width = `${Math.min(1, g.done / g.target) * 100}%`;
    if (g.done >= g.target) bar.classList.add("done");
    return section("goal", "Weekly goal",
      el("p", { class: "goal-line", text: `${g.done} of ${g.target} papers this week` }),
      bar,
      el("p", { class: "fine", text: `Since Monday ${short(g.week_start)}.` }));
  }

  function catName(id) {
    for (const g of S.catalogue?.groups ?? []) {
      const c = g.categories.find((x) => x.id === id);
      if (c) return c.name;
    }
    return "";
  }

  function topicsSection() {
    const rows = data.topics.map((w) => el("tr", {},
      td(w.name, ""), td(w.fetched), td(w.seen), td(w.read), td(w.saved), td(rateBar(w.rate), "rate-cell")));
    const cats = data.categories.map((w) => el("tr", {},
      el("td", {}, el("span", { class: "cat-id", text: w.id }), " ", el("small", { text: w.name || catName(w.id) })),
      td(w.fetched), td(w.seen), td(w.read), td(w.saved), td(rateBar(w.rate), "rate-cell")));
    return section("topics", "Topics worth it",
      el("p", { class: "fine", text: "Papers that arrived in this range, and how many of those you saw, read (opened) or saved." }),
      rows.length ? table(["Topic", "Arrived", "Seen", "Read", "Saved", "Read rate"], rows) : el("p", { class: "calm", text: "No research topics switched on." }),
      cats.length ? el("h3", { text: "arXiv categories" }) : null,
      cats.length ? table(["Category", "Arrived", "Seen", "Read", "Saved", "Read rate"], cats) : null);
  }

  function feedsSection() {
    const rows = data.feeds.map((f) => el("tr", { class: f.noisy ? "noisy" : "" },
      el("td", {}, f.name, f.noisy ? el("span", { class: "noisy-tag", text: "noisy" }) : null),
      td(f.fetched), td(f.seen), td(f.read), td(rateBar(f.rate), "rate-cell"),
      el("td", { class: "act" }, f.noisy
        ? el("button", { class: "btn small", title: "Switch feeds off or narrow them in Topics", onclick: () => D.showView("topics") }, "Topics…")
        : null)));
    return section("feeds", "Feeds worth it",
      rows.length
        ? table(["Feed", "Arrived", "Seen", "Read", "Read rate", ""], rows)
        : el("p", { class: "calm", text: "No news arrived in this range." }),
      rows.some((r) => r.classList.contains("noisy"))
        ? el("p", { class: "fine", text: "Noisy: many items, very few seen or read." }) : null);
  }

  async function follow(name) {
    try {
      S.topics = { ...S.topics, follow: await invoke("follow_author", { name, on: true }) };
      D.toast(`Following ${name}`);
    } catch (e) {
      D.toast(`Couldn't save: ${e}`);
    }
    await load();
    draw();
  }

  function peopleSection() {
    const p = data.people;
    const people = p.authors.map((a) => el("li", {},
      el("button", { class: "linkish person", title: `${a.name}: papers, co-authors`, onclick: () => D.authors?.open(a.name) },
        a.followed ? `👤 ${a.name}` : a.name),
      el("span", { class: "n", text: String(a.count) }),
      a.followed ? null : el("button", { class: "btn small", onclick: () => follow(a.name) }, "Follow")));
    return section("people", "People you read",
      people.length
        ? el("ol", { class: "people" }, ...people)
        : el("p", { class: "calm", text: "Nobody yet: authors of papers you read or save show here." }),
      p.tags.length ? el("h3", { text: "Your most-used tags" }) : null,
      p.tags.length ? el("div", { class: "tag-row" }, ...p.tags.map(([t, n]) =>
        el("span", { class: "chip" }, `#${t}`, el("span", { class: "n", text: String(n) })))) : null);
  }

  function backlogSection() {
    const tips = data.suggestions.map((s) => el("li", {},
      el("span", { text: s.text }),
      s.action === "topics" ? el("button", { class: "btn small", onclick: () => D.showView("topics") }, "Open Topics") : null));
    return section("backlog", "Backlog over time",
      el("p", { class: "fine", text: "Unread items waiting at the end of each " + data.bucket + " (only topics switched on)." }),
      backlogChart(data.backlog),
      tips.length ? el("ul", { class: "tips" }, ...tips) : null);
  }

  function timeSection() {
    const t = data.time_to_read;
    if (!t.count) {
      return section("time", "Time to read", el("p", { class: "calm", text: "No papers read in this range yet." }));
    }
    const days = (d) => (d === 1 ? "1 day" : `${d} days`);
    const rows = t.topics.map((m) => el("tr", {}, td(m.name, ""), td(days(m.median_days)), td(m.count)));
    return section("time", "Time to read",
      el("p", { class: "calm", text:
        `Half the papers you read were read within ${days(t.median_days)} of being announced (${plural(t.count, "paper")}); ${pct(t.within_day)} within a day.` }),
      rows.length ? table(["Topic", "Median", "Papers"], rows) : null);
  }

  /// The next deadlines from deadlines.json (none = no section, unless it's broken).
  function deadlineSection() {
    const list = deadlines.upcoming || [];
    if (!list.length && !deadlines.error) return null;
    const edit = el("button", { class: "linkish", onclick: () => invoke("edit_deadlines") }, "Edit deadlines…");
    return section("deadlines", "Deadlines",
      deadlines.error ? el("p", { class: "warn-line", text: `⚠ ${deadlines.error}` }) : null,
      ...list.map((d) => el("div", { class: `deadline${d.days <= 7 ? " soon" : ""}` },
        el("span", { class: "when", text: d.when }),
        d.url ? el("button", { class: "linkish", onclick: () => invoke("open_link", { url: d.url }) }, d.name) : el("span", { text: d.name }),
        el("span", { class: "muted-note", text: ` · ${short(d.date)}` }))),
      el("p", { class: "calm" }, edit));
  }

  // ---- the tab -----------------------------------------------------------------

  function draw() {
    const pane = $("stats");
    if (problem) {
      pane.replaceChildren(el("div", { class: "settings-body" }, el("p", { class: "empty", text: `Couldn't build the stats: ${problem}` })));
      return;
    }
    if (!data) { pane.replaceChildren(); return; }
    const switcher = el("div", { class: "stats-range", role: "group", "aria-label": "Range" },
      ...RANGES.map(([id, label]) => el("button", { class: "chip", "aria-pressed": String(id === range), onclick: () => setRange(id) }, label)));
    const top = pane.scrollTop;
    pane.replaceChildren(el("div", { class: "settings-body stats-body" },
      el("div", { class: "stats-head" },
        el("span", { class: "stats-when", text: `${short(data.start)} – today` }),
        switcher),
      deadlineSection(),
      readingSection(),
      goalSection(),
      topicsSection(),
      feedsSection(),
      peopleSection(),
      backlogSection(),
      timeSection(),
      el("p", { class: "about", text: "Worked out on this Mac from Dispatch's own database. Nothing is sent anywhere." })));
    pane.scrollTop = top;
  }

  // Read state or topics changed elsewhere (menu bar, refresh, sync): redraw if showing.
  for (const ev of ["refreshed", "synced", "topics", "settings"]) {
    window.__TAURI__.event.listen(ev, async () => { if (S.view === "stats") { await load(); draw(); } });
  }

  D.statsView = {
    async open() { await load(); draw(); },
    draw,
  };
})();
