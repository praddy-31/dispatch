//! `dispatch-cli`: the core without a window, for checks and manual runs.
//!
//!   dispatch-cli refresh [--if-due]     fetch every enabled source now (or only when due)
//!   dispatch-cli status                 last refresh, next slot, each source's state
//!   dispatch-cli list research|news [TOPIC] [--limit N]
//!   dispatch-cli theme                  the colours the app would use
//!   dispatch-cli backfill               recent papers for categories that have none yet
//!   dispatch-cli cite KEY [ris]         a paper's citation (BibTeX unless `ris`)
//!   dispatch-cli search WORDS…          the search archive (same query language as the app)
//!   dispatch-cli tray                   what the menu-bar menu would show
//!   dispatch-cli stats [4w|3m|1y]       the Stats tab as text (default 4 weeks)
//!
//! Uses `$DISPATCH_HOME` when set; try it with a fake one first:
//!   DISPATCH_HOME=/tmp/dispatch-test dispatch-cli refresh

use std::process::ExitCode;

use chrono::{Local, Utc};
use dispatch::{DB_FILE, Section, Store, TOPICS_FILE, Topics, fetch, paths, refresh, theme};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dispatch-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let dir = paths::data_dir();
    dispatch::netlog::init(&dir);
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    if cmd == "theme" {
        let t = theme::Watch::new(&paths::palette_dir()).current().clone();
        println!("{} ({})", t.source, t.mode);
        for (k, v) in &t.colours {
            println!("  {k:<20} {v}");
        }
        return Ok(());
    }
    if !matches!(
        cmd,
        "refresh"
            | "status"
            | "list"
            | "backfill"
            | "cite"
            | "search"
            | "tray"
            | "stats"
            | "bib"
            | "notes"
            | "suggest"
            | "related"
            | "health"
            | "network"
            | "add"
            | "status-file"
    ) {
        println!(
            "usage: dispatch-cli refresh [--if-due] | status | list research|news [TOPIC] [--limit N] | theme | backfill | cite KEY [ris] | search WORDS… | tray | stats [4w|3m|1y] | bib FILE [TAG] | notes FOLDER | suggest [DAYS] | related KEY | health | network | add LINK|ID|DOI | status-file"
        );
        return Ok(());
    }
    let mut store = Store::open(&dir.join(DB_FILE))?;
    let topics = Topics::load_or_seed(&dir.join(TOPICS_FILE))?;
    let (settings, _) = dispatch::settings::Settings::load(&dir.join(dispatch::settings::FILE));
    match cmd {
        "backfill" => {
            let cats = refresh::needs_backfill(&store, &topics);
            println!("backfilling {} categories: {}", cats.len(), cats.join(", "));
            let r = refresh::backfill(
                &mut store,
                &topics,
                &cats,
                &fetch::Http::new()?,
                Utc::now(),
                &|| std::thread::sleep(refresh::API_PAUSE),
            );
            println!("{} papers, {} new", r.fetched, r.new);
            for (c, e) in r.failed {
                println!("  FAIL {c}: {e}");
            }
        }
        "search" => {
            let q = args[1..].join(" ");
            println!(
                "FTS5 query: {}",
                dispatch::search::fts_query(&q).unwrap_or_default()
            );
            let r = store.search(&q, &Default::default(), 20)?;
            println!("{} matches", r.total);
            for h in r.hits {
                let snippet: String = h
                    .snippet
                    .replace(dispatch::search::MARK_START, "[")
                    .replace(dispatch::search::MARK_END, "]");
                println!("{}  {}\n    {}", h.row.item.key, h.row.item.title, snippet);
            }
        }
        "stats" => {
            let range = match args.get(1) {
                Some(r) => dispatch::stats::Range::parse(r).ok_or("stats [4w|3m|1y]")?,
                None => Default::default(),
            };
            let s = dispatch::stats::Stats::build(
                &store,
                &topics,
                &settings,
                dispatch::stats::local_now(),
                range,
            )?;
            print!("{}", s.summary());
        }
        "health" => {
            let h = dispatch::health::check(&store, &topics, &settings, &dir, Utc::now())?;
            println!("{h:#?}");
        }
        "add" => {
            // What a Shortcut's dispatch://add link does.
            let input = args[1..].join(" ");
            let a = dispatch::add::add(
                &mut store,
                &topics,
                &input,
                &fetch::Http::new()?,
                Utc::now(),
            )?;
            let how = if a.new {
                "added and saved"
            } else {
                "saved (already here)"
            };
            println!("{how}: {}  {}", a.key, a.title);
        }
        "status-file" => {
            // What status.json would say now.
            let c = dispatch::companion::Companion::build(&store, &topics, &settings, Utc::now())?;
            let s = dispatch::status::Status::build(&c, &store, &settings, Utc::now())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?
            );
        }
        "network" => {
            for h in dispatch::netlog::recent(&dir, Utc::now()) {
                println!(
                    "{:>5} {:<32} {} (failed {})",
                    h.count, h.host, h.why, h.failed
                );
            }
        }
        "tray" => {
            let c = dispatch::companion::Companion::build(&store, &topics, &settings, Utc::now())?;
            println!("{c:#?}");
        }
        "suggest" => {
            // Suggested over the past DAYS (default 7).
            let days: i64 = args.get(1).and_then(|d| d.parse().ok()).unwrap_or(7);
            let now = Utc::now().timestamp();
            let rows =
                dispatch::rank::suggested(&store, &topics.follow, now - days * 86400, now + 1)?;
            let p = dispatch::rank::Profile::build(&store)?;
            println!(
                "profile: {} papers + {} opened; {} words: {}",
                p.papers,
                p.opened,
                p.terms.len(),
                p.terms.join(" ")
            );
            for r in rows {
                println!(
                    "{:6.2}  {}  {}",
                    r.score.unwrap_or(0.0),
                    r.item.key,
                    r.item.title
                );
                if !r.why.is_empty() {
                    println!("        {}", r.why.join(" · "));
                }
            }
        }
        "related" => {
            let key = args.get(1).ok_or("related KEY")?;
            for r in dispatch::rank::related(&store, key, dispatch::rank::RELATED)? {
                println!("{}  {}", r.item.key, r.item.title);
            }
        }
        "bib" => {
            // The live .bib file, once.
            let path = args.get(1).ok_or("bib FILE [TAG]")?;
            let rows = store.list_saved(args.get(2).map(String::as_str), 100_000)?;
            let items: Vec<_> = rows.into_iter().map(|r| r.item).collect();
            let text = dispatch::bibfile::render(&items, settings.cite_keys);
            let changed = dispatch::bibfile::write(std::path::Path::new(path), &text)?;
            println!(
                "{} papers, {}",
                items.len(),
                if changed { "written" } else { "unchanged" }
            );
        }
        "notes" => {
            // One Markdown note per saved paper, once.
            let folder = args.get(1).ok_or("notes FOLDER")?;
            let rows = store.list_saved(None, 100_000)?;
            let r = dispatch::vault::sync(&dir, std::path::Path::new(folder), &rows)?;
            println!(
                "{} written, {} unchanged, {} yours (left alone)",
                r.written, r.unchanged, r.yours
            );
        }
        "cite" => {
            let key = args.get(1).ok_or("cite KEY [ris]")?;
            let item = store.get(key)?.ok_or("no such item")?;
            let format = if args.get(2).is_some_and(|a| a == "ris") {
                dispatch::cite::Format::Ris
            } else {
                dispatch::cite::Format::Bibtex
            };
            let http = fetch::Http::new()?;
            let c = dispatch::cite::citation(&store, &item, format, Some(&http), Utc::now());
            println!("% from {}\n{}", c.origin, c.text);
        }
        "refresh" => {
            let now = Utc::now();
            let scope = if args.iter().any(|a| a == "--if-due") {
                match refresh::due_scope(&store, &settings, now) {
                    Some(s) => s,
                    None => {
                        println!(
                            "not due (last good refresh {})",
                            local(store.time_meta(refresh::LAST_OK))
                        );
                        return Ok(());
                    }
                }
            } else {
                refresh::Scope::All
            };
            println!("refreshing: {scope:?}");
            let report = refresh::run(&mut store, &topics, &fetch::Http::new()?, now, scope);
            for s in &report.sources {
                match &s.error {
                    None => println!(
                        "  ok    {:<18} {:>4} items, {:>4} new",
                        s.name, s.fetched, s.new
                    ),
                    Some(e) => println!("  FAIL  {:<18} {e}", s.name),
                }
            }
            println!(
                "{} new items in {:.1} s",
                report.new_items(),
                (Utc::now() - now).num_milliseconds() as f64 / 1000.0
            );
        }
        "status" => {
            let now = Utc::now();
            println!("data folder:  {}", dir.display());
            for section in [Section::Research, Section::News] {
                println!(
                    "{:<13} last ok {}, next {}",
                    format!("{}:", section.as_str()),
                    local(refresh::last_ok(&store, section)),
                    local(Some(refresh::next_due(&store, &settings, section, now)))
                );
            }
            println!(
                "items:        {} research, {} news",
                store.count(Section::Research)?,
                store.count(Section::News)?
            );
            for s in store.statuses()? {
                let state = match &s.last_error {
                    None => "ok".to_owned(),
                    Some(e) => format!("FAIL: {e}"),
                };
                println!(
                    "  {:<8} {:<18} {:<30} last ok {}",
                    s.section,
                    s.name,
                    state,
                    local(s.last_ok)
                );
            }
        }
        _ => {
            let section = args
                .get(1)
                .and_then(|s| Section::parse(s))
                .ok_or("list research|news")?;
            let topic = args
                .get(2)
                .filter(|a| !a.starts_with("--"))
                .map(String::as_str);
            let limit = args
                .iter()
                .position(|a| a == "--limit")
                .and_then(|i| args.get(i + 1))
                .and_then(|n| n.parse().ok())
                .unwrap_or(20);
            for r in store.list(section, topic, limit)? {
                let who = if section == Section::Research {
                    r.item.authors.first().cloned().unwrap_or_default()
                } else {
                    r.item.source_name.clone()
                };
                println!("[{}] {} — {}", r.topics.join(","), r.item.title, who);
            }
        }
    }
    Ok(())
}

fn local(t: Option<chrono::DateTime<Utc>>) -> String {
    t.map(|t| t.with_timezone(&Local).format("%a %d %b %H:%M").to_string())
        .unwrap_or_else(|| "never".into())
}
