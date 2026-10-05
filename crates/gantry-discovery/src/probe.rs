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

/// Whether a board response mentions the company, for board APIs that do
/// not return its name. Postings shorten names ("Cobalt Harbor Software"
/// writes "Cobalt Harbor"), so the first two words are enough. An empty
/// board cannot be checked and does not count.
pub fn mentions_company(body: &[u8], company: &str) -> bool {
    let words = name_words(company);
    let name = words[..words.len().min(2)].join(" ");
    contains_phrase(&normalize(&String::from_utf8_lossy(body)), &name)
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
    fn mention_check_uses_the_first_two_words() {
        let body = br#"[{"descriptionPlain":"Cobalt Harbor builds billing tools."}]"#;
        assert!(mentions_company(body, "Cobalt Harbor Software, Inc."));
        assert!(!mentions_company(body, "Cobalt Robotics"));
        assert!(!mentions_company(b"[]", "Cobalt Harbor"));
    }
}
