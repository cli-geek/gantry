//! Board-slug probing (§4.1.1): guess a company's board token from its
//! name and ask each board API whether it exists.

use gantry_core::text::{contains_phrase, normalize};

/// Trailing words that are legal form, not name.
const LEGAL_SUFFIXES: &[&str] = &[
    "inc",
    "incorporated",
    "llc",
    "ltd",
    "limited",
    "corp",
    "corporation",
    "co",
    "company",
    "gmbh",
    "plc",
    "sa",
    "ag",
    "bv",
    "pbc",
    "lp",
    "llp",
    "pty",
];

/// Host labels that name a site section rather than the company.
const GENERIC_LABELS: &[&str] = &[
    "www", "careers", "career", "jobs", "job", "apply", "work", "hire", "talent", "join", "co",
    "com", "org", "net",
];

fn name_words(name: &str) -> Vec<String> {
    let norm = normalize(name);
    let mut words: Vec<&str> = norm.split(' ').filter(|w| !w.is_empty()).collect();
    if words.first() == Some(&"the") {
        words.remove(0);
    }
    while words.len() > 1 && words.last().is_some_and(|w| LEGAL_SUFFIXES.contains(w)) {
        words.pop();
    }
    words.into_iter().map(str::to_owned).collect()
}

/// Candidate slugs, most likely first: "Pinecrest Robotics, Inc." →
/// `["pinecrestrobotics", "pinecrest-robotics"]`. Only the full name is
/// used; a first-word guess would hit unrelated companies. Names with
/// non-ASCII letters yield nothing, since slugs are ASCII.
pub fn slug_variants(name: &str) -> Vec<String> {
    let words = name_words(name);
    if words.is_empty() || words.iter().any(|w| !w.is_ascii()) {
        return Vec::new();
    }
    let mut out = vec![words.concat()];
    if words.len() > 1 {
        out.push(words.join("-"));
    }
    out.retain(|s| (2..=60).contains(&s.len()));
    out
}

/// Whether a board's display name is the company we probed for.
pub fn names_match(board_name: &str, company: &str) -> bool {
    let a = name_words(board_name).concat();
    !a.is_empty() && a == name_words(company).concat()
}

/// Whether posting descriptions mention the company, for board APIs that
/// do not return its name. Only description text counts: posting URLs
/// contain the slug, which was built from the name. Postings shorten names
/// ("Cobalt Harbor Software" writes "Cobalt Harbor"), so the first two
/// words are enough. A one-word name must appear capitalized and not just
/// as the first word of a sentence or list item ("Ramp up fast" is not
/// the company Ramp), unless it reads as a subject ("Plaid is hiring").
/// An empty board cannot be checked and does not count.
pub fn mentions_company<'a>(
    descriptions: impl IntoIterator<Item = &'a str>,
    company: &str,
) -> bool {
    let words = name_words(company);
    match words.as_slice() {
        [] => false,
        [word] => descriptions.into_iter().any(|d| names_company(d, word)),
        [first, second, ..] => {
            let name = format!("{first} {second}");
            descriptions
                .into_iter()
                .any(|d| contains_phrase(&normalize(d), &name))
        }
    }
}

/// Whether `word` (lowercase ASCII, as from [`name_words`]) appears as a
/// capitalized whole word that is not merely sentence- or item-initial.
fn names_company(text: &str, word: &str) -> bool {
    text.char_indices().any(|(i, c)| {
        let Some(found) = text.get(i..i + word.len()) else {
            return false;
        };
        let rest = &text[i + word.len()..];
        if !c.is_uppercase()
            || !found.eq_ignore_ascii_case(word)
            || text[..i].ends_with(char::is_alphanumeric)
            || rest.starts_with(char::is_alphanumeric)
        {
            return false;
        }
        let before = text[..i].trim_end_matches([' ', '\t']);
        let starts_sentence =
            before.is_empty() || before.ends_with(['.', '!', '?', '\n', ':', '•', '-', '*']);
        !starts_sentence
            || [" is ", " was ", "'s ", "’s "]
                .iter()
                .any(|v| rest.starts_with(v))
    })
}

/// A company-name guess from a careers-site host:
/// "careers.northwind.example" → "northwind".
pub fn host_company(host: &str) -> Option<String> {
    let labels: Vec<&str> = host.split('.').collect();
    let without_tld = &labels[..labels.len().saturating_sub(1)];
    without_tld
        .iter()
        .rev()
        .find(|l| !GENERIC_LABELS.contains(l) && l.len() > 2)
        .map(|l| (*l).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_strip_legal_form() {
        assert_eq!(
            slug_variants("Pinecrest Robotics, Inc."),
            ["pinecrestrobotics", "pinecrest-robotics"]
        );
        assert_eq!(slug_variants("The Linear Company"), ["linear"]);
        assert_eq!(slug_variants("Co"), ["co"]);
        assert!(slug_variants("Zürich Labs").is_empty());
        assert!(slug_variants("  ").is_empty());
    }

    #[test]
    fn board_names_compare_on_normalized_words() {
        assert!(names_match(
            "Pinecrest Robotics",
            "Pinecrest Robotics, Inc."
        ));
        assert!(!names_match("Pinecrest", "Pinecrest Robotics"));
    }

    #[test]
    fn host_names() {
        assert_eq!(
            host_company("careers.northwind.example").as_deref(),
            Some("northwind")
        );
        assert_eq!(host_company("www.acme.co.uk").as_deref(), Some("acme"));
        assert_eq!(host_company("localhost"), None);
    }

    #[test]
    fn mention_check_reads_descriptions_only() {
        let text = ["Cobalt Harbor builds billing tools. Ramp up fast."];
        assert!(mentions_company(text, "Cobalt Harbor Software, Inc."));
        assert!(!mentions_company(text, "Cobalt Robotics"));
        assert!(!mentions_company(text, "Ramp"), "the word, not the name");
        assert!(mentions_company(["Join Plaid in Utah."], "Plaid"));
        assert!(mentions_company(["Work. At Ramp, we build cards."], "Ramp"));
        assert!(!mentions_company(["Rampant growth at Acme."], "Ramp"));
        assert!(!mentions_company([], "Cobalt Harbor"));
        for text in [
            "Responsibilities\nRamp up quickly on the codebase.",
            "• Ramp up on Rust",
            "help customers - Ramp up adoption",
            "You will ramp up quickly.",
        ] {
            assert!(!mentions_company([text], "Ramp"), "{text}");
            assert!(!mentions_company([text], "ramp"), "{text}");
        }
        assert!(mentions_company(
            ["Plaid is hiring. Plaid builds APIs."],
            "Plaid"
        ));
        assert!(mentions_company(["We are hiring at Plaid."], "PLAID"));
    }
}
