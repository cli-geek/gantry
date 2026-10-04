//! Location text → structure. Splitting and work-mode detection happen
//! here; turning the remaining place text into coordinates needs the
//! gazetteer and happens in discovery.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::WorkMode;
use crate::geo::Place;

/// An area a location covers when it is not a single point, e.g. the "US"
/// in "Remote - US".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Area {
    Anywhere,
    Continent { code: String },
    Country { country: String },
    Admin1 { country: String, admin1: String },
}

/// One listed location of a posting, after parsing and geocoding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Location {
    pub raw: String,
    /// Work modes stated in the location text. Empty when the text states
    /// none; the posting's structured `work_mode` then applies.
    pub modes: Vec<WorkMode>,
    pub place: Option<Place>,
    pub area: Option<Area>,
    /// Place text that could not be resolved; the posting is flagged.
    pub unresolved: Option<String>,
}

impl Location {
    /// The area this location lies in, from the explicit area or the place.
    pub fn effective_area(&self) -> Option<Area> {
        if let Some(area) = &self.area {
            return Some(area.clone());
        }
        self.place.as_ref().map(|p| match &p.admin1 {
            Some(a) => Area::Admin1 {
                country: p.country.clone(),
                admin1: a.clone(),
            },
            None => Area::Country {
                country: p.country.clone(),
            },
        })
    }
}

/// A location string split into one entry, before geocoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLocation {
    pub raw: String,
    pub modes: Vec<WorkMode>,
    /// "Anywhere", "Worldwide" and similar.
    pub anywhere: bool,
    /// Comma-separated place components with mode words removed, e.g.
    /// `["Seattle", "WA"]`. May hold several places run together
    /// ("San Francisco, CA, Seattle WA"); the geocoder splits those.
    pub parts: Vec<String>,
}

const MODE_WORDS: &[(&str, Option<WorkMode>)] = &[
    ("work from home", Some(WorkMode::Remote)),
    ("remote first", Some(WorkMode::Remote)),
    ("fully remote", Some(WorkMode::Remote)),
    ("remotely", Some(WorkMode::Remote)),
    ("remote", Some(WorkMode::Remote)),
    ("wfh", Some(WorkMode::Remote)),
    ("hybrid", Some(WorkMode::Hybrid)),
    ("on site", Some(WorkMode::Onsite)),
    ("onsite", Some(WorkMode::Onsite)),
    ("in office", Some(WorkMode::Onsite)),
    ("in person", Some(WorkMode::Onsite)),
    ("office based", Some(WorkMode::Onsite)),
    ("home based", Some(WorkMode::Remote)),
    ("distributed", Some(WorkMode::Remote)),
    // Scope words: no mode of their own.
    ("anywhere", None),
    ("worldwide", None),
    ("world wide", None),
    ("global", None),
];

const ANYWHERE_WORDS: &[&str] = &["anywhere", "worldwide", "world wide", "global"];

/// Trailing words naming a site rather than a place: "San Francisco Office".
const SITE_SUFFIXES: &[&str] = &[
    " office",
    " offices",
    " hq",
    " headquarters",
    " metropolitan area",
    " metro area",
    " metro",
];

/// Parts that carry no place at all.
const NOISE_PARTS: &[&str] = &["n/a", "na", "tbd", "hq"];

/// Splits a raw location field into entries and strips work-mode words.
///
/// Handles the forms seen on Greenhouse, Lever, Ashby and list feeds:
/// "City, ST", "Remote - US", "US - Remote", "Remote (US)", "Remote-USA",
/// "Hybrid - City", "City (Hybrid)", and lists joined with `;` or `|`.
pub fn parse(raw: &str) -> Vec<ParsedLocation> {
    raw.split([';', '|', '\n'])
        .flat_map(|s| s.split(" / "))
        .flat_map(|s| s.split(" or "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_one)
        .collect()
}

fn parse_one(segment: &str) -> ParsedLocation {
    let mut modes = Vec::new();
    let mut anywhere = false;
    let mut text = segment.to_owned();
    for (word, mode) in MODE_WORDS {
        while let Some((start, end)) = find_word(&text, word) {
            if let Some(m) = mode
                && !modes.contains(m)
            {
                modes.push(*m);
            }
            if ANYWHERE_WORDS.contains(word) {
                anywhere = true;
            }
            text.replace_range(start..end, ",");
        }
    }
    let parts = text
        .split([',', '(', ')', '–', '—', '•', '·'])
        .flat_map(|s| s.split(" - "))
        .map(|s| s.trim_matches(|c: char| c.is_whitespace() || "-/:.".contains(c)))
        .map(strip_leading_in)
        .map(strip_site_suffix)
        .filter(|s| !s.is_empty() && !NOISE_PARTS.contains(&s.to_ascii_lowercase().as_str()))
        .map(str::to_owned)
        .collect();
    ParsedLocation {
        raw: segment.to_owned(),
        modes,
        anywhere,
        parts,
    }
}

/// "in USA" (from "Remote in USA") → "USA"; "based in Berlin" → "Berlin".
fn strip_leading_in(part: &str) -> &str {
    for prefix in ["based in ", "in "] {
        if let (Some(head), Some(rest)) = (part.get(..prefix.len()), part.get(prefix.len()..))
            && head.eq_ignore_ascii_case(prefix)
            && !rest.is_empty()
        {
            return rest.trim_start();
        }
    }
    part
}

fn strip_site_suffix(part: &str) -> &str {
    for suffix in SITE_SUFFIXES {
        let Some(cut) = part.len().checked_sub(suffix.len()) else {
            continue;
        };
        if let (Some(head), Some(tail)) = (part.get(..cut), part.get(cut..))
            && tail.eq_ignore_ascii_case(suffix)
            && !head.trim().is_empty()
        {
            return head.trim_end();
        }
    }
    part
}

/// Finds `word` (lowercase, may contain spaces matching any run of
/// whitespace or hyphens) in `text`, case-insensitively, on word boundaries.
/// Matching runs on an ASCII-lowercased copy, which keeps byte offsets
/// valid for cutting the original.
fn find_word(text: &str, word: &str) -> Option<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let pieces: Vec<&str> = word.split(' ').collect();
    let mut from = 0;
    while let Some(rel) = lower[from..].find(pieces[0]) {
        let start = from + rel;
        let mut end = start + pieces[0].len();
        let mut ok = true;
        for piece in &pieces[1..] {
            let sep_start = end;
            while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'-') {
                end += 1;
            }
            if end == sep_start || !lower[end..].starts_with(piece) {
                ok = false;
                break;
            }
            end += piece.len();
        }
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if ok && before_ok && after_ok {
            return Some((start, end));
        }
        from = start + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(raw: &str) -> ParsedLocation {
        let mut v = parse(raw);
        assert_eq!(v.len(), 1, "{raw}: {v:?}");
        v.remove(0)
    }

    #[test]
    fn plain_city_state() {
        let p = one("San Francisco, CA");
        assert!(p.modes.is_empty());
        assert_eq!(p.parts, ["San Francisco", "CA"]);
    }

    #[test]
    fn remote_forms_seen_on_live_boards() {
        for raw in [
            "Remote - US",
            "US - Remote",
            "Remote (US)",
            "Remote, US",
            "Remote-US",
            "Remote US",
            "Remote in US",
        ] {
            let p = one(raw);
            assert_eq!(p.modes, [WorkMode::Remote], "{raw}");
            assert_eq!(p.parts, ["US"], "{raw}");
        }
    }

    #[test]
    fn hybrid_with_city() {
        for raw in ["Hybrid - Seattle, WA", "Seattle, WA (Hybrid)"] {
            let p = one(raw);
            assert_eq!(p.modes, [WorkMode::Hybrid], "{raw}");
            assert_eq!(p.parts, ["Seattle", "WA"], "{raw}");
        }
    }

    #[test]
    fn multiple_modes_and_anywhere() {
        let v = parse("Remote or Hybrid - NYC");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].modes, [WorkMode::Remote]);
        assert!(v[0].parts.is_empty());
        assert_eq!(v[1].modes, [WorkMode::Hybrid]);
        assert_eq!(v[1].parts, ["NYC"]);

        let p = one("Remote/Hybrid - Denver, CO");
        assert_eq!(p.modes, [WorkMode::Remote, WorkMode::Hybrid]);
        assert_eq!(p.parts, ["Denver", "CO"]);

        let p = one("REMOTE (Worldwide)");
        assert!(p.anywhere);
        assert!(p.parts.is_empty());
    }

    #[test]
    fn list_separators_split_entries() {
        let v = parse("Dublin, Ireland; London, United Kingdom");
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].parts, ["London", "United Kingdom"]);
        assert_eq!(parse("Austin, TX | Denver, CO").len(), 2);
    }

    #[test]
    fn hyphenated_place_names_survive() {
        assert_eq!(one("Winston-Salem, NC").parts, ["Winston-Salem", "NC"]);
        assert_eq!(
            one("Remote - Bangalore, India").parts,
            ["Bangalore", "India"]
        );
    }

    #[test]
    fn site_words_and_separators_seen_on_live_boards() {
        assert_eq!(one("San Francisco Office").parts, ["San Francisco"]);
        assert_eq!(one("SF HQ").parts, ["SF"]);
        assert_eq!(
            one("New York City Metropolitan Area").parts,
            ["New York City"]
        );
        assert_eq!(one("NY • United States").parts, ["NY", "United States"]);
        assert!(one("N/A").parts.is_empty());
        assert!(one("HQ").parts.is_empty());
        let home = one("Home based - EMEA");
        assert_eq!(home.modes, [WorkMode::Remote]);
        assert_eq!(home.parts, ["EMEA"]);
        assert!(one("World Wide").anywhere);
        assert_eq!(one("Office Based").modes, [WorkMode::Onsite]);
    }

    #[test]
    fn mode_word_inside_another_word_is_ignored() {
        assert!(one("Remoteville, TX").modes.is_empty());
    }
}
