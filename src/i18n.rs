//! Display-only localization. Stored identifiers, CLI JSON and user content stay unchanged.
use serde::{Deserialize, Serialize};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
    #[serde(rename = "zh-TW")]
    TraditionalChinese,
    #[serde(rename = "ms")]
    Malay,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "ko")]
    Korean,
    #[serde(rename = "vi")]
    Vietnamese,
    #[serde(rename = "th")]
    Thai,
    #[serde(rename = "pl")]
    Polish,
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "uk")]
    Ukrainian,
    #[serde(rename = "ar")]
    Arabic,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "sv")]
    Swedish,
    #[serde(rename = "no")]
    NorwegianBokmal,
    #[serde(rename = "de-CH")]
    SwissGerman,
    #[serde(rename = "it")]
    Italian,
    #[serde(rename = "rm")]
    Romansh,
}

impl Language {
    pub const ALL: [Self; 20] = [
        Self::System,
        Self::English,
        Self::SimplifiedChinese,
        Self::TraditionalChinese,
        Self::Malay,
        Self::Japanese,
        Self::Korean,
        Self::Vietnamese,
        Self::Thai,
        Self::Polish,
        Self::Russian,
        Self::Ukrainian,
        Self::Arabic,
        Self::German,
        Self::French,
        Self::Swedish,
        Self::NorwegianBokmal,
        Self::SwissGerman,
        Self::Italian,
        Self::Romansh,
    ];
    pub fn from_locale(locale: &str) -> Self {
        let locale = locale.to_ascii_lowercase().replace('_', "-");
        let parts: Vec<_> = locale.split('-').collect();
        match parts.first().copied() {
            Some("zh")
                if parts.contains(&"hant")
                    || (!parts.contains(&"hans")
                        && parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo"))) =>
            {
                Self::TraditionalChinese
            }
            Some("zh") => Self::SimplifiedChinese,
            Some("ms") => Self::Malay,
            Some("ja") => Self::Japanese,
            Some("ko") => Self::Korean,
            Some("vi") => Self::Vietnamese,
            Some("th") => Self::Thai,
            Some("pl") => Self::Polish,
            Some("ru") => Self::Russian,
            Some("uk") => Self::Ukrainian,
            Some("ar") => Self::Arabic,
            Some("de") if parts.contains(&"ch") => Self::SwissGerman,
            Some("de") => Self::German,
            Some("fr") => Self::French,
            Some("sv") => Self::Swedish,
            Some("no" | "nb") => Self::NorwegianBokmal,
            Some("it") => Self::Italian,
            Some("rm") => Self::Romansh,
            _ => Self::English,
        }
    }
    pub fn resolved(self) -> Self {
        if self == Self::System {
            system_language()
        } else {
            self
        }
    }
    pub fn native_name(self) -> &'static str {
        match self {
            Self::System => tr("System language"),
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
            Self::TraditionalChinese => "繁體中文",
            Self::Malay => "Bahasa Melayu",
            Self::Japanese => "日本語",
            Self::Korean => "한국어",
            Self::Vietnamese => "Tiếng Việt",
            Self::Thai => "ไทย",
            Self::Polish => "Polski",
            Self::Russian => "Русский",
            Self::Ukrainian => "Українська",
            Self::Arabic => "العربية",
            Self::German => "Deutsch",
            Self::French => "Français",
            Self::Swedish => "Svenska",
            Self::NorwegianBokmal => "Norsk bokmål",
            Self::SwissGerman => "Deutsch (Schweiz)",
            Self::Italian => "Italiano",
            Self::Romansh => "Rumantsch Grischun",
        }
    }
}

fn system_language() -> Language {
    static VALUE: std::sync::OnceLock<Language> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        #[cfg(windows)]
        {
            let mut name = [0_u16; 85];
            // Follow Windows display language, which can differ from the user's
            // regional date/number format. Default sort order gives its LCID.
            let length = unsafe {
                windows_sys::Win32::Globalization::LCIDToLocaleName(
                    u32::from(windows_sys::Win32::Globalization::GetUserDefaultUILanguage()),
                    name.as_mut_ptr(),
                    name.len() as i32,
                    0,
                )
            };
            if length > 1 {
                return Language::from_locale(&String::from_utf16_lossy(
                    &name[..length as usize - 1],
                ));
            }
        }
        Language::English
    })
}

thread_local! { static LANGUAGE: Cell<Language> = const { Cell::new(Language::English) }; }
pub fn current() -> Language {
    LANGUAGE.get()
}

/// A scoped locale prevents tests, independent windows and background workers from
/// changing each other's language. Drop restores the caller's locale even on unwind.
#[must_use]
pub struct Scope(Language, std::marker::PhantomData<*mut ()>);
pub fn scope(language: Language) -> Scope {
    Scope(
        LANGUAGE.replace(language.resolved()),
        std::marker::PhantomData,
    )
}
impl Drop for Scope {
    fn drop(&mut self) {
        LANGUAGE.set(self.0);
    }
}

pub fn tr(text: &str) -> &str {
    lookup(current(), text).unwrap_or(text)
}

include!(concat!(env!("OUT_DIR"), "/localization.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regional_and_script_locales_resolve_without_touching_identifiers() {
        for locale in ["zh-CN", "zh-SG", "zh-Hans-HK", "zh_Hans"] {
            assert_eq!(Language::from_locale(locale), Language::SimplifiedChinese);
        }
        for locale in ["zh-TW", "zh-HK", "zh-MO", "zh-Hant-CN"] {
            assert_eq!(Language::from_locale(locale), Language::TraditionalChinese);
        }
        assert_eq!(Language::from_locale("ms-MY"), Language::Malay);
        for (locale, expected) in [
            ("ja-JP", Language::Japanese),
            ("ko-KR", Language::Korean),
            ("vi-VN", Language::Vietnamese),
            ("th-TH", Language::Thai),
            ("pl-PL", Language::Polish),
            ("ru-RU", Language::Russian),
            ("uk-UA", Language::Ukrainian),
            ("ar-SA", Language::Arabic),
            ("ar_EG", Language::Arabic),
            ("de-DE", Language::German),
            ("de-AT", Language::German),
            ("de_CH", Language::SwissGerman),
            ("de-Latn-CH", Language::SwissGerman),
            ("fr-FR", Language::French),
            ("fr-CH", Language::French),
            ("sv-SE", Language::Swedish),
            ("nb-NO", Language::NorwegianBokmal),
            ("no-NO", Language::NorwegianBokmal),
            ("it-IT", Language::Italian),
            ("it_CH", Language::Italian),
            ("rm-CH", Language::Romansh),
            ("rm_Latn_CH", Language::Romansh),
        ] {
            assert_eq!(Language::from_locale(locale), expected);
        }
        assert_eq!(Language::from_locale("es-ES"), Language::English);
        assert_eq!(Language::from_locale("nn-NO"), Language::English);
    }
    #[test]
    fn scoped_language_restores_and_does_not_leak_to_workers() {
        assert_eq!(current(), Language::English);
        {
            let _scope = scope(Language::SimplifiedChinese);
            assert_eq!(tr("Settings"), "设置");
            assert_eq!(tr("user-file.zip"), "user-file.zip");
            assert_eq!(
                std::thread::spawn(|| tr("Settings")).join().unwrap(),
                "Settings"
            );
            let _nested = scope(Language::Malay);
            assert_eq!(tr("Settings"), "Tetapan");
        }
        assert_eq!(current(), Language::English);
    }
}
