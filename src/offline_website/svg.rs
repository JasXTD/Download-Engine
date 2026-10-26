//! Retain SVG drawing content without active elements or network dependencies,
//! including when the user opens an individual saved SVG from the report.
use super::*;
use quick_xml::{
    Reader, Writer,
    events::{BytesStart, BytesText, Event},
};

fn static_css(text: &str, changed: &mut bool) -> Result<String> {
    css::rewrite(text, |raw, _| {
        if raw.starts_with('#') {
            raw.into()
        } else {
            *changed = true;
            "data:,".into()
        }
    })
}

pub(super) fn sanitize(bytes: &[u8]) -> Result<(Vec<u8>, bool)> {
    let text = std::str::from_utf8(bytes).context("Only UTF-8 SVG is supported")?;
    let mut reader = Reader::from_str(text);
    let mut writer = Writer::new(Vec::new());
    let mut skipped = 0usize;
    let mut stack = Vec::new();
    let mut style_text: Option<String> = None;
    let mut changed = false;
    let mut count = 0;
    loop {
        let event = reader.read_event().context("Invalid SVG")?;
        count += 1;
        ensure!(count <= 100_000 && stack.len() <= 256, "SVG is too complex");
        match event {
            Event::Start(ref original) | Event::Empty(ref original) => {
                ensure!(
                    style_text.is_none(),
                    "Nested markup in SVG styles is not supported"
                );
                let local = String::from_utf8(original.local_name().as_ref().to_vec())?;
                let empty = matches!(event, Event::Empty(_));
                let allowed = matches!(
                    local.as_str(),
                    "svg"
                        | "g"
                        | "defs"
                        | "path"
                        | "rect"
                        | "circle"
                        | "ellipse"
                        | "line"
                        | "polyline"
                        | "polygon"
                        | "text"
                        | "tspan"
                        | "textPath"
                        | "use"
                        | "image"
                        | "symbol"
                        | "view"
                        | "clipPath"
                        | "mask"
                        | "linearGradient"
                        | "radialGradient"
                        | "stop"
                        | "pattern"
                        | "marker"
                        | "title"
                        | "desc"
                        | "style"
                        | "filter"
                        | "feBlend"
                        | "feColorMatrix"
                        | "feComponentTransfer"
                        | "feComposite"
                        | "feConvolveMatrix"
                        | "feDiffuseLighting"
                        | "feDisplacementMap"
                        | "feDistantLight"
                        | "feDropShadow"
                        | "feFlood"
                        | "feFuncA"
                        | "feFuncB"
                        | "feFuncG"
                        | "feFuncR"
                        | "feGaussianBlur"
                        | "feMerge"
                        | "feMergeNode"
                        | "feMorphology"
                        | "feOffset"
                        | "fePointLight"
                        | "feSpecularLighting"
                        | "feSpotLight"
                        | "feTile"
                        | "feTurbulence"
                );
                if !allowed || skipped > 0 {
                    changed = true;
                    if !empty {
                        skipped += 1;
                        stack.push(local);
                    }
                    continue;
                }
                let name = std::str::from_utf8(original.name().as_ref())?.to_owned();
                let mut start = BytesStart::new(name);
                for attribute in original.attributes() {
                    let attribute = attribute?;
                    let key = std::str::from_utf8(attribute.key.as_ref())?;
                    if key.to_ascii_lowercase().starts_with("on") || key == "xml:base" {
                        changed = true;
                        continue;
                    }
                    let value = attribute.decoded_and_normalized_value(
                        quick_xml::XmlVersion::Implicit1_0,
                        reader.decoder(),
                    )?;
                    let value = if key == "href" || key.ends_with(":href") {
                        if value.starts_with('#') {
                            value.into_owned()
                        } else {
                            changed = true;
                            "data:,".into()
                        }
                    } else if matches!(
                        key,
                        "style"
                            | "fill"
                            | "stroke"
                            | "filter"
                            | "clip-path"
                            | "mask"
                            | "cursor"
                            | "marker-start"
                            | "marker-mid"
                            | "marker-end"
                    ) {
                        static_css(&value, &mut changed)?
                    } else {
                        value.into_owned()
                    };
                    start.push_attribute((key, value.as_str()));
                }
                if empty {
                    writer.write_event(Event::Empty(start))?;
                } else {
                    writer.write_event(Event::Start(start))?;
                    if local == "style" {
                        style_text = Some(String::new());
                    }
                    stack.push(local);
                }
            }
            Event::End(end) => {
                ensure!(!stack.is_empty(), "Invalid SVG nesting");
                let local = stack.pop().unwrap();
                if skipped > 0 {
                    skipped -= 1;
                } else {
                    if local == "style" {
                        let text = style_text.take().context("Missing SVG style content")?;
                        let clean = static_css(&text, &mut changed)?;
                        writer.write_event(Event::Text(BytesText::new(&clean)))?;
                    }
                    writer.write_event(Event::End(end))?;
                }
            }
            // Browsers concatenate XML text, CDATA and character references
            // before parsing CSS. Rewriting individual events misses URLs whose
            // tokens span those boundaries.
            Event::Text(text) if style_text.is_some() => {
                style_text.as_mut().unwrap().push_str(&text.decode()?);
            }
            Event::CData(data) if style_text.is_some() => {
                style_text.as_mut().unwrap().push_str(&data.decode()?);
            }
            Event::GeneralRef(reference) if style_text.is_some() => {
                let encoded = format!("&{};", reference.decode()?);
                style_text
                    .as_mut()
                    .unwrap()
                    .push_str(&quick_xml::escape::unescape(&encoded)?);
            }
            Event::Comment(_) if style_text.is_some() => {}
            Event::DocType(_) => {
                anyhow::bail!("SVG document types and external entities are not supported")
            }
            Event::PI(_) => {
                changed = true;
            }
            Event::Eof => break,
            event if skipped == 0 => writer.write_event(event)?,
            _ => {}
        }
    }
    ensure!(stack.is_empty(), "Unclosed SVG elements");
    Ok((writer.into_inner(), changed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_styles_are_rewritten_after_joining_xml_text_and_character_references() {
        let input = br##"<svg xmlns="http://www.w3.org/2000/svg"><style>@im<![CDATA[port]]> &quot;https://example.com/remote.css&quot;; .x {fill:u<![CDATA[rl]]>(https://example.com/image.svg);stroke:r&#103;b(1,2,3)}</style><path marker-start="url(https://example.com/marker.svg#m)" marker-end="url(#local)"/></svg>"##;
        let (output, changed) = sanitize(input).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(changed);
        assert!(!output.contains("https://"), "{output}");
        assert!(
            output.contains("rgb(1,2,3)") && output.contains("#local"),
            "{output}"
        );
    }

    #[test]
    fn saved_svg_is_static_even_when_opened_on_its_own() {
        let input = br##"<svg xmlns="http://www.w3.org/2000/svg" onload="bad()"><defs><linearGradient id="g"/></defs><rect fill="url(#g)" width="20" height="20"/><script>bad()</script><foreignObject><body>bad</body></foreignObject><image href="https://example.com/tracker.png"/><style>@import 'remote.css';.x{fill:url(https://example.com/image)}</style></svg>"##;
        let (output, changed) = sanitize(input).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(changed);
        assert!(
            !output.contains("bad")
                && !output.contains("example.com")
                && !output.contains("remote.css")
        );
        assert!(output.contains("<rect") && output.contains("#g"));
        assert!(
            sanitize(b"<!DOCTYPE svg [<!ENTITY e SYSTEM 'file:///secret'>]><svg>&e;</svg>")
                .is_err()
        );
    }
}
