use super::*;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Plain,
    Word,
    Keyword,
    String,
    Comment,
    Number,
    Variable,
}
#[derive(Debug)]
pub(super) struct Token {
    pub range: Range<usize>,
    pub kind: Kind,
}

pub(super) const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];
pub(super) const PS_KEYWORDS: &[&str] = &[
    "begin", "break", "catch", "class", "continue", "data", "do", "else", "elseif", "end", "enum",
    "exit", "filter", "finally", "for", "foreach", "function", "if", "in", "param", "process",
    "return", "switch", "throw", "trap", "try", "until", "using", "while",
];

fn long_string(text: &str) -> Option<(usize, String)> {
    let rest = text.strip_prefix('[')?;
    let equal = rest.bytes().take_while(|c| *c == b'=').count();
    rest[equal..]
        .starts_with('[')
        .then(|| (equal + 2, format!("]{}]", "=".repeat(equal))))
}
pub(super) fn tokens(text: &str, language: Language) -> Vec<Token> {
    let mut result = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let tail = &text[at..];
        let c = tail.chars().next().unwrap();
        let mut end = at + c.len_utf8();
        let mut kind = Kind::Plain;
        let comment = if language == Language::Lua { "--" } else { "#" };
        if language == Language::PowerShell && tail.starts_with("<#") {
            end = tail[2..].find("#>").map_or(text.len(), |n| at + n + 4);
            kind = Kind::Comment;
        } else if let Some(after) = tail.strip_prefix(comment) {
            end = if language == Language::Lua
                && let Some((open, close)) = long_string(after)
            {
                after[open..]
                    .find(&close)
                    .map_or(text.len(), |n| at + comment.len() + open + n + close.len())
            } else {
                tail.find('\n').map_or(text.len(), |n| at + n)
            };
            kind = Kind::Comment;
        } else if language == Language::Lua
            && let Some((open, close)) = long_string(tail)
        {
            end = tail[open..]
                .find(&close)
                .map_or(text.len(), |n| at + open + n + close.len());
            kind = Kind::String;
        } else if language == Language::PowerShell
            && (tail.starts_with("@'\n")
                || tail.starts_with("@\"\n")
                || tail.starts_with("@'\r\n")
                || tail.starts_with("@\"\r\n"))
        {
            let delimiter = format!("\n{}@", tail.as_bytes()[1] as char);
            end = tail[2..]
                .find(&delimiter)
                .map_or(text.len(), |n| at + 2 + n + delimiter.len());
            kind = Kind::String;
        } else if c == '\'' || c == '"' {
            let escape = if language == Language::Lua { '\\' } else { '`' };
            while end < text.len() {
                let next = text[end..].chars().next().unwrap();
                end += next.len_utf8();
                if next == escape && (language == Language::Lua || c == '"') {
                    if let Some(escaped) = text[end..].chars().next() {
                        end += escaped.len_utf8();
                    }
                } else if next == c {
                    if language == Language::PowerShell && text[end..].starts_with(c) {
                        end += 1;
                    } else {
                        break;
                    }
                }
            }
            kind = Kind::String;
        } else if c.is_alphabetic() || c == '_' || (language == Language::PowerShell && c == '$') {
            kind = if c == '$' { Kind::Variable } else { Kind::Word };
            while let Some(next) = text[end..].chars().next() {
                if !next.is_alphanumeric()
                    && next != '_'
                    && !(language == Language::PowerShell && next == '-')
                {
                    break;
                }
                end += next.len_utf8();
            }
            let word = &text[at..end];
            if (language == Language::Lua && LUA_KEYWORDS.contains(&word))
                || (language == Language::PowerShell
                    && PS_KEYWORDS.contains(&word.to_ascii_lowercase().as_str()))
            {
                kind = Kind::Keyword;
            }
        } else if c.is_ascii_digit() {
            kind = Kind::Number;
            while let Some(next) = text[end..].chars().next() {
                if !next.is_ascii_hexdigit() && !['.', 'x', 'X', '_'].contains(&next) {
                    break;
                }
                end += next.len_utf8();
            }
        }
        result.push(Token {
            range: at..end,
            kind,
        });
        at = end;
    }
    result
}

pub(super) fn layout(text: &str, language: Language, dark: bool) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for token in tokens(text, language) {
        let color = match (token.kind, dark) {
            (Kind::Keyword, true) => Color32::from_rgb(196, 160, 255),
            (Kind::Keyword, false) => Color32::from_rgb(105, 42, 160),
            (Kind::String, true) => Color32::from_rgb(153, 212, 167),
            (Kind::String, false) => Color32::from_rgb(40, 112, 54),
            (Kind::Comment, true) => Color32::from_rgb(148, 161, 176),
            (Kind::Comment, false) => Color32::from_rgb(99, 111, 125),
            (Kind::Number, true) => Color32::from_rgb(238, 191, 123),
            (Kind::Number, false) => Color32::from_rgb(146, 82, 17),
            (Kind::Variable, true) => Color32::from_rgb(126, 200, 236),
            (Kind::Variable, false) => Color32::from_rgb(0, 103, 143),
            _ => Palette::new(dark).text,
        };
        job.append(
            &text[token.range],
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::monospace(13.0),
                color,
                ..Default::default()
            },
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lexer_preserves_unicode_and_multiline_literals() {
        for (language, source) in [
            (
                Language::Lua,
                "--[=[ comment\n xd.add ]=]\nlocal 名 = [==[🦀\nxd.]==]\nprint(名)",
            ),
            (
                Language::PowerShell,
                "<# multi\ncomment #>\n$名 = @'\nxd.add\n'@\nWrite-Output 'it''s 🦀'",
            ),
        ] {
            let spans = tokens(source, language);
            assert_eq!(
                spans
                    .iter()
                    .map(|t| &source[t.range.clone()])
                    .collect::<String>(),
                source
            );
            assert!(
                spans
                    .iter()
                    .any(|t| t.kind == Kind::Comment && source[t.range.clone()].contains('\n'))
            );
            assert!(
                spans
                    .iter()
                    .any(|t| t.kind == Kind::String && source[t.range.clone()].contains('\n'))
            );
            for dark in [true, false] {
                assert_eq!(layout(source, language, dark).text, source);
            }
        }
    }
}
