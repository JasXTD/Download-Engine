//! Content-Disposition is advisory metadata, never a destination path.
use crate::model::{Job, safe_filename};
use reqwest::header::{CONTENT_DISPOSITION, HeaderMap};

/// Resolve once, before saving a transfer plan or writing bytes. Retries, resume,
/// link replacement and recovery must not rename an already-started download.
pub(crate) fn resolve(job: &mut Job, headers: &HeaderMap) {
    if !job.server_name_pending {
        return;
    }
    if job.plan.is_none()
        && job.downloaded == 0
        && job.saved_path.is_none()
        && let Some(name) = filename(headers)
    {
        job.name = name;
    }
    job.server_name_pending = false;
}

fn filename(headers: &HeaderMap) -> Option<String> {
    let mut values = headers.get_all(CONTENT_DISPOSITION).iter();
    let value = values.next()?.as_bytes();
    // This field is not a list. Ambiguous or excessive metadata falls back to
    // the URL name without turning an otherwise valid download into a failure.
    if values.next().is_some() || value.len() > 8192 {
        return None;
    }
    parse(value)
}
fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_\x60|~".contains(&byte)
}
fn whitespace(input: &mut &[u8]) {
    while input.first().is_some_and(|b| matches!(b, b' ' | b'\t')) {
        *input = &input[1..];
    }
}
fn token<'a>(input: &mut &'a [u8]) -> Option<&'a [u8]> {
    let end = input
        .iter()
        .position(|b| !token_byte(*b))
        .unwrap_or(input.len());
    let value = &input[..end];
    *input = &input[end..];
    (!value.is_empty()).then_some(value)
}
fn parameter(input: &mut &[u8]) -> Option<(Vec<u8>, bool)> {
    if input.first() != Some(&b'"') {
        return Some((token(input)?.to_vec(), false));
    }
    *input = &input[1..];
    let mut value = Vec::new();
    while let Some((&byte, rest)) = input.split_first() {
        *input = rest;
        match byte {
            b'"' => return Some((value, true)),
            b'\\' => {
                let (&escaped, rest) = input.split_first()?;
                value.push(escaped);
                *input = rest;
            }
            _ => value.push(byte),
        }
    }
    None
}
fn parse(mut input: &[u8]) -> Option<String> {
    if input
        .iter()
        .any(|b| (*b < 0x20 && *b != b'\t') || *b == 0x7f)
    {
        return None;
    }
    whitespace(&mut input);
    token(&mut input)?; // disposition type (attachment, inline or an extension)
    let mut seen = std::collections::HashSet::new();
    let mut plain = None;
    let mut extended = None;
    loop {
        whitespace(&mut input);
        if input.is_empty() {
            break;
        }
        input = input.strip_prefix(b";")?;
        whitespace(&mut input);
        let key = token(&mut input)?.to_ascii_lowercase();
        if !seen.insert(key.clone()) {
            return None; // RFC 6266: repeated parameters are invalid.
        }
        whitespace(&mut input);
        input = input.strip_prefix(b"=")?;
        whitespace(&mut input);
        let (value, quoted) = parameter(&mut input)?;
        match key.as_slice() {
            b"filename" => plain = safe_leaf(&legacy_text(&value)),
            b"filename*" if !quoted => extended = extended_text(&value).and_then(|s| safe_leaf(&s)),
            _ => {}
        }
    }
    // RFC 6266 / 8187: prefer filename*, regardless of parameter order.
    extended.or(plain)
}
fn legacy_text(bytes: &[u8]) -> String {
    // Accept common UTF-8 senders as well as the historical ISO-8859-1 form.
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .unwrap_or_else(|_| bytes.iter().map(|b| char::from(*b)).collect())
}
fn extended_text(bytes: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(bytes).ok()?;
    let mut pieces = value.splitn(3, '\'');
    let charset = pieces.next()?;
    let language = pieces.next()?;
    let encoded = pieces.next()?.as_bytes();
    if !language
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    let mut decoded = Vec::new();
    let mut index = 0;
    while index < encoded.len() {
        let byte = encoded[index];
        if byte == b'%' {
            let high = (*encoded.get(index + 1)? as char).to_digit(16)?;
            let low = (*encoded.get(index + 2)? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            if !token_byte(byte) || matches!(byte, b'*' | b'\'') {
                return None;
            }
            decoded.push(byte);
            index += 1;
        }
    }
    if charset.eq_ignore_ascii_case("UTF-8") {
        String::from_utf8(decoded).ok()
    } else if charset.eq_ignore_ascii_case("ISO-8859-1") {
        Some(decoded.into_iter().map(char::from).collect())
    } else {
        None
    }
}
fn safe_leaf(value: &str) -> Option<String> {
    let leaf = value.rsplit(['/', '\\']).next()?.trim();
    if leaf.trim_matches(['.', ' ']).is_empty() {
        return None;
    }
    Some(safe_filename(leaf))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn name(value: &str) -> Option<String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_DISPOSITION, value.parse().unwrap());
        filename(&headers)
    }
    #[test]
    fn quoted_tokens_unicode_and_extended_precedence() {
        for (header, expected) in [
            ("attachment; filename=report.pdf", "report.pdf"),
            (
                r#"INLINE; FileName = "a;b \"report\".pdf" ; size=12"#,
                "a;b _report_.pdf",
            ),
            (
                "attachment; filename=old.txt; filename*=UTF-8'en'%E6%97%85%E8%A1%8C.pdf",
                "旅行.pdf",
            ),
            (
                "attachment; filename*=utf-8''caf%C3%A9.zip; filename=old.zip",
                "café.zip",
            ),
            ("attachment; filename*=ISO-8859-1''caf%E9.zip", "café.zip"),
            ("attachment; filename=100%25.zip", "100%25.zip"),
            ("attachment; filename*=UTF-8''a+b.pdf", "a+b.pdf"),
        ] {
            assert_eq!(name(header).as_deref(), Some(expected), "{header}");
        }
    }
    #[test]
    fn bad_or_ambiguous_metadata_falls_back_without_guessing() {
        for header in [
            "attachment",
            "attachment; filename=",
            "attachment; filename=one; FILENAME=two",
            "attachment; filename*=UTF-8''one; filename*=UTF-8''two",
            "attachment; filename=two words.txt",
            "attachment; filename=one, attachment; filename=two",
            "attachment; filename=one;",
            "attachment; filename=\"unclosed",
            "attachment; filename=\"..\"",
            "attachment; filename*=UTF-8''bad%Q1",
            "attachment; filename*=UTF-8''bad%",
            "attachment; filename*=UTF-8''%FF",
            "attachment; filename*=unknown''test",
            "attachment; filename*=\"UTF-8''quoted\"",
        ] {
            assert_eq!(name(header), None, "{header}");
        }
        for invalid in ["UTF-8''bad%Q1", "unknown''test", "UTF-8''%FF"] {
            assert_eq!(
                name(&format!(
                    "attachment; filename=fallback.txt; filename*={invalid}"
                )),
                Some("fallback.txt".into())
            );
        }
        let mut headers = HeaderMap::new();
        for _ in 0..2 {
            headers.append(
                CONTENT_DISPOSITION,
                "attachment; filename=a.pdf".parse().unwrap(),
            );
        }
        assert_eq!(filename(&headers), None);
        assert_eq!(
            name(&format!("attachment; filename={}", "x".repeat(8192))),
            None
        );
    }
    #[test]
    fn server_names_cannot_choose_paths_or_hide_extensions() {
        for (value, expected) in [
            ("UTF-8''..%2F..%2Freport.pdf", "report.pdf"),
            ("UTF-8''C%3A%5Ctmp%5CCON.txt", "_CON.txt"),
            ("UTF-8''report%3Aevil.exe", "report_evil.exe"),
            ("UTF-8''invoice%E2%80%AEfdp.exe", "invoice_fdp.exe"),
            ("UTF-8''a%00.txt", "a_.txt"),
        ] {
            assert_eq!(
                name(&format!("attachment; filename*={value}")).as_deref(),
                Some(expected)
            );
        }
        assert!(
            name(&format!("attachment; filename={}", "x".repeat(200)))
                .unwrap()
                .encode_utf16()
                .count()
                <= 160
        );
    }
}
