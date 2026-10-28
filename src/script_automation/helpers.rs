//! Bounded, pure host helpers. No networking, file access or JavaScript execution.
use anyhow::{Result, ensure};
use mlua::{Lua, Table, Value};
use scraper::{Html, Selector};

const MAX_HTML: usize = 2 * 1024 * 1024;
const MAX_RESULTS: usize = 1000;
const MAX_RESULT_BYTES: usize = 1024 * 1024;

fn select(html: &str, selector: &str, attribute: Option<&str>) -> Result<Vec<String>> {
    ensure!(html.len() <= MAX_HTML, "xd.html_select HTML exceeds 2 MiB");
    ensure!(selector.len() <= 512, "CSS selector exceeds 512 bytes");
    ensure!(
        attribute.is_none_or(|a| !a.is_empty() && a.len() <= 128),
        "Invalid HTML attribute name"
    );
    let selector =
        Selector::parse(selector).map_err(|_| anyhow::anyhow!("Invalid CSS selector"))?;
    let document = Html::parse_document(html);
    let mut results = Vec::new();
    let mut bytes = 0;
    for node in document.select(&selector) {
        ensure!(
            results.len() < MAX_RESULTS,
            "HTML selection exceeds 1000 results"
        );
        let mut value = String::new();
        let mut append = |part: &str| -> Result<()> {
            bytes += part.len();
            ensure!(bytes <= MAX_RESULT_BYTES, "HTML selection exceeds 1 MiB");
            value.push_str(part);
            Ok(())
        };
        if let Some(attribute) = attribute {
            let Some(text) = node.value().attr(attribute) else {
                continue;
            };
            append(text)?;
        } else {
            for text in node.text() {
                append(text)?;
            }
        }
        results.push(value.trim().to_owned());
    }
    Ok(results)
}

pub(super) fn install(lua: &Lua, xd: &Table, reader: bool) -> mlua::Result<()> {
    use super::{api, lua::to_lua};
    xd.set("api_version", api::VERSION)?;
    xd.set("app_version", env!("CARGO_PKG_VERSION"))?;
    xd.set(
        "script_kind",
        if reader { "site_reader" } else { "automation" },
    )?;
    xd.set(
        "supports",
        lua.create_function(move |_, name: String| Ok(api::supports(&name, reader)))?,
    )?;
    xd.set(
        "json",
        lua.create_function(|lua, text: mlua::LuaString| {
            if text.as_bytes().len() > super::json::MAX_JSON_BYTES {
                return Err(mlua::Error::runtime("JSON input exceeds 8 MiB"));
            }
            let value: serde_json::Value = serde_json::from_str(&text.to_str()?)
                .map_err(|e| mlua::Error::runtime(format!("xd.json: invalid JSON ({e})")))?;
            to_lua(lua, &value)
        })?,
    )?;
    xd.set(
        "to_json",
        lua.create_function(|lua, value: Value| {
            let value = super::json::from_lua(lua, value, super::json::MAX_JSON_BYTES)?;
            serde_json::to_string(&value).map_err(mlua::Error::external)
        })?,
    )?;
    xd.set(
        "url_encode",
        lua.create_function(|_, text: String| {
            use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
            const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
                .remove(b'-')
                .remove(b'_')
                .remove(b'.')
                .remove(b'~');
            Ok(utf8_percent_encode(&text, COMPONENT).to_string())
        })?,
    )?;
    xd.set(
        "url_join",
        lua.create_function(|_, (base, relative): (String, String)| {
            let resolve = || -> Result<String> {
                ensure!(
                    base.len() <= 16384 && relative.len() <= 16384,
                    "URL exceeds 16 KiB"
                );
                let base = reqwest::Url::parse(&base)?;
                for url in [&base, &base.join(&relative)?] {
                    ensure!(
                        matches!(url.scheme(), "http" | "https")
                            && url.host_str().is_some()
                            && url.username().is_empty()
                            && url.password().is_none(),
                        "Use an HTTP(S) URL without embedded credentials"
                    );
                }
                let url = base.join(&relative)?.to_string();
                ensure!(url.len() <= 16384, "URL exceeds 16 KiB");
                Ok(url)
            };
            resolve().map_err(mlua::Error::external)
        })?,
    )?;
    xd.set(
        "html_select",
        lua.create_function(
            |lua, (html, selector, attribute): (String, String, Option<String>)| {
                let values = select(&html, &selector, attribute.as_deref())
                    .map_err(mlua::Error::external)?;
                to_lua(lua, &values)
            },
        )?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selectors_parse_entities_relative_links_and_nested_text_without_running_code() {
        let html = r#"<script>throw Error('never execute')</script><a class='file' href='../one.zip?a=1&amp;b=2'>One <b>file</b></a><a>Other</a>"#;
        assert_eq!(
            select(html, "a.file", Some("href")).unwrap(),
            ["../one.zip?a=1&b=2"]
        );
        assert_eq!(select(html, "a", None).unwrap(), ["One file", "Other"]);
        assert!(select(html, "video", None).unwrap().is_empty());
        assert!(select(html, "[", None).is_err());
        assert!(select(&"x".repeat(MAX_HTML + 1), "a", None).is_err());
        assert!(select(&"<a>x</a>".repeat(MAX_RESULTS + 1), "a", None).is_err());
        assert!(
            select(
                &format!("<p>{}</p>", "x".repeat(MAX_RESULT_BYTES + 1)),
                "p",
                None
            )
            .is_err()
        );
    }
}
