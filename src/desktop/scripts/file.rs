//! Open a .lua or .ps1 file into the script editor, or save the editor's code to one.
//! Opening copies the file once; nothing is saved until Save script.
use super::*;
use download_engine::script_automation::EXAMPLE;
use std::io::{Read, Write};

/// A button in the editor's header was clicked.
pub(super) enum Request {
    Open,
    Save,
}

/// Code read from a file, waiting to replace unsaved code in the editor.
pub(super) struct Loaded {
    file: String,
    stem: String,
    body: String,
    language: Language,
}

/// The most read from disk: a 64 KiB script saved as UTF-16 with a byte-order mark.
const MAX_FILE_BYTES: u64 = MAX_SCRIPT_BYTES as u64 * 2 + 2;

/// Text from a script file: UTF-8 with or without a byte-order mark, or UTF-16
/// with one, with Windows line endings converted. None when it is not text.
pub(super) fn decode(bytes: &[u8]) -> Option<String> {
    let utf16 = |rest: &[u8], little: bool| {
        let (pairs, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return None;
        }
        let units = pairs.iter().map(|&pair| {
            if little {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        });
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .ok()
    };
    let text = if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        String::from_utf8(rest.to_vec()).ok()?
    } else if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        utf16(rest, true)?
    } else if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        utf16(rest, false)?
    } else {
        String::from_utf8(bytes.to_vec()).ok()?
    };
    (!text.contains('\0')).then(|| text.replace("\r\n", "\n"))
}

/// The bytes saved for a script. Windows PowerShell 5.1 reads a .ps1 without a
/// byte-order mark in the system code page, so PowerShell files get one.
pub(super) fn encode(body: &str, language: Language) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(body.len() + 3);
    if language == Language::PowerShell {
        bytes.extend_from_slice(b"\xEF\xBB\xBF");
    }
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

fn extension(language: Language) -> &'static str {
    match language {
        Language::Lua => "lua",
        Language::PowerShell => "ps1",
    }
}

fn language_of(path: &std::path::Path) -> Option<Language> {
    match path
        .extension()?
        .to_string_lossy()
        .to_ascii_lowercase()
        .as_str()
    {
        "lua" => Some(Language::Lua),
        "ps1" => Some(Language::PowerShell),
        _ => None,
    }
}

fn read(path: &std::path::Path, reader: bool, current: Language) -> Result<Loaded, String> {
    let file = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let language = language_of(path).unwrap_or(current);
    if reader && language != Language::Lua {
        return Err(tr("Site readers are Lua only. Open a .lua file.").into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|handle| handle.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| tf!("Cannot open {file}: {error}", file = file, error = error))?;
    let too_large = || tf!("{file} is larger than 64 KiB.", file = file);
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(too_large());
    }
    let body = decode(&bytes).ok_or_else(|| {
        tf!(
            "{file} is not a text file. Save it as UTF-8 and try again.",
            file = file
        )
    })?;
    if body.len() > MAX_SCRIPT_BYTES {
        return Err(too_large());
    }
    Ok(Loaded {
        stem: path
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned()),
        file,
        body,
        language,
    })
}

/// The replace question as screenshots show it, for a file named other.lua.
#[cfg(feature = "screenshot")]
pub(super) fn screenshot_replace(state: &mut State) {
    state.replace = Some(Loaded {
        file: "other.lua".into(),
        stem: "other".into(),
        body: "print('other')\n".into(),
        language: Language::Lua,
    });
}

/// Whether replacing the editor's code loses nothing: it is the saved code or
/// an untouched starter example.
fn untouched(state: &State) -> bool {
    let Some(draft) = &state.draft else {
        return true;
    };
    match &state.baseline {
        Some(saved) => draft.body == saved.body,
        None => {
            [LUA_EXAMPLE, EXAMPLE, READER_EXAMPLE, FAILED_EXAMPLE].contains(&draft.body.as_str())
        }
    }
}

/// Put the file's code in the editor. A brand-new script also takes the file's name.
fn apply(state: &mut State, loaded: Loaded) -> Notice {
    if let Some(draft) = &mut state.draft {
        let stem = loaded.stem.trim();
        if draft.revision == 0 && draft.name == tr("New script") && !stem.is_empty() {
            draft.name = stem.chars().take(64).collect();
        }
        draft.language = loaded.language;
        draft.body = loaded.body;
    }
    state.error = None;
    Notice::info(tf!(
        "Loaded {file}. Choose Save script to keep it.",
        file = loaded.file
    ))
}

/// Replace a file atomically. The save dialog already asked before choosing an existing file.
fn write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let temporary = parent.join(format!("~{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

impl Desktop {
    pub(super) fn open_script_file(&mut self, state: &mut State) {
        let Some(draft) = &state.draft else {
            return;
        };
        let reader = matches!(draft.trigger, Trigger::SiteReader { .. });
        let filter = if reader {
            (tr("Lua script (*.lua)"), "*.lua")
        } else {
            (tr("Scripts (*.lua, *.ps1)"), "*.lua;*.ps1")
        };
        let path = match self.folder_picker.open_file(
            tr("Open script file"),
            &self.store.paths.downloads,
            filter,
        ) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                state.error = Some(tf!("Cannot choose a script file: {error}", error = error));
                return;
            }
        };
        match read(&path, reader, draft.language) {
            Ok(loaded) if untouched(state) => self.notice = Some(apply(state, loaded)),
            Ok(loaded) => {
                state.replace = Some(loaded);
                self.dialog = Some(Dialog::ScriptReplace);
            }
            Err(message) => state.error = Some(message),
        }
    }

    pub(super) fn save_script_file(&mut self, state: &mut State) {
        let Some(draft) = &state.draft else {
            return;
        };
        let extension = extension(draft.language);
        let name = format!(
            "{}.{extension}",
            download_engine::model::safe_filename(draft.name.trim())
        );
        let filter = match draft.language {
            Language::Lua => (tr("Lua script (*.lua)"), "*.lua"),
            Language::PowerShell => (tr("PowerShell script (*.ps1)"), "*.ps1"),
        };
        let path = match self.folder_picker.save_file(
            tr("Save script file"),
            &self.store.paths.downloads,
            &name,
            filter,
            extension,
        ) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                state.error = Some(tf!("Cannot choose a script file: {error}", error = error));
                return;
            }
        };
        match write(&path, &encode(&draft.body, draft.language)) {
            Ok(()) => {
                state.error = None;
                let file = path
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
                self.notice = Some(Notice::success(tf!("Saved {file}.", file = file)));
            }
            Err(error) => {
                state.error = Some(tf!("Cannot save the script file: {error}", error = error))
            }
        }
    }

    /// Asked only when opening a file would replace unsaved code.
    pub(in crate::desktop) fn script_replace_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        if self.scripts.replace.is_none() {
            self.dialog = None;
            return;
        }
        let name = self
            .scripts
            .draft
            .as_ref()
            .map_or_else(String::new, |draft| draft.name.clone());
        let mut close = false;
        let mut replace = false;
        let response = form_dialog::modal("script-replace-code", p).show(ctx, |ui| {
            ui.set_width(430.0);
            close = form_dialog::title(
                ui,
                tr("Replace the code in the editor?"),
                form_dialog::Icon::Symbol("\u{e7ba}"),
                p,
                true,
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(tf!(
                    "Your unsaved changes to “{script}” will be lost. The saved script stays as it is until you choose Save script.",
                    script = name
                ))
                .color(p.muted),
            );
            replace = form_dialog::controls(ui, |ui| {
                form_dialog::danger_button(ui, tr("Replace code"), p).clicked()
            });
        });
        if close || response.should_close() {
            self.scripts.replace = None;
            self.dialog = None;
        } else if replace && let Some(loaded) = self.scripts.replace.take() {
            self.dialog = None;
            self.notice = Some(apply(&mut self.scripts, loaded));
        }
    }
}
