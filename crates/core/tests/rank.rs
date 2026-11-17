//! Ranking: "For you" order from watched keywords, followed authors and words
//! shared with the papers you saved, noted or tagged, through the real FTS5 index.

use chrono::{DateTime, Utc};
use dispatch::follow::Follow;
use dispatch::rank::{self, Profile};
use dispatch::store::Store;
use dispatch::topics::{Category, ResearchTopic, Topics};
use dispatch::{Item, Section};

fn at() -> DateTime<Utc> {
    "2026-10-30T05:30:00Z".parse().unwrap()
}

fn paper(id: &str, title: &str, summary: &str, authors: &[&str]) -> Item {
    Item {
        key: format!("arxiv:{id}"),
        section: Section::Research,
        source: "arxiv".into(),
        source_name: "arXiv".into(),
        title: title.into(),
        authors: authors.iter().map(|a| (*a).into()).collect(),
        summary: summary.into(),
        link: format!("https://arxiv.org/abs/{id}"),
        pdf: None,
        categories: vec!["quant-ph".into()],
        published: Some(at()),
        announce_type: Some("new".into()),
        version: Some(1),
        doi: None,
        journal_ref: None,
    }
}

/// Five papers of yours about Floquet driving of spin chains, and today's list.
fn store() -> (Store, Vec<String>) {
    let mut mine: Vec<Item> = (0..5)
        .map(|i| {
            paper(
                &format!("2609.0000{i}"),
                &format!("Floquet driving of spin chains, part {i}"),
                "Periodic Floquet driving of a spin chain gives prethermal phases.",
                &["Old Author"],
            )
        })
        .collect();
    let today = vec![
        paper(
            "2610.00001",
            "Dark matter halos in dwarf galaxies",
            "Galaxies and halos.",
            &["X"],
        ),
        paper(
            "2610.00002",
            "Prethermal Floquet phases of spin chains",
            "A spin chain under driving.",
            &["Y"],
        ),
        paper(
            "2610.00003",
            "Graphene plasmonics",
            "Plasmons in graphene.",
            &["Ana Lima"],
        ),
        paper(
            "2610.00004",
            "Spin waves in magnets",
            "A spin wave.",
            &["Z"],
        ),
        paper(
            "2610.00005",
            "Topological qudits",
            "Qudits with a topological code.",
            &["W"],
        ),
    ];
    mine.extend(today.clone());
    let mut s = Store::in_memory().unwrap();
    s.upsert(&mine, at()).unwrap();
    let topics = Topics {
        version: 1,
        research: vec![ResearchTopic {
            id: "quantum".into(),
            name: "Quantum".into(),
            enabled: true,
            categories: vec![Category {
                id: "quant-ph".into(),
                enabled: true,
            }],
            include: vec![],
            exclude: vec![],
            category_include: Default::default(),
        }],
        news: vec![],
        follow: Default::default(),
        pinned: vec![],
        mute: vec![],
    };
    s.retag(&topics).unwrap();
    let saved: Vec<String> = mine[0..4].iter().map(|i| i.key.clone()).collect();
    s.set_saved(&saved, true, at()).unwrap();
    s.set_note(&mine[4].key, "notes count too", at()).unwrap();
    (s, today.iter().map(|i| i.key.clone()).collect())
}

#[test]
fn the_profile_is_the_words_your_papers_share() {
    let (s, _) = store();
    let p = Profile::build(&s).unwrap();
    assert_eq!(p.papers, 5);
    for w in ["floquet", "spin", "chain", "driving", "prethermal"] {
        assert!(p.terms.iter().any(|t| t == w), "{w} in {:?}", p.terms);
    }
    assert!(
        !p.terms.iter().any(|t| t == "gives" || t == "part"),
        "{:?}",
        p.terms
    );
    assert!(
        !p.terms.iter().any(|t| t == "chains"),
        "plurals folded: {:?}",
        p.terms
    );
    assert!(
        p.fts_query()
            .unwrap()
            .starts_with("{ftitle fabstract} : (\"")
    );
    assert_eq!(
        Profile::build(&Store::in_memory().unwrap())
            .unwrap()
            .fts_query(),
        None
    );
}

#[test]
fn keywords_and_people_come_first_then_papers_like_yours() {
    let (s, today) = store();
    let mut rows = s.list(Section::Research, None, 50).unwrap();
    rows.retain(|r| today.contains(&r.item.key));
    rows.sort_by(|a, b| a.item.key.cmp(&b.item.key));
    let follow = Follow {
        authors: vec!["A. Lima".into()],
        keywords: vec!["qudit".into()],
    };
    let profile = Profile::build(&s).unwrap();
    rank::score_rows(&s, &follow, &profile, &mut rows).unwrap();
    rank::sort(&mut rows);
    let order: Vec<&str> = rows.iter().map(|r| &r.item.key[6..]).collect();
    // Keyword and followed author tie at the top (kept in listed order); then the Floquet paper,
    // then the one sharing only "spin"; the galaxy paper last.
    assert_eq!(
        order,
        vec![
            "2610.00003",
            "2610.00005",
            "2610.00002",
            "2610.00004",
            "2610.00001"
        ]
    );
    assert_eq!(rows[0].why, vec!["you follow A. Lima"]);
    assert_eq!(rows[1].why, vec!["keyword “qudit”"]);
    assert_eq!(
        rows[2].score,
        Some(rank::SIMILAR),
        "the closest paper gets the full share"
    );
    assert!(
        rows[2].why[0].starts_with("like your papers: "),
        "{:?}",
        rows[2].why
    );
    assert_eq!(rows[2].why[0], "like your papers: chain, driving, floquet");
    assert_eq!(rows[4].score, Some(0.0));
    assert!(rows[4].why.is_empty());

    // Nothing saved and nothing followed: every score is 0, so the order stays newest first.
    let empty = Store::in_memory().unwrap();
    let mut rows = s.list(Section::Research, None, 50).unwrap();
    let before: Vec<String> = rows.iter().map(|r| r.item.key.clone()).collect();
    rank::score_rows(&empty, &Follow::default(), &Profile::default(), &mut rows).unwrap();
    rank::sort(&mut rows);
    let after: Vec<String> = rows.iter().map(|r| r.item.key.clone()).collect();
    assert_eq!(before, after);
}

#[test]
fn suggested_is_the_best_unopened_across_topics_without_your_own() {
    let (s, _) = store();
    let follow = Follow {
        authors: vec![],
        keywords: vec!["qudit".into()],
    };
    let from = at().timestamp();
    let got = rank::suggested(&s, &follow, from, from + 1).unwrap();
    let keys: Vec<&str> = got.iter().map(|r| &r.item.key[6..]).collect();
    assert_eq!(&keys[..2], &["2610.00005", "2610.00002"], "{keys:?}");
    assert!(
        !keys.contains(&"2610.00001"),
        "the galaxy paper is too far: {keys:?}"
    );
    assert!(
        !keys.iter().any(|k| k.starts_with("2609")),
        "your own papers left out"
    );
    // Seen (read) papers still come in; opened ones and other days don't.
    s.set_read("arxiv:2610.00002", true, at()).unwrap();
    let got = rank::suggested(&s, &follow, from, from + 1).unwrap();
    assert!(got.iter().any(|r| r.item.key == "arxiv:2610.00002"));
    s.set_opened("arxiv:2610.00005", at()).unwrap();
    let got = rank::suggested(&s, &follow, from, from + 1).unwrap();
    assert_ne!(got[0].item.key, "arxiv:2610.00005");
    assert!(
        rank::suggested(&s, &follow, from + 1, from + 99)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn related_papers_share_the_subject() {
    let (s, _) = store();
    let got = rank::related(&s, "arxiv:2610.00002", 3).unwrap();
    assert_eq!(got.len(), 3);
    assert!(
        got.iter().all(|r| r.item.key != "arxiv:2610.00002"),
        "not itself"
    );
    assert!(
        got.iter().all(|r| r.item.title.contains("Floquet")),
        "{:?}",
        got.iter().map(|r| &r.item.title).collect::<Vec<_>>()
    );
    assert!(rank::related(&s, "arxiv:nope", 3).unwrap().is_empty());
}

#[test]
fn opened_papers_count_half() {
    let mut s = Store::in_memory().unwrap();
    s.upsert(
        &[paper(
            "2610.00009",
            "Rydberg tweezer arrays",
            "Rydberg atoms in tweezers.",
            &["Q"],
        )],
        at(),
    )
    .unwrap();
    assert!(Profile::build(&s).unwrap().terms.is_empty());
    s.set_opened("arxiv:2610.00009", at()).unwrap();
    let p = Profile::build(&s).unwrap();
    assert_eq!((p.papers, p.opened), (0, 1));
    assert!(p.terms.iter().any(|t| t == "rydberg"), "{:?}", p.terms);
}
