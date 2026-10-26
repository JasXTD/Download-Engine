use super::*;
use scraper::{Html, Node, Selector};

const POLICY: &str = "default-src 'none'; img-src 'self' file: data:; style-src 'self' file: 'unsafe-inline'; font-src 'self' file: data:; media-src 'none'; script-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
fn active_element(name: &str) -> bool {
    matches!(
        name,
        "script" | "iframe" | "object" | "embed" | "noscript" | "template"
            // SVG animation can restore an online href after URL rewriting.
            | "animate" | "animateMotion" | "animateTransform" | "set" | "discard"
    )
}
pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn rewrite(
    text: &str,
    url: &Url,
    mut resolve: impl FnMut(&Url, &str, Kind) -> String,
) -> Result<(String, bool)> {
    let mut document = Html::parse_document(text);
    ensure!(
        document.tree.nodes().take(100_001).count() <= 100_000,
        "Page contains too many HTML nodes"
    );
    let base = document
        .select(&Selector::parse("base[href]").unwrap())
        .next()
        .and_then(|e| url.join(e.value().attr("href")?).ok())
        .unwrap_or_else(|| url.clone());
    let mut interactive = false;
    // Collect IDs before editing to avoid retaining node borrows across mutation.
    let mut ids = Vec::new();
    for node in document.tree.root().descendants() {
        // Ancestor checks below and the serializer must not process arbitrarily
        // deep markup, even when a page fits the byte and node-count limits.
        ensure!(
            node.ancestors().take(257).count() <= 256,
            "HTML nesting exceeds the supported limit"
        );
        ids.push(node.id());
    }
    for id in ids {
        let node = document.tree.get(id).unwrap();
        if node
            .ancestors()
            .any(|n| matches!(n.value(), Node::Element(e) if active_element(e.name())))
        {
            continue;
        }
        let Some(element) = node.value().as_element() else {
            continue;
        };
        let tag = element.name().to_string();
        if active_element(&tag) {
            interactive = true;
            document.tree.get_mut(id).unwrap().detach();
            continue;
        }
        if tag == "base"
            || (tag == "meta"
                && (element.attr("http-equiv").is_some() || element.attr("charset").is_some()))
        {
            document.tree.get_mut(id).unwrap().detach();
            continue;
        }
        if tag == "link"
            && !element
                .attr("rel")
                .unwrap_or("")
                .split_ascii_whitespace()
                .any(|r| r.eq_ignore_ascii_case("stylesheet") || r.eq_ignore_ascii_case("icon"))
        {
            document.tree.get_mut(id).unwrap().detach();
            continue;
        }
        let stylesheet = tag == "link"
            && element
                .attr("rel")
                .unwrap_or("")
                .split_ascii_whitespace()
                .any(|r| r.eq_ignore_ascii_case("stylesheet"));
        if tag == "style" {
            let children: Vec<_> = node.children().map(|n| n.id()).collect();
            for child in children {
                if let Node::Text(value) = document.tree.get_mut(child).unwrap().value() {
                    value.text =
                        css::rewrite(&value.text, |raw, kind| resolve(&base, raw, kind))?.into();
                }
            }
        }
        let mut node = document.tree.get_mut(id).unwrap();
        let Node::Element(element) = node.value() else {
            unreachable!()
        };
        if tag == "form" {
            element.name.local = "div".into();
            interactive = true;
        }
        element.attrs.retain(|(name, _)| {
            let name = name.local.as_ref();
            let remove = name.starts_with("on")
                || matches!(
                    name,
                    "integrity"
                        | "crossorigin"
                        | "nonce"
                        | "ping"
                        | "action"
                        | "formaction"
                        | "srcdoc"
                        | "manifest"
                        | "autoplay"
                );
            if name.starts_with("on") || matches!(name, "action" | "formaction" | "srcdoc") {
                interactive = true;
            }
            !remove
        });
        for (name, value) in &mut element.attrs {
            let attribute = name.local.as_ref();
            let rewritten = match attribute {
                "href" if matches!(tag.as_str(), "a" | "area") => {
                    Some(resolve(&base, value, Kind::Page))
                }
                "href" if tag == "link" => Some(resolve(
                    &base,
                    value,
                    if stylesheet { Kind::Css } else { Kind::Asset },
                )),
                "href" if matches!(tag.as_str(), "image" | "use") => {
                    Some(if value.starts_with('#') {
                        value.to_string()
                    } else {
                        resolve(&base, value, Kind::Asset)
                    })
                }
                "src" if matches!(tag.as_str(), "img" | "input") => {
                    Some(resolve(&base, value, Kind::Asset))
                }
                "src" => {
                    interactive = true;
                    Some("data:,".into())
                }
                "poster" | "background" => Some(resolve(&base, value, Kind::Asset)),
                "srcset" | "imagesrcset" => {
                    Some(srcset(value, |raw| resolve(&base, raw, Kind::Asset)))
                }
                "style" => Some(css::rewrite(value, |raw, kind| resolve(&base, raw, kind))?),
                // SVG presentation attributes can contain paint-server URLs.
                "fill" | "stroke" | "filter" | "clip-path" | "mask" | "cursor" | "marker-start"
                | "marker-mid" | "marker-end" => Some(css::rewrite(value, |raw, kind| {
                    if raw.starts_with('#') {
                        raw.into()
                    } else {
                        resolve(&base, raw, kind)
                    }
                })?),
                _ => None,
            };
            if let Some(rewritten) = rewritten {
                *value = rewritten.into();
            }
        }
    }
    // First in <head>: prevents original markup from fetching or executing online
    // dependencies when opened with file://. The HTML serializer handles escaping.
    let head = document
        .select(&Selector::parse("head").unwrap())
        .next()
        .context("HTML has no head")?
        .id();
    let fragment = Html::parse_fragment(&format!(
        "<meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{}\"><meta name=\"referrer\" content=\"no-referrer\">",
        escape(POLICY)
    ));
    let metas: Vec<_> = fragment
        .select(&Selector::parse("meta").unwrap())
        .map(|e| Node::Element(e.value().clone()))
        .collect();
    for meta in metas.into_iter().rev() {
        document.tree.get_mut(head).unwrap().prepend(meta);
    }
    Ok((document.html(), interactive))
}

fn srcset(value: &str, mut resolve: impl FnMut(&str) -> String) -> String {
    let mut remaining = value;
    let mut parts = Vec::new();
    while !remaining.is_empty() {
        remaining = remaining.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if remaining.is_empty() {
            break;
        }
        let end = remaining
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(remaining.len());
        let url = &remaining[..end];
        remaining = &remaining[end..];
        if url.ends_with(',') {
            parts.push(resolve(url.trim_end_matches(',')));
            continue;
        }
        let mut nesting = 0usize;
        let end = remaining
            .char_indices()
            .find_map(|(i, c)| {
                if c == '(' {
                    nesting += 1;
                } else if c == ')' {
                    nesting = nesting.saturating_sub(1);
                }
                (c == ',' && nesting == 0).then_some(i)
            })
            .unwrap_or(remaining.len());
        let descriptor = remaining[..end].trim();
        parts.push(format!(
            "{}{}{}",
            resolve(url),
            if descriptor.is_empty() { "" } else { " " },
            descriptor
        ));
        remaining = &remaining[end..];
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_svg_cannot_restore_online_links_through_animation() {
        let url = Url::parse("https://example.com/page").unwrap();
        let input = r##"<svg><a href="/next"><set attributeName="href" to="https://outside.example/"/><text>Next</text></a><animate href="#image" attributeName="href" values="https://outside.example/a;https://outside.example/b"/><animateMotion/><animateTransform/><discard/><path marker-start="url(marker.svg#m)" marker-end="url(#local)"/></svg>"##;
        let (output, interactive) = rewrite(input, &url, |_, raw, _| {
            if raw.starts_with('#') {
                raw.into()
            } else {
                "saved.svg#m".into()
            }
        })
        .unwrap();
        assert!(interactive);
        assert!(
            !output.contains("outside.example")
                && !output.contains("<set")
                && !output.contains("<animate")
                && !output.contains("<discard"),
            "{output}"
        );
        assert!(output.contains("saved.svg#m") && output.contains("#local"));
        assert!(!output.contains("marker.svg"));
    }

    #[test]
    fn deeply_nested_pages_are_rejected_before_ancestor_checks_and_serialization() {
        let url = Url::parse("https://example.com/page").unwrap();
        let text = format!("{}content{}", "<div>".repeat(300), "</div>".repeat(300));
        let error = rewrite(&text, &url, |_, _, _| "saved".into()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "HTML nesting exceeds the supported limit"
        );
    }

    #[test]
    fn oversized_documents_are_rejected_before_rewriting() {
        let url = Url::parse("https://example.com/page").unwrap();
        let text = "<i></i>".repeat(100_001);

        let error = rewrite(&text, &url, |_, _, _| "saved".into()).unwrap_err();

        assert_eq!(error.to_string(), "Page contains too many HTML nodes");
    }

    #[test]
    fn base_srcset_styles_and_active_content_are_handled_by_html_parser() {
        let url = Url::parse("https://example.com/folder/page").unwrap();
        let text = r#"<!doctype html><base href="/assets/"><script src="bad.js"></script><meta http-equiv="refresh" content="0;url=https://bad.example"><link rel=stylesheet href="style.css" integrity="old"><style>.x{background:url(a.png)}</style><a href="../next?q=1&amp;x=2#part" onclick="fetch('bad')">Next</a><img srcset="data:image/png;base64,abc 1x, big.png 2x"><form action=/send><button>Send</button></form>"#;
        let mut found = Vec::new();
        let (out, active) = rewrite(text, &url, |base, raw, kind| {
            found.push((base.join(raw).unwrap().to_string(), kind));
            "saved".into()
        })
        .unwrap();
        assert!(active);
        assert!(
            !out.contains("bad.js")
                && !out.contains("onclick")
                && !out.contains("<base")
                && !out.contains("refresh")
                && !out.contains("integrity=")
        );
        assert!(out.contains("script-src 'none'") && out.contains("<div><button>Send"));
        assert!(found.contains(&("https://example.com/next?q=1&x=2#part".into(), Kind::Page)));
        assert!(found.contains(&("https://example.com/assets/big.png".into(), Kind::Asset)));
        assert!(found.contains(&("data:image/png;base64,abc".into(), Kind::Asset)));
    }
}
