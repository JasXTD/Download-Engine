//! Native folder and list-file selection. Cancelling keeps the draft untouched.
use super::*;
use std::path::Path;

#[derive(Default)]
pub(super) struct FolderPicker {
    #[cfg(windows)]
    owner: Option<isize>,
    #[cfg(test)]
    pub response: Option<anyhow::Result<Option<String>>>,
}
impl FolderPicker {
    pub fn set_owner(&mut self, frame: &eframe::Frame) {
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            self.owner = frame
                .window_handle()
                .ok()
                .and_then(|handle| match handle.as_raw() {
                    RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                    _ => None,
                });
        }
        #[cfg(not(windows))]
        let _ = frame;
    }
    pub fn field(
        &mut self,
        ui: &mut egui::Ui,
        value: &mut String,
        default: &Path,
        error: &mut Option<String>,
    ) -> egui::Response {
        ui.horizontal(|ui| {
            let height = ui.text_style_height(&egui::TextStyle::Button)
                + ui.spacing().button_padding.y * 2.0;
            // Leave exactly the translated Browse button's width so the row ends at the edge.
            let browse = super::view::button_width(ui, tr("Browse…"));
            let input = ui.add(
                view::line_edit(value)
                    .hint_text(tr("Automatic folder — leave blank"))
                    .char_limit(4096)
                    .min_size(egui::vec2(0.0, height))
                    .vertical_align(egui::Align::Center)
                    .desired_width(
                        (ui.available_width() - ui.spacing().item_spacing.x - browse).max(100.0),
                    ),
            );
            if ui
                .add_enabled(cfg!(windows), egui::Button::new(tr("Browse…")))
                .on_hover_text(tr("Choose a folder"))
                .clicked()
            {
                match self.choose(value, default) {
                    Ok(Some(path)) => {
                        *value = path;
                        *error = None;
                    }
                    Ok(None) => {}
                    Err(failure) => {
                        *error = Some(tf!("Cannot choose folder: {failure}", failure = failure))
                    }
                }
            }
            input
        })
        .inner
    }
    fn choose(&mut self, value: &str, default: &Path) -> anyhow::Result<Option<String>> {
        self.choose_titled(value, default, tr("Select download folder"))
    }
    pub fn choose_titled(
        &mut self,
        value: &str,
        default: &Path,
        title: &str,
    ) -> anyhow::Result<Option<String>> {
        #[cfg(test)]
        if let Some(response) = self.response.take() {
            return response;
        }
        #[cfg(windows)]
        {
            let owner = self.owner.ok_or_else(|| {
                anyhow::anyhow!(tr(
                    "The download window is unavailable. Enter the folder path instead."
                ))
            })?;
            native::choose(owner, value, default, title)
        }
        #[cfg(not(windows))]
        {
            let _ = (value, default, title);
            anyhow::bail!(tr("Enter the folder path manually on this platform."))
        }
    }
    /// Choose where to save a file. Windows asks before an existing file is chosen.
    pub fn save_file(
        &mut self,
        title: &str,
        folder: &Path,
        name: &str,
        filter: (&str, &str),
        extension: &str,
    ) -> anyhow::Result<Option<PathBuf>> {
        #[cfg(test)]
        if let Some(response) = self.response.take() {
            return response.map(|path| path.map(PathBuf::from));
        }
        #[cfg(windows)]
        {
            native::save_file(self.file_owner()?, title, folder, name, filter, extension)
                .map(|path| path.map(PathBuf::from))
        }
        #[cfg(not(windows))]
        {
            let _ = (title, folder, name, filter, extension);
            anyhow::bail!(tr("Choosing files is not available on this platform."))
        }
    }
    pub fn open_file(
        &mut self,
        title: &str,
        folder: &Path,
        filter: (&str, &str),
    ) -> anyhow::Result<Option<PathBuf>> {
        #[cfg(test)]
        if let Some(response) = self.response.take() {
            return response.map(|path| path.map(PathBuf::from));
        }
        #[cfg(windows)]
        {
            native::open_file(self.file_owner()?, title, folder, filter)
                .map(|path| path.map(PathBuf::from))
        }
        #[cfg(not(windows))]
        {
            let _ = (title, folder, filter);
            anyhow::bail!(tr("Choosing files is not available on this platform."))
        }
    }
    #[cfg(windows)]
    fn file_owner(&self) -> anyhow::Result<isize> {
        self.owner.ok_or_else(|| {
            anyhow::anyhow!(tr(
                "The Download Engine window is unavailable. Try again in a moment."
            ))
        })
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        Win32::{
            Foundation::{ERROR_CANCELLED, HWND},
            System::Com::*,
            UI::Shell::*,
        },
        core::{HSTRING, PCWSTR, PWSTR},
    };

    struct Apartment;
    impl Apartment {
        fn new() -> windows::core::Result<Self> {
            // Balance both S_OK and S_FALSE. This chooser runs on the owning UI thread.
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
            }
            Ok(Self)
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    struct ShellPath(PWSTR);
    impl Drop for ShellPath {
        fn drop(&mut self) {
            unsafe {
                CoTaskMemFree(Some(self.0.as_ptr().cast()));
            }
        }
    }

    fn dialog() -> windows::core::Result<IFileOpenDialog> {
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(
                dialog.GetOptions()?
                    | FOS_PICKFOLDERS
                    | FOS_FORCEFILESYSTEM
                    | FOS_PATHMUSTEXIST
                    | FOS_NOCHANGEDIR,
            )?;
            dialog.SetTitle(&HSTRING::from(tr("Select download folder")))?;
            dialog.SetOkButtonLabel(&HSTRING::from(tr("Select folder")))?;
            Ok(dialog)
        }
    }
    fn shell_item(path: &Path) -> windows::core::Result<IShellItem> {
        let value = HSTRING::from(path.as_os_str());
        unsafe { SHCreateItemFromParsingName(PCWSTR(value.as_ptr()), None) }
    }
    fn path(item: &IShellItem) -> anyhow::Result<String> {
        let value = ShellPath(unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? });
        Ok(unsafe { value.0.to_string()? })
    }
    pub(super) fn choose(
        owner: isize,
        value: &str,
        default: &Path,
        title: &str,
    ) -> anyhow::Result<Option<String>> {
        let _apartment = Apartment::new()?;
        let dialog = dialog()?;
        let entered = Path::new(value.trim());
        unsafe {
            dialog.SetTitle(&HSTRING::from(title))?;
            if !value.trim().is_empty() && entered.is_dir() {
                dialog.SetFolder(&shell_item(entered)?)?;
            } else if let Some(initial) = default.ancestors().find(|p| p.is_dir()) {
                dialog.SetDefaultFolder(&shell_item(initial)?)?;
            }
            match dialog.Show(Some(HWND(owner as *mut std::ffi::c_void))) {
                Ok(()) => Ok(Some(path(&dialog.GetResult()?)?)),
                Err(error)
                    if error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0) =>
                {
                    Ok(None)
                }
                Err(error) => Err(error.into()),
            }
        }
    }

    fn save_dialog() -> windows::core::Result<IFileSaveDialog> {
        unsafe {
            let dialog: IFileSaveDialog =
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
            // Windows asks before an existing file is chosen; the extension
            // always matches the single file type offered.
            dialog.SetOptions(
                dialog.GetOptions()?
                    | FOS_OVERWRITEPROMPT
                    | FOS_STRICTFILETYPES
                    | FOS_FORCEFILESYSTEM
                    | FOS_PATHMUSTEXIST
                    | FOS_NOCHANGEDIR,
            )?;
            Ok(dialog)
        }
    }
    fn open_dialog() -> windows::core::Result<IFileOpenDialog> {
        unsafe {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(
                dialog.GetOptions()?
                    | FOS_FORCEFILESYSTEM
                    | FOS_FILEMUSTEXIST
                    | FOS_PATHMUSTEXIST
                    | FOS_NOCHANGEDIR,
            )?;
            Ok(dialog)
        }
    }
    fn show_file(
        dialog: &IFileDialog,
        owner: isize,
        title: &str,
        folder: &Path,
        (name, spec): (&str, &str),
    ) -> anyhow::Result<Option<String>> {
        let (name, spec) = (HSTRING::from(name), HSTRING::from(spec));
        unsafe {
            dialog.SetTitle(&HSTRING::from(title))?;
            dialog.SetFileTypes(&[Common::COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(spec.as_ptr()),
            }])?;
            if let Some(initial) = folder.ancestors().find(|p| p.is_dir()) {
                dialog.SetDefaultFolder(&shell_item(initial)?)?;
            }
            match dialog.Show(Some(HWND(owner as *mut std::ffi::c_void))) {
                Ok(()) => Ok(Some(path(&dialog.GetResult()?)?)),
                Err(error)
                    if error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0) =>
                {
                    Ok(None)
                }
                Err(error) => Err(error.into()),
            }
        }
    }
    pub(super) fn save_file(
        owner: isize,
        title: &str,
        folder: &Path,
        name: &str,
        filter: (&str, &str),
        extension: &str,
    ) -> anyhow::Result<Option<String>> {
        let _apartment = Apartment::new()?;
        let dialog = save_dialog()?;
        unsafe {
            dialog.SetFileName(&HSTRING::from(name))?;
            dialog.SetDefaultExtension(&HSTRING::from(extension))?;
        }
        show_file(&dialog, owner, title, folder, filter)
    }
    pub(super) fn open_file(
        owner: isize,
        title: &str,
        folder: &Path,
        filter: (&str, &str),
    ) -> anyhow::Result<Option<String>> {
        let _apartment = Apartment::new()?;
        let dialog = open_dialog()?;
        show_file(&dialog, owner, title, folder, filter)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn native_folder_dialog_options_and_unicode_paths() {
            let _apartment = Apartment::new().unwrap();
            let dialog = dialog().unwrap();
            let options = unsafe { dialog.GetOptions().unwrap() };
            for flag in [
                FOS_PICKFOLDERS,
                FOS_FORCEFILESYSTEM,
                FOS_PATHMUSTEXIST,
                FOS_NOCHANGEDIR,
            ] {
                assert_eq!(options.0 & flag.0, flag.0);
            }
            let temp = tempfile::tempdir().unwrap();
            let folder = temp.path().join("下載 résumé");
            std::fs::create_dir(&folder).unwrap();
            let item = shell_item(&folder).unwrap();
            assert_eq!(Path::new(&path(&item).unwrap()), folder);
            unsafe {
                dialog.SetFolder(&item).unwrap();
            }
        }
        #[test]
        fn native_file_dialogs_confirm_replacing_and_require_existing_files() {
            let _apartment = Apartment::new().unwrap();
            let save = unsafe { save_dialog().unwrap().GetOptions().unwrap() };
            for flag in [
                FOS_OVERWRITEPROMPT,
                FOS_STRICTFILETYPES,
                FOS_FORCEFILESYSTEM,
                FOS_PATHMUSTEXIST,
                FOS_NOCHANGEDIR,
            ] {
                assert_eq!(save.0 & flag.0, flag.0);
            }
            let open = unsafe { open_dialog().unwrap().GetOptions().unwrap() };
            for flag in [FOS_FORCEFILESYSTEM, FOS_FILEMUSTEXIST, FOS_PATHMUSTEXIST] {
                assert_eq!(open.0 & flag.0, flag.0);
            }
            assert_eq!(open.0 & FOS_PICKFOLDERS.0, 0);
        }
    }
}
