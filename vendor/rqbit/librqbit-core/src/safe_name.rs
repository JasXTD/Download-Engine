//! Added by Download Engine. See ../../patch.md.
//!
//! File names in a torrent are chosen by strangers, and Windows gives some names a
//! special meaning. `C:name` replaces the download folder when joined to it, `a.txt:b`
//! writes a hidden stream, `CON` opens a device, and `a.` and `a` are the same file.
//! These rules match `safe_filename` in Download Engine's src/model.rs.

/// Windows limits a name to 255 UTF-16 units. Leaving room keeps full paths usable.
const MAX_UTF16_UNITS: usize = 160;

fn is_unsafe(c: char) -> bool {
    c.is_control()
        // Direction marks can make "gpj.exe" display as "exe.jpg".
        || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        || "<>:\"/\\|?*".contains(c)
}

/// Turns one part of a torrent's file path into a name that is safe on Windows. The
/// result is never empty, and joining it to a folder always stays inside that folder.
pub fn safe_path_component(value: &str) -> String {
    let mut units = 0;
    let replaced: String = value
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= MAX_UTF16_UNITS
        })
        .map(|c| if is_unsafe(c) { '_' } else { c })
        .collect();
    let mut result = replaced.trim().trim_end_matches(['.', ' ']).to_string();
    if result.is_empty() {
        result.push('_');
    }
    let stem = result
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let device_number = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || device_number.is_some_and(|n| n.chars().count() == 1 && "123456789¹²³".contains(n))
    {
        result.insert(0, '_');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::safe_path_component;

    #[test]
    fn test_empty_names_get_a_placeholder() {
        for name in ["", " ", ".", "..", " . "] {
            assert_eq!(safe_path_component(name), "_", "{name:?}");
        }
    }

    #[test]
    fn test_all_device_names() {
        for name in ["PRN", "aux", "CONOUT$", "COM9", "LPT1", "com¹", "LPT³.txt"] {
            assert!(safe_path_component(name).starts_with('_'), "{name:?}");
        }
        for name in ["COM0", "COM", "LPT10", "CONFIG", "PRINT"] {
            assert_eq!(safe_path_component(name), name);
        }
    }

    #[test]
    fn test_long_names_keep_whole_characters() {
        let result = safe_path_component(&"界".repeat(200));
        assert_eq!(result.chars().count(), 160);
        let result = safe_path_component(&"😀".repeat(200));
        assert_eq!(result.chars().count(), 80);
    }

    #[test]
    fn test_shortening_cannot_leave_a_trailing_dot() {
        let name = format!("{}.txt", "x".repeat(159));
        assert_eq!(safe_path_component(&name), "x".repeat(159));
    }
}
