//! Text normalization and the stable hashes stored in the database.
//!
//! FNV-1a is hand-rolled because `std`'s `DefaultHasher` may change between
//! Rust releases, and these values are persisted.

/// Lowercases and replaces every run of non-alphanumeric characters with a
/// single space. "Sr. Software Engineer (Backend)" → "sr software engineer backend".
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.extend(c.to_lowercase());
        } else {
            pending_space = true;
        }
    }
    out
}

/// Whether `phrase` occurs in `haystack` as whole words, after normalizing
/// both. An empty phrase never matches.
pub fn contains_phrase(haystack_normalized: &str, phrase: &str) -> bool {
    let needle = normalize(phrase);
    if needle.is_empty() {
        return false;
    }
    let padded = format!(" {haystack_normalized} ");
    padded.contains(&format!(" {needle} "))
}

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// 64-bit SimHash over word 3-shingles of the normalized text. Near-identical
/// descriptions (a repost with a changed date or req number) land within a
/// few bits of each other.
pub fn simhash(text: &str) -> u64 {
    let norm = normalize(text);
    let words: Vec<&str> = norm.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return 0;
    }
    let mut weights = [0i64; 64];
    let mut add = |shingle: &[&str]| {
        let h = fnv1a64(shingle.join(" ").as_bytes());
        for (bit, w) in weights.iter_mut().enumerate() {
            if h >> bit & 1 == 1 {
                *w += 1;
            } else {
                *w -= 1;
            }
        }
    };
    if words.len() < 3 {
        add(&words);
    } else {
        words.windows(3).for_each(&mut add);
    }
    weights
        .iter()
        .enumerate()
        .filter(|(_, w)| **w > 0)
        .fold(0u64, |acc, (bit, _)| acc | 1 << bit)
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Reposts within this many differing SimHash bits count as the same job.
pub const REPOST_SIMHASH_BITS: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_punctuation() {
        assert_eq!(
            normalize("Sr. Software Engineer (Backend) – NYC"),
            "sr software engineer backend nyc"
        );
        assert_eq!(normalize("  "), "");
    }

    #[test]
    fn phrase_match_is_word_bounded() {
        let title = normalize("Senior Data Engineer");
        assert!(contains_phrase(&title, "data engineer"));
        assert!(contains_phrase(&title, "SENIOR"));
        assert!(!contains_phrase(&title, "engine"));
        assert!(!contains_phrase(&title, "  "));
    }

    #[test]
    fn fnv_matches_reference_vector() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn simhash_tolerates_small_edits_and_separates_different_text() {
        let base = "We are hiring a registered dental hygienist to provide preventive care, \
                    take radiographs, and educate patients on oral health at our Austin clinic. \
                    Requires an active state license and CPR certification.";
        let repost = base.replace("Austin clinic", "Austin clinic (reposted)");
        let other = "Build distributed storage systems in Rust; own on-call for the ingest \
                     pipeline and work with product teams on new APIs for customers.";
        assert!(hamming(simhash(base), simhash(&repost)) <= 12);
        assert!(hamming(simhash(base), simhash(other)) > REPOST_SIMHASH_BITS * 4);
        assert_eq!(simhash(base), simhash(base));
    }
}
