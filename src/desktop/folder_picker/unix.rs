//! Unix chooser calls kept independent of rendering so native portal tests use
//! exactly the same path, filter, cancellation and overwrite handling as the app.
use std::path::{Path, PathBuf};

pub(super) fn choose_folder(
    dialog: rfd::FileDialog,
    value: &str,
    default: &Path,
    title: &str,
) -> anyhow::Result<Option<String>> {
    let initial = if !value.trim().is_empty() && Path::new(value).is_dir() {
        Path::new(value)
    } else {
        default
    };
    Ok(dialog
        .set_title(title)
        .set_directory(initial)
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned()))
}

pub(super) fn save_file(
    dialog: rfd::FileDialog,
    title: &str,
    folder: &Path,
    name: &str,
    filter: (&str, &str),
    extension: &str,
) -> anyhow::Result<Option<PathBuf>> {
    let result = file_dialog(dialog, title, folder, filter)
        .set_file_name(name)
        .save_file();
    if let Some(mut path) = result {
        if path.extension().is_none() && !extension.is_empty() {
            path.set_extension(extension);
            anyhow::ensure!(
                !path.exists(),
                "This file already exists. Choose its full filename to confirm replacement."
            );
        }
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

pub(super) fn open_file(
    dialog: rfd::FileDialog,
    title: &str,
    folder: &Path,
    filter: (&str, &str),
) -> anyhow::Result<Option<PathBuf>> {
    Ok(file_dialog(dialog, title, folder, filter).pick_file())
}

fn file_dialog(
    dialog: rfd::FileDialog,
    title: &str,
    folder: &Path,
    filter: (&str, &str),
) -> rfd::FileDialog {
    let dialog = dialog.set_title(title).set_directory(folder);
    let extensions: Vec<_> = filter
        .1
        .split(';')
        .filter_map(|pattern| pattern.trim().strip_prefix("*."))
        .filter(|extension| !extension.contains('*'))
        .collect();
    if extensions.is_empty() {
        dialog
    } else {
        dialog.add_filter(filter.0, &extensions)
    }
}
