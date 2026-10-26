//! "Start with Windows" through the per-user Run key; no elevation is needed.
//! The Run value is the only source of truth for the setting. This copy's data
//! folder records which executable it registered, so a moved copy can repair
//! its own entry. Another copy's entry changes only when the user turns
//! startup on here, which makes this copy the one that starts.
use anyhow::{Context, Result, ensure};
use download_engine::{preferences::StartupPreferences, store::Store, tf};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Shared by every copy, so at most one Download Engine starts with Windows.
/// Keeps the pre-2.0 name: older copies and Task Manager's on/off state use this
/// value, and Windows shows the program's own description instead.
pub const VALUE: &str = "XDownloader";
/// Marks a launch by Windows sign-in rather than by the user.
pub const ARGUMENT: &str = "--autostart";

/// One named startup value, so tests never need the real Run key.
pub trait Registry {
    fn command(&self) -> Result<Option<String>>;
    fn set_command(&self, command: &str) -> Result<()>;
    fn remove_command(&self) -> Result<()>;
    /// Task Manager's per-user startup flag: `Some(false)` when turned off there.
    fn approved(&self) -> Result<Option<bool>>;
    fn remove_approval(&self) -> Result<()>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    On,
    /// This copy's entry exists, but Task Manager turned it off.
    Blocked,
    /// The entry starts another Download Engine copy.
    OtherCopy(PathBuf),
    /// The entry starts a program that no longer exists or cannot be read.
    Missing(String),
}

/// The quoted Run command for an executable. Quoting keeps paths with spaces
/// from being split or resolved to a different program.
pub fn command(executable: &Path) -> Result<String> {
    let text = executable
        .to_str()
        .context("This program path cannot be used for Windows startup")?;
    let text = match text.strip_prefix(r"\\?\UNC\") {
        Some(share) => format!(r"\\{share}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(text).to_owned(),
    };
    // Windows file names cannot contain quotes; one here would split the command.
    ensure!(
        Path::new(&text).is_absolute()
            && !text.contains('"')
            && !text.chars().any(char::is_control),
        "This program path cannot be used for Windows startup"
    );
    Ok(format!("\"{text}\" {ARGUMENT}"))
}

/// The program a Run command starts: quoted, or unquoted through `.exe`.
pub fn executable(command: &str) -> Option<PathBuf> {
    let command = command.trim_start();
    let path = if let Some(rest) = command.strip_prefix('"') {
        &rest[..rest.find('"')?]
    } else {
        // ASCII lowercase keeps byte offsets, so `end` is a boundary in `command`.
        let lower = command.to_ascii_lowercase();
        let end = lower
            .match_indices(".exe")
            .map(|(index, _)| index + 4)
            .find(|&end| {
                command[end..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
            })?;
        &command[..end]
    };
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

fn same_path(a: &Path, b: &Path) -> bool {
    fn normalized(path: &Path) -> String {
        let text = path.to_string_lossy().replace('/', "\\");
        text.strip_prefix(r"\\?\")
            .unwrap_or(&text)
            .trim_end_matches('\\')
            .to_lowercase()
    }
    matches!(same_file::is_same_file(a, b), Ok(true)) || normalized(a) == normalized(b)
}

fn points_to(command: Option<&str>, executable: &Path) -> bool {
    command
        .and_then(self::executable)
        .is_some_and(|target| same_path(&target, executable))
}

pub fn state(registry: &dyn Registry, executable: &Path) -> Result<State> {
    let Some(command) = registry.command()? else {
        return Ok(State::Off);
    };
    Ok(match self::executable(&command) {
        Some(target) if same_path(&target, executable) => {
            if registry.approved()? == Some(false) {
                State::Blocked
            } else {
                State::On
            }
        }
        Some(target) if target.is_file() => State::OtherCopy(target),
        _ => State::Missing(command),
    })
}

pub fn enable(registry: &dyn Registry, executable: &Path) -> Result<()> {
    registry.set_command(&command(executable)?)?;
    // Turning startup on here overrides an earlier "turn off" in Task Manager.
    registry.remove_approval()
}

/// Removes only this copy's entry; another copy keeps starting with Windows.
pub fn disable(registry: &dyn Registry, executable: &Path) -> Result<()> {
    if points_to(registry.command()?.as_deref(), executable) {
        registry.remove_command()?;
        registry.remove_approval()?;
    }
    Ok(())
}

/// Launch-time repair of this copy's own entry. Returns whether it was rewritten.
pub fn repair(
    registry: &dyn Registry,
    executable: &Path,
    registered: Option<&Path>,
) -> Result<bool> {
    let Some(current) = registry.command()? else {
        // Removed by the user or an uninstaller: respect that.
        return Ok(false);
    };
    let Some(target) = self::executable(&current) else {
        return Ok(false);
    };
    let expected = command(executable)?;
    let ours = same_path(&target, executable);
    // The entry still names where this copy was registered and that program is
    // gone, so this copy was moved. Entries of other existing copies, and stale
    // entries this copy never registered (such as an unplugged USB drive), stay.
    let moved = registered.is_some_and(|old| same_path(&target, old) && !old.exists());
    if (ours || moved) && current != expected {
        registry.set_command(&expected)?;
        return Ok(true);
    }
    Ok(false)
}

/// A Windows sign-in launch starts hidden only with the tray choice;
/// launching Download Engine yourself always shows the window.
pub fn launch_hidden(autostart: bool, preferences: Result<StartupPreferences>) -> bool {
    autostart && preferences.is_ok_and(|value| value.start_hidden)
}

/// Settings-page view of this copy's startup entry, refreshed from the registry.
pub struct Settings {
    registry: Box<dyn Registry>,
    executable: std::result::Result<PathBuf, String>,
    state: std::result::Result<State, String>,
    start_hidden: bool,
    checked: Instant,
}

impl Settings {
    /// Loads the choices and repairs this copy's entry after a move. Returns an
    /// error message for the notice area when a needed repair failed.
    pub fn load(store: &Store) -> (Self, Option<String>) {
        let (registry, executable) = environment();
        let mut notice = None;
        let preferences = store.startup_preferences().unwrap_or_else(|error| {
            notice = Some(tf!("Cannot check Windows startup: {error}", error = error));
            StartupPreferences::default()
        });
        let mut settings = Self {
            registry,
            executable,
            state: Ok(State::Off),
            start_hidden: preferences.start_hidden,
            checked: Instant::now(),
        };
        if let Ok(executable) = &settings.executable {
            let result = repair(
                settings.registry.as_ref(),
                executable,
                preferences.registered.as_deref(),
            )
            .and_then(|_| {
                // Record this copy once its entry is in place, so a later move can be repaired.
                let ours = matches!(
                    state(settings.registry.as_ref(), executable)?,
                    State::On | State::Blocked
                );
                if ours && preferences.registered.as_deref() != Some(executable.as_path()) {
                    store.save_startup_preferences(&StartupPreferences {
                        registered: Some(executable.clone()),
                        ..preferences.clone()
                    })?;
                }
                Ok(())
            });
            if let Err(error) = result {
                notice = Some(tf!("Cannot change Windows startup: {error}", error = error));
            }
        }
        settings.refresh();
        (settings, notice)
    }

    pub fn refresh(&mut self) {
        self.checked = Instant::now();
        self.state = match &self.executable {
            Ok(executable) => state(self.registry.as_ref(), executable)
                .map_err(|error| tf!("Cannot check Windows startup: {error}", error = error)),
            Err(reason) => Err(reason.clone()),
        };
    }

    /// Keeps the page in step with changes made in Task Manager or another copy.
    pub fn refresh_if_stale(&mut self) {
        if self.checked.elapsed() >= Duration::from_secs(2) {
            self.refresh();
        }
    }

    pub fn state(&self) -> std::result::Result<&State, &str> {
        self.state.as_ref().map_err(String::as_str)
    }

    pub fn enabled(&self) -> bool {
        matches!(self.state, Ok(State::On))
    }

    pub fn start_hidden(&self) -> bool {
        self.start_hidden
    }

    pub fn set_enabled(&mut self, store: &Store, enabled: bool) -> Result<()> {
        let result = (|| {
            let executable = self
                .executable
                .as_ref()
                .map_err(|reason| anyhow::anyhow!("{reason}"))?;
            let mut preferences = store.startup_preferences()?;
            if enabled {
                enable(self.registry.as_ref(), executable)?;
                preferences.registered = Some(executable.clone());
            } else {
                disable(self.registry.as_ref(), executable)?;
                preferences.registered = None;
            }
            store.save_startup_preferences(&preferences)
        })();
        // The page always shows what Windows will actually do, also after a failure.
        self.refresh();
        result
    }

    pub fn set_start_hidden(&mut self, store: &Store, start_hidden: bool) -> Result<()> {
        let mut preferences = store.startup_preferences()?;
        preferences.start_hidden = start_hidden;
        store.save_startup_preferences(&preferences)?;
        self.start_hidden = start_hidden;
        Ok(())
    }
}

type Environment = (Box<dyn Registry>, std::result::Result<PathBuf, String>);

fn current_executable() -> std::result::Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| tf!("Cannot check Windows startup: {error}", error = error))
}

// Unit tests never reach the real Run key.
#[cfg(test)]
fn environment() -> Environment {
    (Box::new(Memory::default()), current_executable())
}

#[cfg(not(test))]
fn environment() -> Environment {
    #[cfg(feature = "screenshot")]
    if std::env::var_os("IDM_EX_SCREENSHOT_TO").is_some() {
        let executable = current_executable();
        return (
            Box::new(Memory::preview(executable.as_deref().ok())),
            executable,
        );
    }
    // A Run entry cannot carry the profile variable, so it would open a different profile.
    // This also keeps development and smoke-test profiles away from the Run key.
    if download_engine::paths::home_override().is_some() {
        return (
            Box::new(Unavailable),
            Err(download_engine::i18n::tr("Unavailable while DOWNLOAD_ENGINE_HOME is set.").into()),
        );
    }
    #[cfg(windows)]
    let registry: Box<dyn Registry> = Box::new(WindowsRun::current_user());
    #[cfg(not(windows))]
    let registry: Box<dyn Registry> = Box::new(Unavailable);
    (registry, current_executable())
}

#[cfg(not(test))]
struct Unavailable;
#[cfg(not(test))]
impl Registry for Unavailable {
    fn command(&self) -> Result<Option<String>> {
        anyhow::bail!("Windows startup is unavailable")
    }
    fn set_command(&self, _: &str) -> Result<()> {
        self.command().map(drop)
    }
    fn remove_command(&self) -> Result<()> {
        self.command().map(drop)
    }
    fn approved(&self) -> Result<Option<bool>> {
        self.command().map(|_| None)
    }
    fn remove_approval(&self) -> Result<()> {
        self.command().map(drop)
    }
}

/// In-memory startup values for unit tests and screenshot fixtures.
#[cfg(any(test, feature = "screenshot"))]
#[derive(Default)]
pub struct Memory {
    command: std::cell::RefCell<Option<String>>,
    approval: std::cell::Cell<Option<bool>>,
}
#[cfg(all(not(test), feature = "screenshot"))]
impl Memory {
    fn preview(executable: Option<&Path>) -> Self {
        let memory = Self::default();
        // "Another copy" must name a program that exists; the CLI beside this build does.
        let other = executable
            .map(|path| path.with_file_name("de-cli.exe"))
            .unwrap_or_default();
        let other = other.display();
        let own = executable.and_then(|path| command(path).ok());
        match std::env::var("IDM_EX_TEST_STARTUP").as_deref() {
            Ok("on") => *memory.command.borrow_mut() = own,
            Ok("blocked") => {
                *memory.command.borrow_mut() = own;
                memory.approval.set(Some(false));
            }
            Ok("other") => *memory.command.borrow_mut() = Some(format!("\"{other}\" {ARGUMENT}")),
            Ok("missing") => {
                *memory.command.borrow_mut() =
                    Some(format!(r#""E:\Download Engine\de.exe" {ARGUMENT}"#))
            }
            _ => {}
        }
        memory
    }
}
#[cfg(any(test, feature = "screenshot"))]
impl Registry for Memory {
    fn command(&self) -> Result<Option<String>> {
        Ok(self.command.borrow().clone())
    }
    fn set_command(&self, command: &str) -> Result<()> {
        *self.command.borrow_mut() = Some(command.into());
        Ok(())
    }
    fn remove_command(&self) -> Result<()> {
        *self.command.borrow_mut() = None;
        Ok(())
    }
    fn approved(&self) -> Result<Option<bool>> {
        Ok(self.approval.get())
    }
    fn remove_approval(&self) -> Result<()> {
        self.approval.set(None);
        Ok(())
    }
}

/// HKEY_CURRENT_USER values. Tests use an isolated key, never the real Run key.
#[cfg(windows)]
pub struct WindowsRun {
    run: String,
    approved: String,
}
#[cfg(windows)]
impl WindowsRun {
    #[cfg(not(test))]
    fn current_user() -> Self {
        Self {
            run: r"Software\Microsoft\Windows\CurrentVersion\Run".into(),
            approved: r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
                .into(),
        }
    }
}
#[cfg(windows)]
impl Registry for WindowsRun {
    fn command(&self) -> Result<Option<String>> {
        use windows_sys::Win32::System::Registry::{
            RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
        };
        // Strings are read unexpanded: only our own quoted REG_SZ command can match.
        let Some(data) = native::read(
            &self.run,
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
        )?
        else {
            return Ok(None);
        };
        let mut wide: Vec<u16> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        while wide.last() == Some(&0) {
            wide.pop();
        }
        // An unreadable value is reported as a missing program, which "on" replaces.
        Ok(Some(String::from_utf16(&wide).unwrap_or_default()))
    }
    fn set_command(&self, command: &str) -> Result<()> {
        native::write(&self.run, command)
    }
    fn remove_command(&self) -> Result<()> {
        native::delete(&self.run)
    }
    fn approved(&self) -> Result<Option<bool>> {
        use windows_sys::Win32::System::Registry::RRF_RT_REG_BINARY;
        // Task Manager stores an even first byte for enabled and an odd one for disabled.
        Ok(native::read(&self.approved, RRF_RT_REG_BINARY)?
            .map(|data| data.first().is_none_or(|flag| flag & 1 == 0)))
    }
    fn remove_approval(&self) -> Result<()> {
        native::delete(&self.approved)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
            ERROR_UNSUPPORTED_TYPE,
        },
        System::Registry::*,
    };
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn check(code: u32) -> Result<()> {
        ensure!(
            code == ERROR_SUCCESS,
            std::io::Error::from_raw_os_error(code as i32)
        );
        Ok(())
    }
    fn missing(code: u32) -> bool {
        code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND
    }
    pub fn read(key: &str, flags: u32) -> Result<Option<Vec<u8>>> {
        let (key, name) = (wide(key), wide(VALUE));
        // A concurrent writer can grow the value between the size query and the read.
        for _ in 0..4 {
            let mut length = 0u32;
            // Null data asks only for the size; both strings are live and terminated.
            let code = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    flags,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut length,
                )
            };
            if missing(code) {
                return Ok(None);
            }
            if code == ERROR_UNSUPPORTED_TYPE {
                return Ok(Some(Vec::new()));
            }
            check(code)?;
            ensure!(length <= 65536, "The Windows startup value is too large");
            let mut data = vec![0u8; length as usize];
            // The writable buffer holds exactly `length` bytes.
            let code = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    flags,
                    std::ptr::null_mut(),
                    data.as_mut_ptr().cast(),
                    &mut length,
                )
            };
            if code == ERROR_MORE_DATA {
                continue;
            }
            if missing(code) {
                return Ok(None);
            }
            check(code)?;
            data.truncate(length as usize);
            return Ok(Some(data));
        }
        anyhow::bail!("The Windows startup value keeps changing")
    }
    pub fn write(key: &str, value: &str) -> Result<()> {
        let (key, name, value) = (wide(key), wide(VALUE), wide(value));
        let mut handle = std::ptr::null_mut();
        check(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        })?;
        // The handle was opened above and is closed on every path below.
        let code = unsafe {
            RegSetValueExW(
                handle,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        unsafe { RegCloseKey(handle) };
        check(code)
    }
    pub fn delete(key: &str) -> Result<()> {
        let (key, name) = (wide(key), wide(VALUE));
        let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        if missing(code) {
            return Ok(());
        }
        check(code)
    }
    #[cfg(test)]
    pub fn cleanup(key: &str) {
        let key = wide(key);
        // Only the caller's freshly generated test namespace is recursively removed.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use download_engine::paths::AppPaths;

    fn store() -> (tempfile::TempDir, Store) {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(AppPaths {
            data: root.path().join("data"),
            downloads: root.path().join("files"),
        })
        .unwrap();
        (root, store)
    }
    fn exe(root: &Path, folder: &str) -> PathBuf {
        let path = root.join(folder).join("de.exe");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"fixture").unwrap();
        path
    }

    #[test]
    fn commands_are_quoted_absolute_and_parse_back_to_the_same_program() {
        assert_eq!(
            command(Path::new(r"C:\Program Files\X Downloader\de.exe")).unwrap(),
            r#""C:\Program Files\X Downloader\de.exe" --autostart"#
        );
        assert_eq!(
            command(Path::new(r"\\?\D:\Portable apps\de.exe")).unwrap(),
            r#""D:\Portable apps\de.exe" --autostart"#
        );
        assert_eq!(
            command(Path::new(r"\\?\UNC\server\share\de.exe")).unwrap(),
            r#""\\server\share\de.exe" --autostart"#
        );
        for invalid in [
            "de.exe",
            r"relative\de.exe",
            "C:\\a\"b\\de.exe",
            "C:\\a\nb\\de.exe",
        ] {
            assert!(command(Path::new(invalid)).is_err(), "{invalid}");
        }
        let program = Path::new(r"C:\Program Files\X Downloader\de.exe");
        assert_eq!(
            executable(&command(program).unwrap()).as_deref(),
            Some(program)
        );
        assert_eq!(
            executable(r"C:\Program Files\X Downloader\de.exe --autostart").as_deref(),
            Some(program)
        );
        assert_eq!(
            executable(r"C:\Tools\DE.EXE").as_deref(),
            Some(Path::new(r"C:\Tools\DE.EXE"))
        );
        assert_eq!(
            executable(r"C:\a.exed\de.exe -x").as_deref(),
            Some(Path::new(r"C:\a.exed\de.exe"))
        );
        for invalid in [
            "",
            "\"C:\\unterminated",
            "\"relative.exe\"",
            "garbage",
            "C:\\no-program",
        ] {
            assert_eq!(executable(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn enabling_and_disabling_change_only_this_copys_entry() {
        let root = tempfile::tempdir().unwrap();
        let (this, other) = (exe(root.path(), "this copy"), exe(root.path(), "other"));
        let registry = Memory::default();
        assert_eq!(state(&registry, &this).unwrap(), State::Off);
        enable(&registry, &this).unwrap();
        assert_eq!(registry.command().unwrap(), Some(command(&this).unwrap()));
        assert_eq!(state(&registry, &this).unwrap(), State::On);
        disable(&registry, &this).unwrap();
        assert_eq!(registry.command().unwrap(), None);
        assert_eq!(state(&registry, &this).unwrap(), State::Off);
        // Another copy's entry is reported and kept when this copy is turned off.
        enable(&registry, &other).unwrap();
        assert_eq!(
            state(&registry, &this).unwrap(),
            State::OtherCopy(other.clone())
        );
        disable(&registry, &this).unwrap();
        assert_eq!(registry.command().unwrap(), Some(command(&other).unwrap()));
        // Turning on here switches startup to this copy.
        enable(&registry, &this).unwrap();
        assert_eq!(
            state(&registry, &other).unwrap(),
            State::OtherCopy(this.clone())
        );
        assert_eq!(state(&registry, &this).unwrap(), State::On);
        // Paths are compared as Windows does: case and verbatim prefixes do not matter.
        let upper = PathBuf::from(this.to_string_lossy().to_uppercase());
        assert_eq!(state(&registry, &upper).unwrap(), State::On);
        registry
            .set_command(&format!("\"\\\\?\\{}\" {ARGUMENT}", this.display()))
            .unwrap();
        assert_eq!(state(&registry, &this).unwrap(), State::On);
    }

    #[test]
    fn task_manager_and_missing_programs_are_reported_and_turning_on_overrides_them() {
        let root = tempfile::tempdir().unwrap();
        let this = exe(root.path(), "this");
        let registry = Memory::default();
        enable(&registry, &this).unwrap();
        registry.approval.set(Some(false));
        assert_eq!(state(&registry, &this).unwrap(), State::Blocked);
        registry.approval.set(Some(true));
        assert_eq!(state(&registry, &this).unwrap(), State::On);
        registry.approval.set(Some(false));
        enable(&registry, &this).unwrap();
        assert_eq!(registry.approved().unwrap(), None);
        assert_eq!(state(&registry, &this).unwrap(), State::On);
        // Turning off also removes the Task Manager flag left for this copy.
        registry.approval.set(Some(false));
        disable(&registry, &this).unwrap();
        assert_eq!(
            (registry.command().unwrap(), registry.approved().unwrap()),
            (None, None)
        );
        let gone = format!(
            r#""{}" {ARGUMENT}"#,
            root.path().join("gone").join("de.exe").display()
        );
        for stale in [gone.as_str(), "garbage", ""] {
            registry.set_command(stale).unwrap();
            assert_eq!(
                state(&registry, &this).unwrap(),
                State::Missing(stale.into())
            );
            disable(&registry, &this).unwrap();
            assert_eq!(registry.command().unwrap().as_deref(), Some(stale));
        }
        enable(&registry, &this).unwrap();
        assert_eq!(state(&registry, &this).unwrap(), State::On);
    }

    #[test]
    fn launch_repairs_a_moved_copy_and_quotes_its_entry_but_leaves_others_alone() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old place").join("de.exe");
        let moved = exe(root.path(), "new place");
        let registry = Memory::default();
        // The registered program is gone and the entry still names it: rewrite.
        registry.set_command(&command(&old).unwrap()).unwrap();
        assert!(repair(&registry, &moved, Some(&old)).unwrap());
        assert_eq!(registry.command().unwrap(), Some(command(&moved).unwrap()));
        assert!(!repair(&registry, &moved, Some(&old)).unwrap());
        // An unquoted entry for this copy is rewritten quoted.
        registry.set_command(&moved.display().to_string()).unwrap();
        assert!(repair(&registry, &moved, None).unwrap());
        assert_eq!(registry.command().unwrap(), Some(command(&moved).unwrap()));
        // A copy that still exists keeps its entry, even if this one registered it before.
        let original = exe(root.path(), "original");
        registry.set_command(&command(&original).unwrap()).unwrap();
        assert!(!repair(&registry, &moved, Some(&original)).unwrap());
        assert_eq!(
            registry.command().unwrap(),
            Some(command(&original).unwrap())
        );
        // A stale entry this copy never registered, such as an unplugged drive, stays.
        registry.set_command(&command(&old).unwrap()).unwrap();
        for registered in [None, Some(original.as_path())] {
            assert!(!repair(&registry, &moved, registered).unwrap());
            assert_eq!(registry.command().unwrap(), Some(command(&old).unwrap()));
        }
        // An entry the user removed is not added back.
        registry.remove_command().unwrap();
        assert!(!repair(&registry, &moved, Some(&old)).unwrap());
        assert_eq!(registry.command().unwrap(), None);
    }

    #[test]
    fn only_windows_launches_with_the_tray_choice_start_hidden() {
        let tray = StartupPreferences::default();
        let window = StartupPreferences {
            start_hidden: false,
            ..Default::default()
        };
        assert!(launch_hidden(true, Ok(tray.clone())));
        assert!(!launch_hidden(false, Ok(tray)));
        assert!(!launch_hidden(true, Ok(window)));
        assert!(!launch_hidden(
            true,
            Err(anyhow::anyhow!("unreadable profile"))
        ));
    }

    #[test]
    fn settings_reflect_the_entry_record_moves_and_keep_the_tray_choice() {
        let (root, store) = store();
        let this = std::env::current_exe().unwrap();
        let (mut settings, notice) = Settings::load(&store);
        assert_eq!(notice, None);
        assert_eq!(settings.state(), Ok(&State::Off));
        assert!(settings.start_hidden());
        settings.set_enabled(&store, true).unwrap();
        assert!(settings.enabled());
        assert_eq!(
            store.startup_preferences().unwrap().registered.as_deref(),
            Some(this.as_path())
        );
        settings.set_start_hidden(&store, false).unwrap();
        settings.set_enabled(&store, false).unwrap();
        assert!(!settings.enabled());
        assert_eq!(
            store.startup_preferences().unwrap(),
            StartupPreferences {
                start_hidden: false,
                registered: None
            }
        );
        // External changes, such as Task Manager, appear after a refresh.
        let other = exe(root.path(), "other");
        settings
            .registry
            .set_command(&command(&other).unwrap())
            .unwrap();
        settings.refresh();
        assert_eq!(settings.state(), Ok(&State::OtherCopy(other)));
        settings.set_enabled(&store, true).unwrap();
        assert_eq!(settings.state(), Ok(&State::On));
    }

    #[cfg(windows)]
    #[test]
    fn isolated_registry_key_round_trips_values_and_is_removed() {
        struct Key(String);
        impl Drop for Key {
            fn drop(&mut self) {
                native::cleanup(&self.0);
            }
        }
        let key = Key(format!(
            r"Software\IDM_EX\Tests\startup-{}",
            uuid::Uuid::new_v4()
        ));
        let registry = WindowsRun {
            run: format!(r"{}\Run", key.0),
            approved: format!(r"{}\StartupApproved", key.0),
        };
        assert_eq!(registry.command().unwrap(), None);
        assert_eq!(registry.approved().unwrap(), None);
        registry.remove_command().unwrap();
        registry.remove_approval().unwrap();
        let program = Path::new(r"C:\Program Files\Download Engine Test\de.exe");
        enable(&registry, program).unwrap();
        assert_eq!(registry.command().unwrap(), Some(command(program).unwrap()));
        assert_eq!(state(&registry, program).unwrap(), State::On);
        let approved = wide_key(&registry.approved);
        for (flag, expected) in [
            (2u8, State::On),
            (3, State::Blocked),
            (6, State::On),
            (7, State::Blocked),
        ] {
            approved.set_binary(&[flag, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            assert_eq!(state(&registry, program).unwrap(), expected, "flag {flag}");
        }
        enable(&registry, program).unwrap();
        assert_eq!(registry.approved().unwrap(), None);
        disable(&registry, program).unwrap();
        assert_eq!(registry.command().unwrap(), None);
        let root = wide_key(&key.0);
        drop(key);
        assert!(!root.exists(), "The isolated test key must be removed");
    }

    #[cfg(windows)]
    struct BinaryKey(Vec<u16>);
    #[cfg(windows)]
    fn wide_key(key: &str) -> BinaryKey {
        BinaryKey(key.encode_utf16().chain(Some(0)).collect())
    }
    #[cfg(windows)]
    impl BinaryKey {
        fn set_binary(&self, data: &[u8]) {
            use windows_sys::Win32::System::Registry::{
                HKEY_CURRENT_USER, REG_BINARY, RegSetKeyValueW,
            };
            let name: Vec<u16> = VALUE.encode_utf16().chain(Some(0)).collect();
            let code = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    self.0.as_ptr(),
                    name.as_ptr(),
                    REG_BINARY,
                    data.as_ptr().cast(),
                    data.len() as u32,
                )
            };
            assert_eq!(code, 0);
        }
        fn exists(&self) -> bool {
            use windows_sys::Win32::System::Registry::{
                HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW,
            };
            let mut handle = std::ptr::null_mut();
            let code = unsafe {
                RegOpenKeyExW(HKEY_CURRENT_USER, self.0.as_ptr(), 0, KEY_READ, &mut handle)
            };
            if code == 0 {
                unsafe { RegCloseKey(handle) };
            }
            code == 0
        }
    }
}
