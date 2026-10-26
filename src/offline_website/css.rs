//! Use the existing CSS tokenizer so comments, escapes, strings and nested rules
//! are distinguished from actual URLs. Preserve all non-URL bytes verbatim.
use super::*;
use cssparser::{Delimiter, Parser, ParserInput, Token};

pub(super) fn rewrite(text: &str, mut resolve: impl FnMut(&str, Kind) -> String) -> Result<String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let mut edits = Vec::new();
    walk(&mut parser, &mut resolve, &mut edits, 0, false)
        .map_err(|_| anyhow::anyhow!("Stylesheet nesting exceeds the supported limit"))?;
    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end, value) in edits {
        result.push_str(&text[last..start]);
        result.push_str(&value);
        last = end;
    }
    result.push_str(&text[last..]);
    Ok(result)
}
fn quoted(value: &str) -> String {
    let mut value_out = String::new();
    cssparser::serialize_string(value, &mut value_out).expect("String writes cannot fail");
    value_out
}
fn walk<'i>(
    parser: &mut Parser<'i, '_>,
    resolve: &mut impl FnMut(&str, Kind) -> String,
    edits: &mut Vec<(usize, usize, String)>,
    depth: usize,
    image_set: bool,
) -> std::result::Result<(), cssparser::ParseError<'i, ()>> {
    if depth > 64 {
        return Err(parser.new_custom_error(()));
    }
    let mut import = false;
    loop {
        let start = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        match token {
            Token::AtKeyword(name) if name.eq_ignore_ascii_case("namespace") => {
                // Namespace URIs are literal identifiers, not resources. Changing
                // one breaks namespace-qualified selectors (CSS Namespaces 3.1).
                parser.parse_until_after(
                    Delimiter::Semicolon | Delimiter::CurlyBracketBlock,
                    |rule| {
                        while rule.next_including_whitespace_and_comments().is_ok() {}
                        Ok(())
                    },
                )?;
                import = false;
            }
            Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => import = true,
            Token::Semicolon => import = false,
            Token::UnquotedUrl(url) => {
                let kind = if import { Kind::Css } else { Kind::Asset };
                edits.push((
                    start,
                    parser.position().byte_index(),
                    format!("url({})", quoted(&resolve(&url, kind))),
                ));
                import = false;
            }
            Token::QuotedString(url) if import || image_set => {
                let kind = if import { Kind::Css } else { Kind::Asset };
                edits.push((
                    start,
                    parser.position().byte_index(),
                    quoted(&resolve(&url, kind)),
                ));
                import = false;
            }
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let url = parser
                    .parse_nested_block(|nested| Ok(nested.expect_string()?.to_string()))
                    .map_err(|_: cssparser::ParseError<'i, ()>| parser.new_custom_error(()))?;
                let kind = if import { Kind::Css } else { Kind::Asset };
                edits.push((
                    start,
                    parser.position().byte_index(),
                    format!("url({})", quoted(&resolve(&url, kind))),
                ));
                import = false;
            }
            Token::Function(name) => parser.parse_nested_block(|nested| {
                walk(
                    nested,
                    resolve,
                    edits,
                    depth + 1,
                    name.eq_ignore_ascii_case("image-set")
                        || name.eq_ignore_ascii_case("-webkit-image-set"),
                )
            })?,
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                parser
                    .parse_nested_block(|nested| walk(nested, resolve, edits, depth + 1, false))?;
                import = false;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespace_identifiers_are_preserved_without_scheduling_downloads() {
        let input = r##"@namespace url(http://www.w3.org/1999/xhtml); @namespace svg url("http://www.w3.org/2000/svg"); svg|rect{fill:url(#paint)}"##;
        let mut found = Vec::new();
        let output = rewrite(input, |url, _| {
            found.push(url.to_string());
            "local-paint".into()
        })
        .unwrap();
        assert_eq!(found, ["#paint"]);
        assert!(output.starts_with(r#"@namespace url(http://www.w3.org/1999/xhtml); @namespace svg url("http://www.w3.org/2000/svg");"#));
        assert!(output.ends_with("svg|rect{fill:url(\"local-paint\")}"));
    }

    #[test]
    fn css_tokens_rewrite_imports_escapes_nested_urls_and_image_sets_only() {
        let css = r#"/* url(fake.png) */ @import "theme.css" screen; @media (min-width: 1px) { .x { background: URL('im\61 ge.png'); content: "url(fake2.png)"; mask: image-set("hi.png" 2x, url(lo.png) 1x); } }"#;
        let mut found = Vec::new();
        let output = rewrite(css, |url, kind| {
            found.push((url.to_string(), kind));
            format!("saved-{url}")
        })
        .unwrap();
        assert_eq!(
            found,
            [
                ("theme.css".into(), Kind::Css),
                ("image.png".into(), Kind::Asset),
                ("hi.png".into(), Kind::Asset),
                ("lo.png".into(), Kind::Asset)
            ]
        );
        assert!(output.contains("/* url(fake.png) */"));
        assert!(output.contains("content: \"url(fake2.png)\""));
        assert!(output.contains("url(\"saved-image.png\")"));
    }
}
