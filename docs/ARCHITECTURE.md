# Architecture

How Dispatch is put together, for anyone reading or changing the code. Keep it current when
modules change.

## Layers

```
app/ui/  (plain JS, HTML, CSS, vendored KaTeX; no build step)
   │  app.js reader · topics.js picker · settings.js · keys.js shortcuts · guide.js ·
   │  authors.js · triage.js · stats.js
   │  invoke(...) → Tauri commands; events back: refreshing, refreshed, topics, settings,
   │  synced, theme, show-view, open-item
app/src/  (Tauri 2 shell)
   ├── main.rs      commands, window + Dock policy, scheduler thread (60 s), theme (2 s), sync (20 s)
   ├── tray.rs      menu-bar menu built from `Companion`, rebuilt on change; icon states
   ├── hotkey.rs    the global shortcut (⌃⌥S by default)
   ├── glass.rs     window look: opaque, a frosted material, or a blur radius
   ├── data.rs      backups, Send to Zotero, Keychain, `dispatch://` links, Semantic Scholar keys
   ├── workflow.rs  live `.bib` and notes export thread, PDF hand-off, deadlines
   ├── stats.rs     Stats commands and `reading-log.json`
   ├── status.rs    `status.json` for a status bar
   ├── health.rs    data health, compact, export everything, network log
   └── login.rs     launch at login (a LaunchAgent that only starts the app)
crates/core/  (lib `dispatch`: everything testable without a window)
   ├── sources/     adapters: arxiv.rs (RSS), arxiv_api.rs (API, Atom), rss.rs (any RSS/Atom)
   ├── refresh.rs   one run: parallel fetch, store, status, retag; per-section clocks; backfill
   ├── schedule.rs  arXiv's daily slot every N days, news every N hours, offline back-off
   ├── store.rs     SQLite: items, topics per item, source status, fetch log, citations,
   │                user data (saved, notes, tags), meta, the FTS5 search index
   ├── search.rs    typed query → FTS5 MATCH (every word quoted), BM25 weights, snippets
   ├── topics.rs    topics.json; groups → categories, keywords, matching
   ├── catalogue.rs arXiv's 20 subject groups for the picker (arxiv_catalogue.json)
   ├── news.rs      mutes, discussion links, the same story from several feeds
   ├── cite.rs      BibTeX/RIS: cache → doi.org → arXiv → written locally
   ├── bibfile.rs   the live `.bib`, citation keys, `\cite`, `\bibitem`
   ├── vault.rs     one Markdown note per saved paper, the weekly reading log
   ├── documents.rs the "Dispatch documents" folder (.bib + PDFs)
   ├── pdf.rs       tidy PDF names, download once, installed PDF apps
   ├── follow.rs    followed authors and keywords, name matching, author card, arXiv author search
   ├── alerts.rs    after a refresh: what deserves a notification, one summary
   ├── rank.rs      "For you", Suggested and Related
   ├── companion.rs what the menu-bar menu shows
   ├── stats.rs     Stats numbers (read = opened, seen = previewed), reading log
   ├── accents.rs   LaTeX accents in author names → Unicode
   ├── hints.rs     "code"/"data" links and reading hints in abstracts
   ├── add.rs       Add to Dispatch: an arXiv link, id or DOI
   ├── s2.rs        Semantic Scholar: similar, citing and cited papers, on request, cached
   ├── zotero.rs    Send to Zotero: the local app or the web API
   ├── sync.rs      iCloud sync file, field-by-field merge, conflict copies
   ├── backup.rs    daily backups (newest 5), restore
   ├── health.rs    data health check, plain export
   ├── netlog.rs    network log: hosts per day, last 7 days
   ├── status.rs    status.json for a status bar
   ├── links.rs     `dispatch://` link parser
   ├── deadlines.rs deadlines.json
   ├── settings.rs  settings.json, clamped limits
   ├── theme.rs     Palette's colour file if present, built-in Tokyo Night otherwise
   ├── fetch.rs     `Fetch` trait; `Http` = blocking reqwest with Dispatch's User-Agent
   ├── paths.rs     data folder (`DISPATCH_HOME`), atomic writes
   └── bin/cli.rs   dispatch-cli
```

The shell holds one `Store` connection for the window's reads; each refresh opens its own on a
background thread (SQLite's WAL mode lets both work at once).

## Sources (adapters)

Each adapter is a `parse(bytes) -> Vec<Item>` function; downloading is separate (`Fetch`), so
tests use saved files. An `Item` has key, section, source, title, authors, summary (plain text,
LaTeX kept), link, pdf, categories, published, announce_type, version, doi, journal_ref. The
refresh step stores them; adapters never touch the database.

| Adapter | Used for | Notes |
|---|---|---|
| arXiv RSS | daily papers | `https://rss.arxiv.org/rss/<cat>+<cat>…`, several categories in one request; empty at weekends. Authors come in one `dc:creator`, and the announce type, DOI and journal in `arxiv:` elements that feed-rs doesn't map, so `arxiv.rs` reads them itself |
| arXiv API | the recent papers of a new category (backfill), an author's recent papers when asked, a paper added by link | Atom; one request every 3 s, 90 s time limit, pauses an hour after HTTP 429 |
| Any RSS/Atom | news | through feed-rs. RSS `<comments>` (read from the XML, since feed-rs drops it) or Atom `<link rel="replies">` becomes the story's Discussion link |
| Semantic Scholar | similar papers, citing papers, references | only on request per paper; 3 s apart without a key, about 1 s with one; cached in `citations` (format `s2`) |

Keys: papers `arxiv:<id without version>` (so cross-listings and new versions are one item),
papers added by DOI `doi:<doi>`, news `news:<link>`.

## Topics

- A **research topic** maps to arXiv categories (`physics.optics`); a bare archive (`cs`)
  matches all its categories. Each category can be switched off on its own.
- `category_include` narrows one category inside a topic: the starter "Language models" topic
  takes `cs.LG` papers only when they mention language models.
- **Keywords** per topic: `include` (at least one must match when present) and `exclude` (none
  may match), checked case-insensitively in the title and abstract.
- A **news topic** is a name and a list of feeds. Dispatch ships with none: the user adds them.
- Membership is stored per item and recomputed after every refresh (`Store::retag`). Items
  matching no enabled topic are hidden, not deleted, and stay searchable.
- Defaults: `crates/core/src/default_topics.json`, a small research starter set, copied once
  into the data folder.

## Data on disk

`~/Library/Application Support/Dispatch/` (or `$DISPATCH_HOME`):

| File | What |
|---|---|
| `dispatch.sqlite` (+ `-wal`, `-shm`) | items (with first seen, read, opened and "read later" times), topics per item, source status, a fetch log (newest 2000), cached citations, user data (saved, note, tags, each with a change time for sync), meta (refresh clocks, backfill state, stats reset), and the FTS5 `search` table kept current by triggers. Schema version in SQLite's `user_version` (now 7), migrated forward on open |
| `topics.json` | the user's topics, follows, pinned searches and mutes; seeded once, never replaced if broken |
| `settings.json` | intervals, look, workflow folders, menu bar, notifications (defaults if missing or broken) |
| `backups/<date_time>/` | daily backups: a `VACUUM INTO` copy of the database plus the two JSON files; newest 5 kept |
| `sync-conflicts/` | iCloud conflict copies after they were merged (kept) |
| iCloud Drive `Dispatch/dispatch-sync.json` | only with sync on: topics, saved items, notes, tags, read state (60 days) |
| `network-log.json` | hosts contacted per day (count, failures), last 7 days; host names only |
| `reading-log.json` | only with the Habit Tracker link on: per-day counts (read, seen, saved, noted), last 400 days |
| `status.json` | only with "Status for SketchyBar" on: today's counts and a label for a bar item |
| `vault.json` | fingerprints of the Markdown notes Dispatch wrote, so edited ones are left alone |
| `deadlines.json` | optional, hand-edited deadlines shown in the menu and Stats |
| `zotero/` | `.bib` files handed to the Zotero app (newest 10) |

Secrets (Zotero and Semantic Scholar API keys) are in the macOS Keychain, service "Dispatch".

## Refresh flow

1. Trigger: the scheduler thread (every 60 s, first check 5 s after launch), Refresh now
   (window, `r`, menu bar), or saving topics that add a source (that section only).
   Only one refresh runs at a time.
2. Automatic runs cover what `refresh::due_scope` says: arXiv when its last good refresh is
   before the latest 05:30 UTC slot (arXiv rebuilds its feeds at midnight US Eastern), news
   when it is older than the chosen interval. A section waits 10 minutes after an attempt in
   which all its sources failed. A section with nothing to fetch moves its clock on anyway.
3. One thread per source: download (25 s timeout) and parse.
4. Per source: on success, upsert its items in one transaction and mark it ok (0 items is ok).
   On failure, keep its old items and record the reason. One failing feed never stops the others.
5. Retag every item by topic; set the per-section clocks; emit `refreshed` (the window reloads
   lists and counts; the menu shows the last refresh). Alerts are collected from items whose
   `first_seen` is this refresh, so backfill and sync never notify.

## Backfill flow

At launch and after saving topics that add a category: `needs_backfill` lists enabled
categories not yet backfilled; one background thread fetches each one's 40 newest papers
through the arXiv API, 3 s apart, upserts and retags. The window says "Fetching recent
papers…" meanwhile.

## Sync flow

A thread every 20 s, when sync is on: runs if something changed here, the sync file's
modification time changed (another Mac wrote it), or 5 minutes passed. `sync::run`: export
this Mac → read the file and any conflict copies → merge field by field (the newer change
wins) → apply what's newer to the store and `topics.json` → write the merged file if it
changed → move conflict copies to `sync-conflicts/`.

## The window

- **Reader** (`app.js`): every visible state is derived from `D.state` inside `draw*`
  functions. Lists are grouped by day; an item counts as seen after 1 s in the preview. Feed
  text is set with `textContent` only; KaTeX (`trust: false`) runs on research titles and
  abstracts afterwards.
- **Picker** (`topics.js`): edits a draft; typing updates the draft without redrawing (keeps
  focus), structural edits redraw; Save writes `topics.json` through `save_topics`.
- **Settings** (`settings.js`): each control saves at once; `save_settings` clamps and writes.
- **Keys** (`keys.js`): one `ACTIONS` table (id, key, group, `when`, `run`) feeds the key
  handler, the shortcut strip, ⌘K and the `?` sheet.
- **Window and Dock:** the menu-bar icon is always there; `LSUIElement` makes Dispatch start
  as a menu-bar app, and it shows in the Dock only while its window is open. Closing the
  window keeps it running.

## Network

Every host Dispatch may contact:

| Host | When |
|---|---|
| the user's feeds | each refresh (news interval) |
| `rss.arxiv.org` | once per daily slot |
| `export.arxiv.org` | backfill of a new category, an author's recent papers (button), a paper added by link |
| `arxiv.org` | BibTeX for a paper, a PDF when asked |
| `doi.org` | citations and details for papers with a DOI |
| `127.0.0.1:23119` | Send to Zotero, with the Zotero app open |
| `api.zotero.org` | Zotero online, only when set up and the app isn't open |
| `api.semanticscholar.org` | only with Semantic Scholar on, when a preview asks |

No telemetry. Every request through `fetch::Http` or the Zotero clients is counted by host in
`network-log.json`, shown in Settings → Data → Network log.

Files written outside the data folder only go where the user chose in Settings: the Dispatch
documents folder (the live `.bib`, never replacing a `.bib` Dispatch didn't write, and
downloaded PDFs; moving the folder moves only Dispatch's own files) and a notes folder (one
Markdown note per saved paper, never replacing a file it didn't write or one the user edited).

## Glossary

- **Cross-listing:** a paper filed in one arXiv category and also listed in others.
- **announce_type:** arXiv RSS field: `new`, `cross`, `replace`, `replace-cross`.
- **Backfill:** fetching a category's recent papers once, when it is added, so a new topic
  isn't empty until the next daily slot.
- **Content negotiation:** asking `doi.org` for a DOI with an `Accept` header
  (`application/x-bibtex`, `application/x-research-info-systems`, or CSL JSON) to get a
  citation or a paper's details back.
- **FTS5:** SQLite's full-text search extension; **BM25:** its relevance score.
- **WAL:** SQLite's write-ahead log, which lets a reader and a writer work at once.
