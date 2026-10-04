//! A small robots.txt reader (RFC 9309): user-agent groups, `Allow` and
//! `Disallow` with `*` and `$`, longest match wins, `Allow` wins ties.
//! `Crawl-delay` is not in the RFC but is honored when present.

use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Robots {
    rules: Vec<Rule>,
    crawl_delay: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    allow: bool,
    pattern: String,
}

impl Robots {
    /// No restrictions: what a missing (4xx) robots.txt means.
    pub fn allow_all() -> Self {
        Self::default()
    }

    /// Everything disallowed: what an unreachable (5xx) robots.txt means.
    pub fn disallow_all() -> Self {
        Self {
            rules: vec![Rule {
                allow: false,
                pattern: "/".into(),
            }],
            crawl_delay: None,
        }
    }

    /// Parses `text`, keeping the group that best matches `agent` (the
    /// product token, e.g. "gantry"), falling back to `*`.
    pub fn parse(text: &str, agent: &str) -> Self {
        let agent = agent.to_ascii_lowercase();
        let mut groups: Vec<(Vec<String>, Robots)> = Vec::new();
        let mut in_agents = false;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            match key.as_str() {
                "user-agent" => {
                    if !in_agents {
                        groups.push((Vec::new(), Robots::default()));
                    }
                    in_agents = true;
                    if let Some((agents, _)) = groups.last_mut() {
                        agents.push(value.to_ascii_lowercase());
                    }
                }
                "allow" | "disallow" => {
                    in_agents = false;
                    if let Some((_, robots)) = groups.last_mut() {
                        // An empty Disallow means "allow everything".
                        if !value.is_empty() {
                            robots.rules.push(Rule {
                                allow: key == "allow",
                                pattern: value.to_owned(),
                            });
                        }
                    }
                }
                "crawl-delay" => {
                    in_agents = false;
                    if let (Some((_, robots)), Ok(secs)) = (groups.last_mut(), value.parse::<f64>())
                        && secs.is_finite()
                        && secs >= 0.0
                    {
                        robots.crawl_delay = Some(Duration::from_secs_f64(secs.min(60.0)));
                    }
                }
                _ => in_agents = false,
            }
        }
        let specific: Vec<&Robots> = groups
            .iter()
            .filter(|(agents, _)| {
                agents
                    .iter()
                    .any(|a| a != "*" && agent.contains(a.as_str()))
            })
            .map(|(_, r)| r)
            .collect();
        let chosen: Vec<&Robots> = if specific.is_empty() {
            groups
                .iter()
                .filter(|(agents, _)| agents.iter().any(|a| a == "*"))
                .map(|(_, r)| r)
                .collect()
        } else {
            specific
        };
        // Several groups for the same agent are merged (RFC 9309 §2.2.1).
        let mut merged = Robots::default();
        for r in chosen {
            merged.rules.extend(r.rules.iter().cloned());
            merged.crawl_delay = merged.crawl_delay.max(r.crawl_delay);
        }
        merged
    }

    /// Whether `path_and_query` (starting with `/`) may be fetched.
    pub fn allows(&self, path_and_query: &str) -> bool {
        let best = self
            .rules
            .iter()
            .filter(|r| pattern_matches(&r.pattern, path_and_query))
            .max_by_key(|r| (r.pattern.len(), r.allow));
        best.is_none_or(|r| r.allow)
    }

    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }
}

/// Matches a robots pattern against a path: `*` is any run of characters,
/// a trailing `$` anchors the end; otherwise the pattern is a prefix.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let pieces: Vec<&str> = pattern.split('*').collect();
    let Some(first) = pieces.first() else {
        return true;
    };
    if !path.starts_with(first) {
        return false;
    }
    let mut pos = first.len();
    let last_index = pieces.len() - 1;
    for (i, piece) in pieces.iter().enumerate().skip(1) {
        if i == last_index && anchored {
            return path.len() >= pos + piece.len() && path.ends_with(piece);
        }
        match path[pos..].find(piece) {
            Some(found) => pos += found + piece.len(),
            None => return false,
        }
    }
    !anchored || pos == path.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greenhouse_robots_blocks_only_embed() {
        let r = Robots::parse(
            "# See http://www.robotstxt.org\n\nUser-agent: *\nDisallow: /embed/\n",
            "gantry",
        );
        assert!(r.allows("/v1/boards/acme/jobs?content=true"));
        assert!(!r.allows("/embed/job_app?for=acme"));
    }

    #[test]
    fn hacker_news_api_allows_only_json() {
        let r = Robots::parse(
            "User-agent: *\nAllow: /*.json$\nAllow: /*.json?*$\nDisallow: /\n",
            "gantry",
        );
        assert!(r.allows("/v0/item/123.json"));
        assert!(r.allows("/v0/user/whoishiring.json"));
        assert!(!r.allows("/v0/item/123"));
        assert!(!r.allows("/"));
    }

    #[test]
    fn specific_agent_group_wins_and_crawl_delay_is_read() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /\n\nUser-agent: Gantry\nUser-agent: other\nAllow: /\nCrawl-delay: 2\n",
            "gantry/0.0.1",
        );
        assert!(r.allows("/anything"));
        assert_eq!(r.crawl_delay(), Some(Duration::from_secs(2)));
    }

    #[test]
    fn longest_match_and_allow_ties() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /jobs\nAllow: /jobs/public\nDisallow: /a\nAllow: /a\n",
            "gantry",
        );
        assert!(!r.allows("/jobs/private"));
        assert!(r.allows("/jobs/public/1"));
        assert!(r.allows("/a"));
    }

    #[test]
    fn empty_disallow_and_missing_file() {
        assert!(Robots::parse("User-agent: *\nDisallow:\n", "gantry").allows("/x"));
        assert!(Robots::allow_all().allows("/x"));
        assert!(!Robots::disallow_all().allows("/x"));
    }
}
