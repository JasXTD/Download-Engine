// Build-time generation keeps every translated format string checked by rustc.
// The JSON catalog is data, never source supplied by the running application.
pub fn generate() {
    println!("cargo:rerun-if-changed=locales/desktop.json");
    println!("cargo:rerun-if-changed=locales/additional.json");
    println!("cargo:rerun-if-changed=locales/languages.json");
    println!("cargo:rerun-if-changed=scripts/localization-build.rs");
    let mut catalog: std::collections::BTreeMap<String, Vec<String>> = serde_json::from_str(
        &std::fs::read_to_string("locales/desktop.json").expect("Read desktop language catalog"),
    )
    .expect("Each catalog entry must have three translations");
    let languages: Vec<serde_json::Value> = serde_json::from_str(
        &std::fs::read_to_string("locales/languages.json").expect("Read language registry"),
    )
    .expect("Parse language registry");
    let additional: std::collections::BTreeMap<String, Vec<String>> = serde_json::from_str(
        &std::fs::read_to_string("locales/additional.json")
            .expect("Read additional language catalog"),
    )
    .expect("Parse additional language catalog");
    for (key, values) in &mut catalog {
        assert_eq!(values.len(), 3, "Original translations: {key}");
        if let Some(extra) = additional.get(key) {
            assert_eq!(
                extra.len(),
                languages.len() - 4,
                "Additional translations: {key}"
            );
            values.extend(extra.iter().cloned());
        } else {
            values.resize(languages.len() - 1, key.clone());
        }
    }
    let quote = |value: &str| serde_json::to_string(value).unwrap();
    let mut source = String::from(
        "fn lookup(language: Language, text: &str) -> Option<&'static str> { match text {\n",
    );
    for (key, values) in &catalog {
        assert!(
            values.iter().all(|v| !v.trim().is_empty()),
            "Empty translation: {key}"
        );
        source.push_str(&format!("{} => match language {{", quote(key)));
        for (language, value) in languages.iter().skip(1).zip(values) {
            source.push_str(&format!(
                "Language::{} => Some({}),",
                language["variant"].as_str().expect("Language variant"),
                quote(value)
            ));
        }
        source.push_str("_ => None },\n");
    }
    source.push_str("_ => None, } }\n#[macro_export]\nmacro_rules! tf {\n");
    for (key, values) in &catalog {
        if !key.contains('{') {
            continue;
        }
        source.push_str(&format!(
            "({}, $($args:tt)*) => {{ match $crate::i18n::current() {{\n",
            quote(key)
        ));
        for (language, value) in languages.iter().skip(1).zip(values) {
            let variant = language["variant"].as_str().expect("Language variant");
            source.push_str(&format!(
                "$crate::i18n::Language::{variant} => format!({}, $($args)*),\n",
                quote(value)
            ));
        }
        source.push_str(&format!(
            "_ => format!({}, $($args)*), }} }};\n",
            quote(key)
        ));
    }
    source.push_str("}\n");
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("localization.rs"),
        source,
    )
    .expect("Write compiled localization catalog");
}
