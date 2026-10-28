use super::*;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Suggestion {
    pub name: String,
    pub signature: String,
    pub function: bool,
    pub arguments: bool,
}
impl Suggestion {
    fn new(name: &str, args: Option<&str>) -> Self {
        Self {
            name: name.into(),
            signature: args.map_or(name.into(), |a| format!("{name}({a})")),
            function: args.is_some(),
            arguments: args.is_some_and(|a| !a.is_empty()),
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Choices {
    /// Byte range of the complete token, including its part after the caret.
    pub replace: Range<usize>,
    pub items: Vec<Suggestion>,
}
fn allowed(c: char, language: Language) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '.')
        || (language == Language::PowerShell && matches!(c, '$' | '-'))
}

pub(super) fn choices(
    source: &str,
    cursor: usize,
    language: Language,
    reader: bool,
    explicit: bool,
) -> Option<Choices> {
    let at = byte_at(source, cursor);
    let tokens = highlight::tokens(source, language);
    if tokens.iter().any(|t| {
        matches!(t.kind, highlight::Kind::String | highlight::Kind::Comment)
            && t.range.start < at
            && at <= t.range.end
    }) {
        return None;
    }
    let start = source[..at]
        .char_indices()
        .rev()
        .find(|(_, c)| !allowed(*c, language))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = source[at..]
        .char_indices()
        .find(|(_, c)| !allowed(*c, language))
        .map_or(source.len(), |(i, _)| at + i);
    let prefix = &source[start..at];
    if !explicit && (prefix.len() < 2 || prefix.chars().next().is_some_and(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let mut items = Vec::new();
    let mut add = |name: &str, args: Option<&str>| items.push(Suggestion::new(name, args));
    if language == Language::Lua {
        use download_engine::script_automation::api;
        for function in api::FUNCTIONS.iter().filter(|f| f.available(reader)) {
            add(function.name, Some(function.arguments));
        }
        for property in api::PROPERTIES {
            add(property, None);
        }
        for (name, args) in [
            ("print", "..."),
            ("ipairs", "table"),
            ("pairs", "table"),
            ("type", "value"),
            ("tostring", "value"),
            ("tonumber", "value"),
            ("assert", "condition, message"),
            ("error", "message"),
            ("pcall", "function, ..."),
        ] {
            add(name, Some(args));
        }
        if reader {
            add("event.url", None);
        } else {
            for name in [
                "event.trigger",
                "event.downloads",
                "event.queue",
                "event.sample",
            ] {
                add(name, None);
            }
        }
        for (name, args) in [
            ("string.format", "format, ..."),
            ("string.find", "text, pattern"),
            ("string.match", "text, pattern"),
            ("string.gsub", "text, pattern, replacement"),
            ("string.sub", "text, start, stop"),
            ("string.lower", "text"),
            ("string.upper", "text"),
            ("string.len", "text"),
            ("table.insert", "table, value"),
            ("table.remove", "table, position"),
            ("table.concat", "table, separator"),
            ("table.sort", "table, compare"),
            ("table.unpack", "table"),
            ("math.floor", "number"),
            ("math.ceil", "number"),
            ("math.min", "..."),
            ("math.max", "..."),
            ("math.abs", "number"),
        ] {
            add(name, Some(args));
        }
        for name in highlight::LUA_KEYWORDS {
            add(name, None);
        }
    } else {
        for name in [
            "Invoke-DownloadEngine",
            "Write-Output",
            "Write-Warning",
            "Write-Error",
            "Start-Sleep",
            "ConvertTo-Json",
            "ConvertFrom-Json",
            "ForEach-Object",
            "Where-Object",
            "Select-Object",
            "$Event.trigger",
            "$Event.downloads",
            "$Event.queue",
            "$Event.sample",
            "$DownloadEngine",
            "$true",
            "$false",
            "$null",
        ] {
            add(name, None);
        }
        for name in highlight::PS_KEYWORDS {
            add(name, None);
        }
    }
    // Buffer words help with user-defined identifiers, without claiming type or
    // scope analysis. Never harvest words from comments or strings.
    for token in tokens {
        if token.range.start < start
            && matches!(
                token.kind,
                highlight::Kind::Word | highlight::Kind::Variable
            )
        {
            add(&source[token.range], None);
        }
    }
    let matches = |name: &str| {
        if language == Language::PowerShell {
            name.to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase())
        } else {
            name.starts_with(prefix)
        }
    };
    items.retain(|s| matches(&s.name) && (explicit || s.name != prefix));
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items.dedup_by(|a, b| a.name == b.name);
    items.truncate(40);
    (!items.is_empty()).then_some(Choices {
        replace: start..end,
        items,
    })
}

pub(super) fn byte_at(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(at, _)| at)
}
pub(super) fn insert(source: &mut String, choices: &Choices, selected: usize) -> Option<usize> {
    let item = choices.items.get(selected)?;
    let mut value = item.name.clone();
    let parentheses = item.function && !source[choices.replace.end..].trim_start().starts_with('(');
    if parentheses {
        value.push_str("()");
    }
    if source.len() - choices.replace.len() + value.len() > MAX_SCRIPT_BYTES {
        return None;
    }
    let cursor = source[..choices.replace.start].chars().count() + value.chars().count()
        - usize::from(parentheses && item.arguments);
    source.replace_range(choices.replace.clone(), &value);
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_respects_language_capabilities_and_literals() {
        let lua =
            |text: &str, reader| choices(text, text.chars().count(), Language::Lua, reader, false);
        assert!(
            lua("xd.", false)
                .unwrap()
                .items
                .iter()
                .any(|s| s.name == "xd.add")
        );
        assert!(
            !lua("xd.", true)
                .unwrap()
                .items
                .iter()
                .any(|s| s.name == "xd.add")
        );
        assert!(
            lua("xd.", true)
                .unwrap()
                .items
                .iter()
                .any(|s| s.name == "xd.fetch")
        );
        assert!(
            !lua("xd.", false)
                .unwrap()
                .items
                .iter()
                .any(|s| s.name == "xd.fetch")
        );
        for text in ["-- xd.", "'xd.", "[=[xd.", "--[[\nxd."] {
            assert!(lua(text, false).is_none());
        }
        assert!(choices("# Inv", 5, Language::PowerShell, false, true).is_none());
        assert!(
            choices("invoke-d", 8, Language::PowerShell, false, false)
                .unwrap()
                .items
                .iter()
                .any(|s| s.name == "Invoke-DownloadEngine")
        );
    }
    #[test]
    fn insertion_preserves_unicode_and_suffix_and_does_not_double_parentheses() {
        let mut text = "-- 🦀中文\nxd.queTAIL\nprint('next')".to_string();
        let caret = "-- 🦀中文\nxd.que".chars().count();
        let options = choices(&text, caret, Language::Lua, false, false).unwrap();
        let at = insert(&mut text, &options, 0).unwrap();
        assert_eq!(text, "-- 🦀中文\nxd.queues()\nprint('next')");
        assert_eq!(at, "-- 🦀中文\nxd.queues()".chars().count());
        let mut text = "xd.ad(url)".to_string();
        let options = choices(&text, 5, Language::Lua, false, false).unwrap();
        insert(&mut text, &options, 0).unwrap();
        assert_eq!(text, "xd.add(url)");
    }
}
