//! Portable definitions only: no saved identity, enablement, run logs or stored values.
use super::{Language, Script, Trigger, api};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 512 * 1024;
pub const EXTENSION: &str = "de-script.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    name: String,
    language: Language,
    body: String,
    trigger: Trigger,
    timeout_seconds: u32,
    change_files: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    format: String,
    format_version: u32,
    api_version: u32,
    script: Definition,
}

pub struct Imported {
    pub script: Script,
    pub requested_file_access: bool,
    pub needs_queue: bool,
}

pub fn encode(script: &Script) -> Result<Vec<u8>> {
    script.validate()?;
    let package = Package {
        format: "download-engine-script".into(),
        format_version: 1,
        api_version: api::VERSION,
        script: Definition {
            name: script.name.clone(),
            language: script.language,
            body: script.body.clone(),
            trigger: script.trigger.clone(),
            timeout_seconds: script.timeout_seconds,
            change_files: script.change_files,
        },
    };
    let mut bytes = serde_json::to_vec_pretty(&package)?;
    bytes.push(b'\n');
    ensure!(bytes.len() <= MAX_BYTES, "Script package exceeds 512 KiB");
    Ok(bytes)
}

/// Decoding never saves or executes anything. Each import starts as a new disabled draft.
pub fn decode(bytes: &[u8]) -> Result<Imported> {
    ensure!(bytes.len() <= MAX_BYTES, "Script package exceeds 512 KiB");
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let p: Package = serde_json::from_slice(bytes)?;
    ensure!(
        p.format == "download-engine-script" && p.format_version == 1,
        "Unsupported script package format"
    );
    ensure!(
        p.api_version == api::VERSION,
        "This script needs Lua API {}; this app provides API {}",
        p.api_version,
        api::VERSION
    );
    let mut script = Script {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 0,
        name: p.script.name,
        language: p.script.language,
        body: p.script.body,
        enabled: false,
        trigger: p.script.trigger,
        timeout_seconds: p.script.timeout_seconds,
        change_files: p.script.change_files,
    };
    script.validate()?;
    let requested_file_access = script.change_files;
    script.change_files = false;
    // Queue IDs belong to the exporting profile. Do not silently bind to a different
    // queue or broaden a queue-specific event to all queues on the importing PC.
    let queue = match &mut script.trigger {
        Trigger::DownloadCompleted { queue } | Trigger::DownloadFailed { queue } => queue.as_mut(),
        Trigger::QueueCompleted { queue } => Some(queue),
        _ => None,
    };
    let needs_queue = queue.is_some();
    if let Some(queue) = queue {
        queue.clear();
    }
    Ok(Imported {
        script,
        requested_file_access,
        needs_queue,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn script(trigger: Trigger) -> Script {
        Script {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 7,
            name: "My reader".into(),
            language: Language::Lua,
            body: "print('hello')".into(),
            enabled: true,
            trigger,
            timeout_seconds: 30,
            change_files: false,
        }
    }
    #[test]
    fn packages_preserve_code_and_triggers_but_never_grant_permissions_or_identity() {
        for trigger in [
            Trigger::Manual,
            Trigger::Startup,
            Trigger::Interval { minutes: 4 },
            Trigger::SiteReader {
                pages: vec!["example.com/*".into()],
            },
        ] {
            let source = script(trigger);
            let bytes = encode(&source).unwrap();
            let a = decode(&bytes).unwrap();
            let b = decode(&bytes).unwrap();
            assert_eq!(a.script.body, source.body);
            assert_eq!(a.script.trigger, source.trigger);
            assert_ne!(a.script.id, source.id);
            assert_ne!(a.script.id, b.script.id);
            assert_eq!(a.script.revision, 0);
            assert!(!a.script.enabled && !a.script.change_files);
        }
        let mut source = script(Trigger::DownloadFailed {
            queue: Some("foreign-queue".into()),
        });
        source.change_files = true;
        let a = decode(&encode(&source).unwrap()).unwrap();
        assert!(a.requested_file_access && a.needs_queue);
        assert!(!a.script.change_files);
        assert_eq!(
            a.script.trigger,
            Trigger::DownloadFailed {
                queue: Some(String::new())
            }
        );
    }
    #[test]
    fn malformed_or_incompatible_packages_are_refused_before_import() {
        let source = script(Trigger::Manual);
        let value: serde_json::Value = serde_json::from_slice(&encode(&source).unwrap()).unwrap();
        for (field, invalid) in [
            ("format", serde_json::json!("other")),
            ("format_version", serde_json::json!(2)),
            ("api_version", serde_json::json!(0)),
            ("api_version", serde_json::json!(99)),
            ("enabled", serde_json::json!(true)),
        ] {
            let mut changed = value.clone();
            changed[field] = invalid;
            assert!(decode(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        for (field, invalid) in [
            ("body", serde_json::json!("")),
            ("timeout_seconds", serde_json::json!(0)),
            ("enabled", serde_json::json!(true)),
            ("id", serde_json::json!("victim")),
        ] {
            let mut changed = value.clone();
            changed["script"][field] = invalid;
            assert!(decode(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        assert!(decode(&vec![b' '; MAX_BYTES + 1]).is_err());
    }
}
