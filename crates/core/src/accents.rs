//! LaTeX accents in author names → Unicode: arXiv sends names as their authors
//! typed them, e.g. `Schr\"oer`, `Micha{\l} Studzi\'nski`, `Va\v{n}o`. Names aren't run
//! through KaTeX (only titles and abstracts are), so they're decoded once, here: when a feed
//! is parsed and when a stored row is read (so papers fetched before this change look right).

/// (accent, base letter, composed letter), one row per accent.
const COMPOSED: &[(char, &str, &str)] = &[
    (
        '\'',
        "aeiouyAEIOUYcCnNsSzZlLrRgG",
        "áéíóúýÁÉÍÓÚÝćĆńŃśŚźŹĺĹŕŔǵǴ",
    ),
    ('`', "aeiouAEIOU", "àèìòùÀÈÌÒÙ"),
    ('^', "aeiouAEIOUcghjswyCGHJSWY", "âêîôûÂÊÎÔÛĉĝĥĵŝŵŷĈĜĤĴŜŴŶ"),
    ('"', "aeiouyAEIOUY", "äëïöüÿÄËÏÖÜŸ"),
    ('~', "anoiuANOIU", "ãñõĩũÃÑÕĨŨ"),
    ('=', "aeiouAEIOU", "āēīōūĀĒĪŌŪ"),
    ('.', "zegcZEGCI", "żėġċŻĖĠĊİ"),
    ('v', "cszernldtgCSZERNLDTG", "čšžěřňľďťǧČŠŽĚŘŇĽĎŤǦ"),
    ('u', "gauiGAUI", "ğăŭĭĞĂŬĬ"),
    ('c', "cstCST", "çşţÇŞŢ"),
    ('k', "aeiuAEIU", "ąęįųĄĘĮŲ"),
    ('H', "ouOU", "őűŐŰ"),
    ('r', "auAU", "åůÅŮ"),
];

/// Commands that are a letter of their own (`{\o}`, `\ss`).
const LETTERS: &[(&str, &str)] = &[
    ("o", "ø"),
    ("O", "Ø"),
    ("l", "ł"),
    ("L", "Ł"),
    ("i", "ı"),
    ("j", "ȷ"),
    ("ss", "ß"),
    ("aa", "å"),
    ("AA", "Å"),
    ("ae", "æ"),
    ("AE", "Æ"),
    ("oe", "œ"),
    ("OE", "Œ"),
    ("TH", "Þ"),
    ("th", "þ"),
    ("DH", "Ð"),
    ("dh", "ð"),
    ("NG", "Ŋ"),
    ("ng", "ŋ"),
];

/// Accents written with a letter (`\v{s}`); they need a brace or a space after them.
const LETTER_ACCENTS: &str = "vuckHrdbt";

/// `Micha{\l} Studzi\'nski` → `Michał Studziński`. Text without `\` or `{` is returned as it is.
pub fn decode(s: &str) -> String {
    if !s.contains('\\') && !s.contains('{') {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' || c == '}' {
            i += 1; // grouping only
            continue;
        }
        if c != '\\' || i + 1 == chars.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let next = chars[i + 1];
        if "'`^\"~=.".contains(next) {
            let (base, used) = argument(&chars[i + 2..]);
            out.push_str(&compose(next, &base));
            i += 2 + used;
            continue;
        }
        if next.is_ascii_alphabetic() {
            let name: String = chars[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let after = i + 1 + name.chars().count();
            let accent = name.len() == 1
                && LETTER_ACCENTS.contains(next)
                && matches!(chars.get(after), Some('{') | Some(' '));
            if accent {
                let (base, used) = argument(&chars[after..]);
                out.push_str(&compose(next, &base));
                i = after + used;
                continue;
            }
            if let Some((_, letter)) = LETTERS.iter().find(|(n, _)| *n == name) {
                out.push_str(letter);
                // `\ss x`: the space only ends the command
                i = after
                    + usize::from(
                        chars.get(after) == Some(&' ') && after + 1 < chars.len() && name.len() > 1,
                    );
                continue;
            }
            out.push('\\');
            out.push_str(&name);
            i = after;
            continue;
        }
        out.push(next); // `\&`, `\-` and the like
        i += 2;
    }
    out
}

/// The accent's letter: `{e}`, `{\i}`, ` e` or `e`. Returns it and how many chars it took.
fn argument(rest: &[char]) -> (String, usize) {
    let mut i = 0;
    while rest.get(i) == Some(&' ') {
        i += 1;
    }
    if rest.get(i) == Some(&'{') {
        let close = rest[i..].iter().position(|&c| c == '}').map(|p| i + p);
        if let Some(close) = close {
            let inner: String = rest[i + 1..close].iter().collect();
            let inner = inner.trim().trim_start_matches('\\');
            return (inner.to_owned(), close + 1);
        }
    }
    if rest.get(i) == Some(&'\\') && matches!(rest.get(i + 1), Some('i') | Some('j')) {
        return (rest[i + 1].to_string(), i + 2);
    }
    match rest.get(i) {
        Some(c) => (c.to_string(), i + 1),
        None => (String::new(), i),
    }
}

fn compose(accent: char, base: &str) -> String {
    let mut letters = base.chars();
    let (Some(b), None) = (letters.next(), letters.next()) else {
        return base.to_owned(); // `\'{}` or two letters: keep what's there
    };
    let found = COMPOSED
        .iter()
        .find(|(a, _, _)| *a == accent)
        .and_then(|(_, from, to)| {
            from.chars()
                .position(|x| x == b)
                .and_then(|p| to.chars().nth(p))
        });
    match found {
        Some(c) => c.to_string(),
        None => {
            // No single letter for it: the base letter with a combining mark.
            let mark = match accent {
                '\'' => '\u{301}',
                '`' => '\u{300}',
                '^' => '\u{302}',
                '"' => '\u{308}',
                '~' => '\u{303}',
                '=' => '\u{304}',
                '.' => '\u{307}',
                'v' => '\u{30C}',
                'u' => '\u{306}',
                'c' => '\u{327}',
                'k' => '\u{328}',
                'H' => '\u{30B}',
                'r' => '\u{30A}',
                'd' => '\u{323}',
                'b' => '\u{331}',
                _ => return b.to_string(),
            };
            format!("{b}{mark}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn names_from_arxiv() {
        let cases = [
            (r"Piotr \'Cwikli\'nski", "Piotr Ćwikliński"),
            (r"{\L}ukasz Fojcik", "Łukasz Fojcik"),
            (r"Micha{\l} Studzi\'nski", "Michał Studziński"),
            (r#"C. M\"ostl"#, "C. Möstl"),
            (r#"Julian Schr\"{o}er"#, "Julian Schröer"),
            (r"Viliam Va\v{n}o", "Viliam Vaňo"),
            (r"Juan B. P\'erez-S\'anchez", "Juan B. Pérez-Sánchez"),
            (r"Fran\c{c}ois", "François"),
            (r"J{\o}rgen", "Jørgen"),
            (r"Stra{\ss}e", "Straße"),
            (r"\ss e", "ße"),
            (r"Gonz\'{a}lez", "González"),
            (r"N\'{\i}colas", "Nícolas"),
            (r"{\'E}mile", "Émile"),
            (r"\v Silhan", "Šilhan"),
            (r"Do\u{g}an", "Doğan"),
            (r#"Bj\"orn {\TH}\'or J\'onsson"#, "Björn Þór Jónsson"),
            (r"Ada Lovelace", "Ada Lovelace"),
            (r"A \& B", "A & B"),
        ];
        for (from, to) in cases {
            assert_eq!(decode(from), to, "{from}");
        }
    }

    #[test]
    fn unknown_commands_are_kept() {
        assert_eq!(decode(r"\foo bar"), r"\foo bar");
        assert_eq!(decode(r"trailing \"), r"trailing \");
    }
}
