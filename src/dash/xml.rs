//! Small bounded XML tree. DTDs, external entities and XLink are never resolved.
use anyhow::{Context, Result, bail, ensure};
use quick_xml::{NsReader, events::Event, name::ResolveResult};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(super) struct Node {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Node>,
    pub text: String,
}
impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |node| node.name == name)
    }
    pub fn one(&self, name: &str) -> Result<Option<&Node>> {
        let mut found = self.children.iter().filter(|node| node.name == name);
        let node = found.next();
        ensure!(
            found.next().is_none(),
            "Multiple DASH {name} elements are not supported"
        );
        Ok(node)
    }
}
pub(super) fn parse(text: &str) -> Result<Node> {
    ensure!(
        text.len() <= crate::hls::MAX_PLAYLIST_BYTES,
        "DASH manifest exceeds 1 MiB"
    );
    let mut reader = NsReader::from_str(text.trim_start_matches('\u{feff}'));
    let mut stack = vec![Node::default()];
    let mut count = 0;
    loop {
        let decoder = reader.decoder();
        let (namespace, event) = reader.read_resolved_event().context("Invalid DASH XML")?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                count += 1;
                ensure!(
                    count <= 50_000 && stack.len() <= 32,
                    "DASH XML is too complex"
                );
                let name = std::str::from_utf8(start.local_name().as_ref())?.to_owned();
                ensure!(
                    name != "ContentProtection",
                    "Encrypted / DRM DASH streams are not supported"
                );
                let known = match namespace {
                    ResolveResult::Unbound => true,
                    ResolveResult::Bound(ns) => ns.as_ref() == b"urn:mpeg:dash:schema:mpd:2011",
                    ResolveResult::Unknown(_) => bail!("DASH XML uses an undeclared namespace"),
                };
                let mut node = Node {
                    name: if known { name } else { String::new() },
                    ..Node::default()
                };
                for attribute in start.attributes() {
                    let attribute = attribute.context("Invalid DASH XML attribute")?;
                    let key = std::str::from_utf8(attribute.key.as_ref())?;
                    ensure!(
                        !key.ends_with(":href") && !key.ends_with(":actuate"),
                        "DASH XLink references are not supported"
                    );
                    ensure!(node.attrs.len() < 64, "Too many DASH attributes");
                    node.attrs.insert(
                        key.into(),
                        attribute
                            .decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                decoder,
                            )?
                            .into_owned(),
                    );
                }
                if matches!(event, Event::Empty(_)) {
                    stack.last_mut().unwrap().children.push(node);
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                ensure!(stack.len() > 1, "Invalid DASH XML nesting");
                let node = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(node);
            }
            Event::Text(text) => stack.last_mut().unwrap().text.push_str(&text.decode()?),
            Event::CData(text) => stack.last_mut().unwrap().text.push_str(&text.decode()?),
            Event::GeneralRef(reference) => {
                let value = reference.decode()?;
                let escaped = format!("&{value};");
                stack
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&quick_xml::escape::unescape(&escaped)?);
            }
            Event::DocType(_) => bail!("DASH XML document types and entities are not supported"),
            Event::PI(_) => bail!("DASH XML processing instructions are not supported"),
            Event::Decl(decl) => {
                if let Some(encoding) = decl.encoding() {
                    ensure!(
                        encoding?.eq_ignore_ascii_case(b"UTF-8"),
                        "DASH XML must be UTF-8"
                    );
                }
            }
            Event::Comment(_) => {}
            Event::Eof => break,
        }
    }
    ensure!(stack.len() == 1, "Incomplete DASH XML");
    let mut document = stack.pop().unwrap();
    ensure!(
        document.text.trim().is_empty() && document.children.len() == 1,
        "DASH needs one MPD document"
    );
    let root = document.children.pop().unwrap();
    ensure!(
        root.name == "MPD",
        "This URL did not return a DASH MPD manifest"
    );
    Ok(root)
}
