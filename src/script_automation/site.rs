//! Page patterns that hand a video page to a user's Lua site reader.
//!
//! A pattern is `host/path`: the host may start with `*.` for any subdomain,
//! and `*` in the path matches any characters, including `/`. A pattern without
//! a path covers the whole site. Scheme, query and fragment are not part of a
//! pattern; only default ports match.
use anyhow::{Result, bail, ensure};
use reqwest::Url;

pub const MAX_PAGES: usize = 20;
const MAX_PAGE_LEN: usize = 200;

/// The stored form of a pattern typed by the user: trimmed, without an
/// `http://` or `https://` prefix, with a lower-case host and an explicit path.
pub fn normalize_page(value: &str) -> Result<String> {
    let mut text = value.trim();
    for scheme in ["https://", "http://"] {
        if text.len() >= scheme.len() && text[..scheme.len()].eq_ignore_ascii_case(scheme) {
            text = &text[scheme.len()..];
        }
    }
    ensure!(!text.is_empty(), "Enter a page such as example.com/watch/*");
    ensure!(
        text.len() <= MAX_PAGE_LEN,
        "A page pattern can be at most {MAX_PAGE_LEN} characters"
    );
    ensure!(
        !text.contains(['?', '#']),
        "Page patterns match the address without its ?query or #fragment"
    );
    let (host, path) = match text.find('/') {
        Some(at) => (&text[..at], &text[at..]),
        None => (text, "/*"),
    };
    let host = host.to_ascii_lowercase();
    let bare = host.strip_prefix("*.").unwrap_or(&host);
    ensure!(
        bare.contains('.')
            && bare.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }),
        "Start a page pattern with a site name such as example.com or *.example.com"
    );
    ensure!(
        path.bytes().all(|b| b.is_ascii_graphic()),
        "A page pattern cannot contain spaces or non-ASCII characters"
    );
    Ok(format!("{host}{path}"))
}

pub fn validate_pages(pages: &[String]) -> Result<()> {
    ensure!(
        !pages.is_empty(),
        "Add at least one page, such as example.com/watch/*"
    );
    ensure!(
        pages.len() <= MAX_PAGES,
        "A site reader can list at most {MAX_PAGES} pages"
    );
    for (index, page) in pages.iter().enumerate() {
        if normalize_page(page)? != *page {
            bail!("Invalid saved page pattern: {page}");
        }
        ensure!(
            !pages[..index].contains(page),
            "This page is listed twice: {page}"
        );
    }
    Ok(())
}

/// Whether a normalized pattern covers this page address.
pub fn page_matches(pattern: &str, url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") || url.port().is_some() {
        return false;
    }
    let (Some(host), Some(at)) = (url.host_str(), pattern.find('/')) else {
        return false;
    };
    let (want, path) = pattern.split_at(at);
    let host_matches = match want.strip_prefix("*.") {
        Some(domain) => host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.len() > 1 && prefix.ends_with('.')),
        None => host == want,
    };
    host_matches && glob(path.as_bytes(), url.path().as_bytes())
}

/// `*` matches any run of bytes; everything else must match exactly.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&b| b == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_pages_are_normalized_or_explained() {
        for (typed, stored) in [
            ("example.com/watch/*", "example.com/watch/*"),
            ("  HTTPS://Example.COM/Watch/*  ", "example.com/Watch/*"),
            (
                "http://*.example.co.uk/v/*/play",
                "*.example.co.uk/v/*/play",
            ),
            ("example.com", "example.com/*"),
        ] {
            assert_eq!(normalize_page(typed).unwrap(), stored, "{typed}");
        }
        for bad in [
            "",
            "localhost/*",
            "*/watch",
            "a.*.example.com/*",
            "example.com:8080/*",
            "example.com/watch?id=*",
            "example.com/watch#t",
            "exa mple.com/*",
            "example.com/wa tch",
            "-example.com/*",
            "ftp://example.com/*",
        ] {
            assert!(normalize_page(bad).is_err(), "{bad}");
        }
        assert!(normalize_page(&format!("example.com/{}", "a".repeat(200))).is_err());
        validate_pages(&["example.com/watch/*".into()]).unwrap();
        assert!(validate_pages(&[]).is_err());
        assert!(validate_pages(&["Example.com/*".into()]).is_err());
        assert!(validate_pages(&["example.com/*".into(), "example.com/*".into()]).is_err());
        assert!(validate_pages(&vec!["example.com/*".to_string(); 21]).is_err());
    }

    #[test]
    fn patterns_match_host_and_path_only() {
        let url = |value: &str| Url::parse(value).unwrap();
        let page = "example.com/watch/*";
        assert!(page_matches(page, &url("https://example.com/watch/a1b2")));
        assert!(page_matches(
            page,
            &url("http://example.com/watch/a1b2?t=30#x")
        ));
        assert!(page_matches(page, &url("https://example.com/watch/a/b")));
        assert!(!page_matches(page, &url("https://example.com/watch")));
        assert!(!page_matches(page, &url("https://www.example.com/watch/a")));
        assert!(!page_matches(
            page,
            &url("https://example.com:8443/watch/a")
        ));
        assert!(!page_matches(
            page,
            &url("https://example.com.evil.test/watch/a")
        ));
        let any = "*.example.com/v/*/play";
        assert!(page_matches(any, &url("https://m.example.com/v/12/play")));
        assert!(page_matches(
            any,
            &url("https://a.b.example.com/v/1/2/play")
        ));
        assert!(!page_matches(any, &url("https://example.com/v/12/play")));
        assert!(!page_matches(any, &url("https://badexample.com/v/12/play")));
        assert!(!page_matches(
            any,
            &url("https://m.example.com/v/12/play/more")
        ));
        assert!(page_matches("example.com/*", &url("https://example.com/")));
        assert!(!page_matches("example.com/*", &url("ftp://example.com/x")));
    }
}
