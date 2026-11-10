//! Native chooser probe compiled in an isolated Linux session. The companion
//! Test-WaylandDialogs.cjs drives its GTK controls over that session's AT-SPI bus.
use std::fs;
use std::path::Path;

#[path = "../../src/desktop/folder_picker/unix.rs"]
mod chooser;

fn phase(name: &str) {
    fs::write("/evidence/phase", name).unwrap();
    println!("START {name}");
}

fn pass(name: &str) {
    println!("PASS {name}");
}

fn main() -> anyhow::Result<()> {
    anyhow::ensure!(std::env::var("GDK_BACKEND").as_deref() == Ok("wayland"));
    anyhow::ensure!(std::env::var_os("DISPLAY").is_none());
    anyhow::ensure!(!Path::new("/mnt/c").exists());
    let fixture = Path::new("/home/test/Fixtures café");
    let selected_folder = fixture.join("選択 folder");
    fs::create_dir_all(&selected_folder)?;
    let source = fixture.join("résumé 日本語.json");
    fs::write(&source, b"{\"fixture\":true}\n")?;
    fs::write(fixture.join("excluded.txt"), b"Harmless non-JSON fixture\n")?;
    let existing = fixture.join("existing.json");
    let original = b"Original fixture: must survive cancelled overwrite.\n";
    fs::write(&existing, original)?;
    fs::write(fixture.join("collision.json"), original)?;
    let dialog = rfd::FileDialog::new;
    let filter = ("JSON files", "*.json");

    phase("folder-select");
    let path = chooser::choose_folder(dialog(), "", fixture, "Wayland test: folder select")?;
    anyhow::ensure!(
        path.as_deref() == selected_folder.to_str(),
        "Wrong folder: {path:?}"
    );
    pass("folder-select");

    phase("folder-cancel");
    let path = chooser::choose_folder(dialog(), "", fixture, "Wayland test: folder cancel")?;
    anyhow::ensure!(path.is_none());
    pass("folder-cancel");

    phase("open-select");
    let path = chooser::open_file(dialog(), "Wayland test: open select", fixture, filter)?;
    anyhow::ensure!(path.as_ref() == Some(&source), "Wrong file: {path:?}");
    anyhow::ensure!(fs::read(&source)? == b"{\"fixture\":true}\n");
    pass("open-select");

    phase("open-cancel");
    anyhow::ensure!(
        chooser::open_file(dialog(), "Wayland test: open cancel", fixture, filter)?.is_none()
    );
    pass("open-cancel");

    phase("save-new");
    let path = chooser::save_file(
        dialog(),
        "Wayland test: save new",
        fixture,
        "saved résumé",
        filter,
        "json",
    )?;
    let expected = fixture.join("saved résumé.json");
    anyhow::ensure!(
        path.as_ref() == Some(&expected),
        "Wrong save filename: {path:?}"
    );
    fs::write(&expected, b"Created by native dialog probe.\n")?;
    pass("save-new");

    phase("save-cancel");
    anyhow::ensure!(
        chooser::save_file(
            dialog(),
            "Wayland test: save cancel",
            fixture,
            "cancelled.json",
            filter,
            "json"
        )?
        .is_none()
    );
    anyhow::ensure!(!fixture.join("cancelled.json").exists());
    pass("save-cancel");

    phase("overwrite-decline");
    anyhow::ensure!(
        chooser::save_file(
            dialog(),
            "Wayland test: overwrite decline",
            fixture,
            "existing.json",
            filter,
            "json"
        )?
        .is_none()
    );
    anyhow::ensure!(fs::read(&existing)? == original);
    pass("overwrite-decline");

    phase("overwrite-confirm");
    let path = chooser::save_file(
        dialog(),
        "Wayland test: overwrite confirm",
        fixture,
        "existing.json",
        filter,
        "json",
    )?;
    anyhow::ensure!(path.as_ref() == Some(&existing));
    anyhow::ensure!(
        fs::read(&existing)? == original,
        "The chooser modified the file before the caller wrote it"
    );
    fs::write(&existing, b"Replacement explicitly confirmed.\n")?;
    pass("overwrite-confirm");

    phase("extension-collision");
    let result = chooser::save_file(
        dialog(),
        "Wayland test: extension collision",
        fixture,
        "collision",
        filter,
        "json",
    );
    anyhow::ensure!(
        result.is_err(),
        "A missing extension must not silently authorize replacing the suffixed file: {result:?}"
    );
    anyhow::ensure!(fs::read(fixture.join("collision.json"))? == original);
    pass("extension-collision");

    phase("done");
    println!("PASS all native chooser scenarios");
    Ok(())
}
