//! HTML → plain text for posting descriptions and HN comments.
//!
//! This only flattens markup. Stripping hidden text and other injection
//! defenses belong to `gantry-guard` (§6.3, M3), before any model sees it.

use scraper::{ElementRef, Html, Node, Selector};

const BLOCK: &[&str] = &[
    "p",
    "br",
    "div",
    "li",
    "ul",
    "ol",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "tr",
    "table",
    "section",
    "article",
    "blockquote",
    "pre",
];
const SKIP: &[&str] = &["script", "style", "noscript", "template", "head"];

/// Text content of an HTML fragment, one line per block element, with
/// runs of whitespace collapsed.
pub fn to_text(html: &str) -> String {
    let doc = Html::parse_fragment(html);
    let mut raw = String::new();
    walk(doc.root_element(), &mut raw);
    raw.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn walk(el: ElementRef<'_>, out: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(e) => {
                let name = e.name();
                if SKIP.contains(&name) {
                    continue;
                }
                let block = BLOCK.contains(&name);
                if block {
                    out.push('\n');
                }
                if let Some(child_el) = ElementRef::wrap(child) {
                    walk(child_el, out);
                }
                if block {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
}

/// Decodes HTML entities in text that is itself escaped HTML (Greenhouse's
/// `content` field), returning the HTML.
pub fn unescape(escaped: &str) -> String {
    Html::parse_fragment(escaped)
        .root_element()
        .text()
        .collect()
}

/// `href` values of every link, entity-decoded.
pub fn links(html: &str) -> Vec<String> {
    let doc = Html::parse_fragment(html);
    // A constant selector; parsing cannot fail.
    let Ok(sel) = Selector::parse("a[href]") else {
        return Vec::new();
    };
    doc.select(&sel)
        .filter_map(|a| a.value().attr("href"))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_become_lines_and_scripts_vanish() {
        let t = to_text(
            "<h2>About</h2><p>We build <b>tools</b>.</p><ul><li>Rust</li><li>SQL</li></ul>\
             <script>alert(1)</script><p>Pay: $45&ndash;$55 per hour</p>",
        );
        assert_eq!(
            t,
            "About\nWe build tools.\nRust\nSQL\nPay: $45–$55 per hour"
        );
    }

    #[test]
    fn greenhouse_content_is_double_decoded() {
        let escaped = "&lt;p&gt;Fish &amp;amp; chips&lt;/p&gt;";
        assert_eq!(unescape(escaped), "<p>Fish &amp; chips</p>");
        assert_eq!(to_text(&unescape(escaped)), "Fish & chips");
    }

    #[test]
    fn hn_links_are_decoded() {
        let html =
            r#"Acme | <a href="https:&#x2F;&#x2F;jobs.lever.co&#x2F;acme" rel="nofollow">x</a>"#;
        assert_eq!(links(html), ["https://jobs.lever.co/acme"]);
    }
}
