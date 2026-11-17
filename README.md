# Dispatch

**A calm daily research and news feed for your Mac's menu bar.** New arXiv papers for the
topics you choose, with their maths typeset, plus the RSS feeds you add yourself, in one
keyboard-first window. Dispatch refreshes itself once a day, works offline from its own
archive, and turns any paper into BibTeX, RIS or a Zotero entry in one key.

<!-- Hero GIF goes here (docs/media/hero.gif) once it is recorded. -->

## Why

Keeping up with a field means opening arXiv's listing pages every morning, scrolling past
LaTeX source where the formulas should be, copying citations by hand, and losing the paper
you meant to come back to. News lives in yet another app. Dispatch puts both in one quiet
list that is ready when arXiv publishes, shows titles and abstracts the way the authors
wrote them, remembers what you've read, and keeps everything on your Mac, so a train ride
without Wi-Fi is still a reading session.

## What it does

- **Research:** the day's arXiv papers for the topics you pick, from 20 subject groups down
  to single categories, narrowed by keywords if you like. Formulas are typeset with KaTeX,
  cross-listed papers show once, and quiet days show a week digest instead of an empty list.
- **News:** any RSS or Atom feed, grouped into your own topics. A Today view, the same story
  from several feeds shown once ("3 sources"), a Discussion link when a feed provides one,
  and muting a feed or a word for a day, a week, a month or for good.
- **Citations:** BibTeX or RIS for one paper or a selection: copy it, save a file, send it
  to the Zotero app (or your Zotero web library), or keep a live `.bib` file in step with
  your saved papers. Taken from doi.org for published papers and from arXiv otherwise.
- **Keyboard first:** `j`/`k` to move, `s` to save, `b` for BibTeX, `c` for `\cite{key}`,
  `/` to search, ⌘K for every command, `?` for the full list, and a short Guide for each
  feature. A strip at the bottom shows the keys that fit what you're doing.
- **Your archive, searchable offline:** full-text search over everything Dispatch has ever
  fetched, plus your notes and tags, with phrases, `-word`, `OR` and fields like `au:`,
  `ti:` and `cat:`.
- **A menu-bar companion:** unread counts, five papers worth a look (open, PDF, BibTeX, save
  without opening the window), source status lights, Refresh now, and ⌃⌥S to search from any app.
- **Reading tools:** save for later, notes, tags, a "read later today" queue, triage (keep,
  later, skip, one at a time), an abstract view in large text, and Stats on what you actually
  read, with an optional weekly goal.
- **Follow people and ideas:** follow authors by name and watch keywords; their new papers
  are marked, sorted first in "For you", and can send one summary notification per refresh.
- **Discovery** (optional): Suggested papers you haven't opened, Related papers from your own
  archive, pinned searches, and similar / citing / cited papers from Semantic Scholar when you
  ask for them.
- **Private by design:** no account, no analytics. Dispatch contacts only your feeds, arXiv,
  doi.org, and the services you switch on; Settings → Data → Network log lists every host it
  contacted this week, and why.

Everything optional is off at first, and so are all notifications.

## Install

Apple silicon Mac, macOS 13 (Ventura) or later.

```sh
brew trust --cask praddy-31/tap/dispatch     # Homebrew 7 asks you to trust casks from other taps once
brew install --cask praddy-31/tap/dispatch
```

Or download `Dispatch-<version>-arm64.dmg` from the
[latest release](https://github.com/praddy-31/dispatch/releases/latest) and drag Dispatch to
Applications.

Dispatch is ad-hoc signed, not notarised by Apple (that needs a paid developer account), so
macOS blocks a downloaded copy the first time. Allow it in **System Settings → Privacy &
Security → Open Anyway**, or remove the download flag yourself:

```sh
xattr -dr com.apple.quarantine /Applications/Dispatch.app
```

To update, quit Dispatch from its menu-bar icon, replace the app (or `brew upgrade`), and
open it again. Your data stays where it is.

## First run

1. Dispatch starts in the menu bar. Click the icon → **Open Dispatch**.
2. **Topics** (⌘T) → Research: start a topic from one of 20 groups (Quantum, Optics &
   photonics, Condensed matter, AI & machine learning, Astrophysics, Mathematics…), switch single arXiv categories on or off, and add keywords to narrow
   or exclude. A small starter set is there to show how it works; change it freely. Dispatch
   fetches the recent papers of each new category straight away.
3. **Topics** → News: add a topic and paste the address of any RSS or Atom feed. **Test**
   checks the feed before you save. Dispatch ships with no news feeds: you choose the sources.
4. Read with `j`/`k`, open with `o`, save with `s`. `?` shows every key, and `?` then `g`
   opens the Guide.

arXiv publishes once a day (Sunday to Thursday nights, US Eastern time), and Dispatch
fetches it at 05:30 UTC, just after. News is fetched every 3 hours by default (Settings →
Refresh: hourly to daily). A Mac that was asleep catches up when it wakes.

## A tour

| Key | Does |
|---|---|
| `j` / `k` (↓ / ↑) | next / previous item |
| `o` (↵) · `p` · `d` | open the page · open the PDF · download the PDF as `2026 Smith - Title.pdf` and open it in your PDF app |
| `u` · `s` · `l` · `n` | read / unread · save · read later today · write a note |
| `b` · `c` | copy BibTeX · copy `\cite{key}` with the same key as your `.bib` |
| `e` · `a` | triage the unread ones · abstract view, one paper at a time in large text |
| `g` · `h` | next topic with unread papers · hide arXiv's replacements (new versions) |
| `[` / `]` | previous / next topic |
| `1` `2` `3` `4` | Research, News, Saved, Search (also ⌘F) |
| `/` | search just this section and topic |
| `w` · `t` | Stats · your tags |
| ⌘N · ⌘T · ⌘, | add a paper by arXiv link, id or DOI · Topics · Settings |
| `r` · `i` · ⌘K · `?` | refresh · source status · command palette · all shortcuts |

**Search** understands `"exact phrase"`, `-word`, `a OR b`, and the fields `au:`, `ti:`,
`abs:`, `note:`, `tag:` and `cat:`; an arXiv id finds its paper. It covers items whose topic
is switched off too, so nothing you've seen is ever lost. Modes for People (followed authors,
watched keywords, any author in the archive), Tags and Notes sit beside Papers.

**Research workflow** (Settings → Research workflow): a "Dispatch documents" folder holds a
`.bib` file kept in step with your saved papers (or one tag), with `smith2024adiabatic` or
arXiv-id keys, and the PDFs you download. A notes folder (an Obsidian vault works well) gets
one Markdown note per saved paper; a note you edit there is never overwritten. ⌘K copies a
`\bibitem`, and clicking a formula copies its LaTeX.

**Links:** `dispatch://search?q=spin%20chain`, `dispatch://paper/2410.01234`,
`dispatch://open/saved`, `dispatch://refresh` and `dispatch://add?q=<arXiv link or DOI>` work
from Raycast, Shortcuts, a note or the terminal (`open "dispatch://refresh"`). Settings →
Links lists them with Copy buttons, and every paper has Copy link.

**Your data, looked after:** a backup a day (the newest five kept), Back up now, Export… and
Restore in Settings → Data, plus Data health (items per feed, quiet feeds, a database check)
and Export everything (JSON plus a CSV of saved papers). Optional iCloud sync keeps topics,
saved papers, notes, tags and read state in step across your Macs.

**Look:** the window can be solid or show the desktop through it, with a blur strength or one
of Apple's frosted materials (Settings → Look), and "Solid when not in front" to save GPU.

## Works with

All optional; Dispatch works fine on its own.

- **[Zotero](https://www.zotero.org):** Send to Zotero puts papers into the collection
  selected in the open Zotero app (through its local connector port), or into your web
  library with an API key kept in the macOS Keychain.
- **[Obsidian](https://obsidian.md)** or any Markdown folder: one note per saved paper, and a
  weekly reading log to copy.
- **[Semantic Scholar](https://www.semanticscholar.org):** similar papers, citing papers and
  references, fetched only when you ask, then kept for offline use. A free API key is
  optional and makes it faster.
- **[Palette](https://github.com/praddy-31/Palette):** Dispatch takes its colours from
  Palette's current theme (or uses its built-in Tokyo Night), and `palette glass dispatch`
  can set the window's glass.
- **[SketchyBar](https://github.com/FelixKratz/SketchyBar):** Settings → Menu bar → Status
  writes `status.json` for a bar item (Palette's Dispatch item reads it), e.g. papers read today.
- **[Habit Tracker](https://github.com/praddy-31/Habit-Tracker):** Settings → Reading → Habit
  Tracker link writes a daily reading log that the Habit Tracker counts as a habit.

## Privacy

No accounts, no analytics, no tracking. The only places Dispatch contacts:

- the RSS and Atom feeds you add;
- arXiv: `rss.arxiv.org` once per daily slot, and `export.arxiv.org` once for each new
  category, for an author's recent papers when you press Search arXiv on their card, and for
  a paper you add by link. At most one request every 3 seconds, and it backs off for an hour
  when arXiv asks it to;
- `arxiv.org` and `doi.org` when you copy or export a citation, add a paper by DOI, or
  download a PDF;
- only when you switch them on: the Zotero app on this Mac (`127.0.0.1:23119`),
  `api.zotero.org`, and `api.semanticscholar.org` (a paper's arXiv id or DOI, on request).

Settings → Data → Network log shows every host contacted in the last 7 days, how often and
why (host names only, kept on your Mac). API keys live in the macOS Keychain, never in a file.

Your data is in `~/Library/Application Support/Dispatch/`: `dispatch.sqlite` (items, read
state, notes, citation cache), and `topics.json` and `settings.json`, both plain text you can
read, edit or back up. Delete the folder to start over.

## Build from source

Needs Rust (1.88 or later, edition 2024), the Xcode command line tools and the Tauri CLI
(`cargo install tauri-cli --version '^2'`). No Node: the window is plain HTML, CSS and
JavaScript with no build step.

```sh
cd app && cargo tauri build              # Dispatch.app and the .dmg in target/release/bundle/
cd app && cargo tauri dev                # run it (uses your real data folder)
```

Checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                   # about 130 tests on saved feed files, no network
```

There is also a command-line tool for trying things without the window, pointed at a
throwaway data folder:

```sh
DISPATCH_HOME=/tmp/dispatch-test cargo run -p dispatch-core --bin dispatch-cli -- refresh
DISPATCH_HOME=/tmp/dispatch-test cargo run -p dispatch-core --bin dispatch-cli -- list research
DISPATCH_HOME=/tmp/dispatch-test cargo run -p dispatch-core --bin dispatch-cli -- search au:smith
```

**How it's put together:** `crates/core` holds everything that can be tested without a
window: the feed adapters (arXiv RSS, the arXiv API, any RSS/Atom feed), the SQLite store
with its FTS5 search index, topics, the refresh schedule, citations, ranking, stats and
sync. `app/` is a Tauri 2 shell around it: the menu-bar icon and menu, the window, the
scheduler and the commands the page calls. KaTeX is vendored, so nothing in the window is
loaded from the network. Details in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

| Folder | What's in it |
|---|---|
| `crates/core/src` | fetching, parsing, storage, search, topics, citations, ranking, stats, sync |
| `crates/core/tests` | end-to-end tests on made-up feed files in `fixtures/` |
| `app/src` | the macOS app: menu bar, window, scheduler, commands, Keychain, backups |
| `app/ui` | the window (plain JS, HTML, CSS) and the vendored KaTeX |
| `docs` | architecture notes |

Bug reports and ideas are welcome as [issues](https://github.com/praddy-31/dispatch/issues).
See [CONTRIBUTING.md](CONTRIBUTING.md) before a pull request.

## Credits

Dispatch stands on a lot of good open work. Thank you to everyone behind these.

- **[arXiv](https://arxiv.org):** thank you to arXiv for use of its open access
  interoperability. Dispatch is not affiliated with or endorsed by arXiv.
- [Tauri](https://tauri.app): the app shell, menu-bar icon and window, using the web view
  macOS already has, plus its plugins for single instance, the clipboard, dialogs,
  notifications, opening links and the global shortcut.
- [KaTeX](https://katex.org): typesets the maths in titles and abstracts (vendored, MIT).
- [feed-rs](https://github.com/feed-rs/feed-rs): reads RSS and Atom feeds.
- [SQLite](https://sqlite.org) through [rusqlite](https://github.com/rusqlite/rusqlite):
  the store, and FTS5 for full-text search.
- [reqwest](https://github.com/seanmonstar/reqwest), [serde](https://serde.rs),
  [chrono](https://github.com/chronotope/chrono) and
  [security-framework](https://github.com/kornelski/rust-security-framework) (the Keychain).
- [doi.org](https://www.doi.org) and Crossref's content negotiation for citations;
  [Semantic Scholar](https://www.semanticscholar.org) and [Zotero](https://www.zotero.org)
  for their open APIs.

News feeds belong to their publishers. Dispatch shows the headline and summary a feed
provides and links back to the publisher's site; it doesn't copy or host articles.

## Licence

[MIT](LICENSE)
