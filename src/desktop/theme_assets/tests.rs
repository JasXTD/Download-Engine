use super::*;
use crate::desktop::interaction_tests::{click, fixture, frame};

#[test]
fn all_embedded_presets_decode_with_bounded_dimensions() {
    for preset in ThemePreset::BUILT_IN {
        let image = load(Key::Preset(preset), Path::new("unused"))
            .unwrap()
            .image;
        assert!(image.width() >= 1000 && image.width() <= 2048);
        assert!(image.height() >= 500 && image.height() <= 2048);
    }
}

fn sample(path: &Path, format: image::ImageFormat) {
    image::RgbImage::from_pixel(32, 16, image::Rgb([195, 165, 255]))
        .save_with_format(path, format)
        .unwrap();
}

#[test]
fn import_copies_png_and_jpeg_deduplicates_and_survives_original_removal() {
    for (format, name) in [
        (image::ImageFormat::Png, "source.png"),
        (image::ImageFormat::Jpeg, "source.jpg"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(name);
        sample(&path, format);
        let profile = root.path().join("profile");
        let first = import(&path, &profile).unwrap();
        assert!(first.created.is_some());
        let second = import(&path, &profile).unwrap();
        assert_eq!(first.id, second.id);
        assert!(second.created.is_none());
        std::fs::remove_file(path).unwrap();
        let loaded = load(Key::Image(first.id), &profile).unwrap();
        assert_eq!(loaded.image.size, [32, 16]);
        std::fs::write(image_path(&profile, first.id), b"changed").unwrap();
        assert!(load(Key::Image(first.id), &profile).is_err());
    }
}

#[test]
fn import_rejects_invalid_oversized_and_non_image_input_without_leaving_files() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let source = root.path().join("invalid.png");
    std::fs::write(&source, b"not an image").unwrap();
    assert!(import(&source, &profile).is_err());
    let file = File::create(&source).unwrap();
    file.set_len(MAX_BYTES + 1).unwrap();
    assert!(import(&source, &profile).is_err());
    drop(file);
    // A highly compressible PNG exercises the dimension limit before pixel decode.
    image::RgbImage::new(4096, 4096).save(&source).unwrap();
    assert!(
        import(&source, &profile)
            .unwrap_err()
            .to_string()
            .contains("16 megapixels")
    );
    assert!(!profile.exists());
}

fn wait_import(app: &mut Desktop, ctx: &egui::Context) {
    let until = Instant::now() + Duration::from_secs(10);
    while app.theme_assets.importing.is_some() {
        assert!(Instant::now() < until);
        app.poll_theme_assets(ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn cancelled_and_stale_imports_do_not_change_new_drafts_or_leave_owned_files() {
    let (root, mut app, ctx) = fixture();
    let source = root.path().join("source.png");
    sample(&source, image::ImageFormat::Png);
    app.open_preferences_window();
    let session = app.preferences_window.as_ref().unwrap().session;
    app.theme_assets
        .start_import(&ctx, session, source.clone(), app.store.paths.data.clone())
        .unwrap();
    app.close_preferences_window(&ctx);
    app.open_preferences_window();
    wait_import(&mut app, &ctx);
    assert_eq!(
        app.preferences_window
            .as_ref()
            .unwrap()
            .interface
            .custom_theme
            .image,
        None
    );
    assert_eq!(
        std::fs::read_dir(app.store.paths.data.join("themes"))
            .unwrap()
            .count(),
        0
    );
    let session = app.preferences_window.as_ref().unwrap().session;
    app.theme_assets
        .start_import(&ctx, session, source.clone(), app.store.paths.data.clone())
        .unwrap();
    wait_import(&mut app, &ctx);
    let id = app
        .preferences_window
        .as_ref()
        .unwrap()
        .interface
        .custom_theme
        .image
        .unwrap();
    assert!(image_path(&app.store.paths.data, id).exists());
    app.close_preferences_window(&ctx);
    assert!(!image_path(&app.store.paths.data, id).exists());
    assert!(source.exists());
}

#[test]
fn saved_import_survives_cancel_of_a_later_preview() {
    let (root, mut app, ctx) = fixture();
    let source = root.path().join("source.jpg");
    sample(&source, image::ImageFormat::Jpeg);
    app.open_preferences_window();
    let session = app.preferences_window.as_ref().unwrap().session;
    app.theme_assets
        .start_import(&ctx, session, source, app.store.paths.data.clone())
        .unwrap();
    wait_import(&mut app, &ctx);
    app.preferences_window
        .as_mut()
        .unwrap()
        .interface
        .appearance = Appearance::Custom;
    app.save_preferences_window(&ctx);
    let saved = app.store.interface_preferences().unwrap();
    assert_eq!(saved.appearance, Appearance::Custom);
    let id = saved.custom_theme.image.unwrap();
    app.open_preferences_window();
    app.preferences_window
        .as_mut()
        .unwrap()
        .interface
        .custom_theme
        .preset = ThemePreset::Aurora;
    app.close_preferences_window(&ctx);
    assert!(load(Key::Image(id), &app.store.paths.data).is_ok());
    assert_eq!(app.interface, saved);
}

#[test]
fn custom_preview_cancel_save_reset_and_switch_back_restore_palette() {
    let (_root, mut app, ctx) = fixture();
    let original = app.interface;
    app.open_preferences_window();
    click(&mut app, &ctx, "Custom");
    assert!(theme::custom(&ctx).is_some());
    assert_eq!(app.interface, original);
    app.close_preferences_window(&ctx);
    assert!(theme::custom(&ctx).is_none());
    assert_eq!(Palette::for_context(&ctx).accent, Palette::new(true).accent);
    app.open_preferences_window();
    click(&mut app, &ctx, "Custom");
    click(&mut app, &ctx, "Moonveil");
    let draft = &mut app
        .preferences_window
        .as_mut()
        .unwrap()
        .interface
        .custom_theme;
    assert_eq!(draft.preset, ThemePreset::Moonveil);
    assert_eq!(draft.accent, ThemePreset::Moonveil.accent());
    draft.strength = 42;
    app.configure_appearance(&ctx);
    app.save_preferences_window(&ctx);
    let saved = app.interface;
    assert_eq!(app.store.interface_preferences().unwrap(), saved);
    assert_eq!(theme::custom(&ctx), Some(saved.custom_theme));
    app.open_preferences_window();
    click(&mut app, &ctx, "Reset");
    app.save_preferences_window(&ctx);
    assert_eq!(app.interface.appearance, Appearance::Dark);
    assert!(theme::custom(&ctx).is_none());
    frame(&mut app, &ctx, egui::vec2(860.0, 600.0), vec![]);
}

#[test]
fn cover_preserves_aspect_ratio_and_keeps_the_anchored_edge() {
    let image = egui::vec2(1600.0, 900.0);
    let (wide, tall) = (egui::vec2(2000.0, 500.0), egui::vec2(500.0, 1000.0));
    for target in [wide, tall] {
        for anchor in [0.0, 0.5, 1.0] {
            let uv = cover_uv(image, target, anchor);
            assert!(uv.min.x >= 0.0 && uv.max.x <= 1.0 && uv.min.y >= 0.0 && uv.max.y <= 1.0);
            assert!(
                ((image.x * uv.width()) / (image.y * uv.height()) - target.x / target.y).abs()
                    < 0.001
            );
        }
    }
    // Side crops keep the anchored edge: presets the right, imports the centre.
    assert!((cover_uv(image, tall, 1.0).max.x - 1.0).abs() < 1e-6);
    assert!(cover_uv(image, tall, 0.0).min.x.abs() < 1e-6);
    assert!((cover_uv(image, tall, 0.5).center().x - 0.5).abs() < 1e-6);
    // Top and bottom crops stay centred whatever the anchor.
    assert!((cover_uv(image, wide, 1.0).center().y - 0.5).abs() < 1e-6);
}

fn custom_surface_luma() -> f32 {
    let ctx = egui::Context::default();
    theme::apply_custom(&ctx, Some(CustomTheme::default()));
    let surface = Palette::for_context(&ctx).surface;
    f32::from(luma(surface.r(), surface.g(), surface.b())) / 255.0
}

#[test]
fn strength_is_shared_and_only_bright_art_gets_a_darker_veil() {
    let surface = custom_surface_luma();
    assert_eq!(opacity(0), 0.0);
    assert_eq!(opacity(100), MAX_OPACITY);
    let full = opacity(100);
    // Every preset is dark enough to keep the lightest veil at full strength.
    for preset in ThemePreset::BUILT_IN {
        let art = load(Key::Preset(preset), Path::new("unused")).unwrap();
        assert_eq!(veil(art.bright, full, surface), MIN_VEIL, "{preset:?}");
    }
    // A white picture keeps the same opacity but is darkened to the text-safe limit,
    // where it previously had to be drawn at a fraction of the preset strength.
    let white = prepare(&image::DynamicImage::ImageRgb8(
        image::RgbImage::from_pixel(8, 8, image::Rgb([255; 3])),
    ));
    assert_eq!(white.bright, 1.0);
    let darker = veil(white.bright, full, surface);
    assert!(darker > 0.5);
    let backdrop = (1.0 - darker) * (full * white.bright + (1.0 - full) * surface);
    assert!(backdrop <= MAX_BACKDROP_LUMA + 1e-4);
    // Transparent pixels show the dark surface, so they do not count as bright.
    let clear = prepare(&image::DynamicImage::ImageRgba8(
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 0])),
    ));
    assert_eq!(clear.bright, 0.0);
}

#[test]
fn remove_image_returns_to_a_preset_cancel_keeps_it_and_save_deletes_it() {
    let (root, mut app, ctx) = fixture();
    let source = root.path().join("source.png");
    sample(&source, image::ImageFormat::Png);
    app.open_preferences_window();
    click(&mut app, &ctx, "Custom");
    let session = app.preferences_window.as_ref().unwrap().session;
    app.theme_assets
        .start_import(&ctx, session, source, app.store.paths.data.clone())
        .unwrap();
    wait_import(&mut app, &ctx);
    app.save_preferences_window(&ctx);
    let id = app.interface.custom_theme.image.unwrap();
    assert_eq!(app.interface.custom_theme.preset, ThemePreset::OwnImage);
    app.open_preferences_window();
    click(&mut app, &ctx, "Remove image");
    let draft = app
        .preferences_window
        .as_ref()
        .unwrap()
        .interface
        .custom_theme;
    assert_eq!(draft.image, None);
    assert_eq!(draft.preset, ThemePreset::default());
    assert_eq!(draft.accent, ThemePreset::default().accent());
    app.close_preferences_window(&ctx);
    assert!(image_path(&app.store.paths.data, id).exists());
    assert_eq!(app.interface.custom_theme.image, Some(id));
    app.open_preferences_window();
    click(&mut app, &ctx, "Remove image");
    app.save_preferences_window(&ctx);
    assert_eq!(
        app.store
            .interface_preferences()
            .unwrap()
            .custom_theme
            .image,
        None
    );
    assert!(!image_path(&app.store.paths.data, id).exists());
}

#[test]
fn missing_saved_image_reports_one_translated_notice() {
    let (_root, mut app, ctx) = fixture();
    app.interface.language = download_engine::i18n::Language::German;
    app.interface.appearance = Appearance::Custom;
    app.interface.custom_theme = CustomTheme {
        preset: ThemePreset::OwnImage,
        image: Some([7; 32]),
        ..Default::default()
    };
    app.configure_appearance(&ctx);
    app.notice = None;
    let message = "Background unavailable. Choose another image in Preferences.";
    let expected = {
        let _scope = download_engine::i18n::scope(download_engine::i18n::Language::German);
        tr(message).to_owned()
    };
    assert_ne!(expected, message);
    let size = egui::vec2(1080.0, 800.0);
    let until = Instant::now() + Duration::from_secs(10);
    while app.notice.is_none() {
        assert!(Instant::now() < until);
        frame(&mut app, &ctx, size, vec![]);
        app.background(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.notice.take().unwrap().message, expected);
    for _ in 0..3 {
        frame(&mut app, &ctx, size, vec![]);
        app.background(&ctx);
    }
    assert!(app.notice.is_none());
}

#[test]
fn abandoned_worker_result_removes_its_uncommitted_image() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.png");
    let profile = root.path().join("profile");
    sample(&source, image::ImageFormat::Png);
    let data = profile.clone();
    let task = background_io::Task::spawn(None, move || import(&source, &data)).unwrap();
    drop(task);
    assert_eq!(
        std::fs::read_dir(profile.join("themes")).unwrap().count(),
        0
    );
}

#[test]
fn failed_save_retains_draft_and_cancel_cleans_only_imported_copy() {
    let (root, mut app, ctx) = fixture();
    app.store.save_interface_preferences(app.interface).unwrap();
    let before = app.interface;
    let source = root.path().join("source.png");
    sample(&source, image::ImageFormat::Png);
    app.open_preferences_window();
    let session = app.preferences_window.as_ref().unwrap().session;
    app.theme_assets
        .start_import(&ctx, session, source.clone(), app.store.paths.data.clone())
        .unwrap();
    wait_import(&mut app, &ctx);
    let id = app
        .preferences_window
        .as_ref()
        .unwrap()
        .interface
        .custom_theme
        .image
        .unwrap();
    let conn =
        crate::desktop::fixture_database::open(app.store.paths.data.join("queue.sqlite3")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_theme BEFORE UPDATE ON interface_preferences BEGIN SELECT RAISE(ABORT, 'theme fixture write failure'); END;").unwrap();
    app.save_preferences_window(&ctx);
    assert_eq!(app.interface, before);
    assert_eq!(app.store.interface_preferences().unwrap(), before);
    assert!(
        app.preferences_window
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("theme fixture write failure")
    );
    assert!(image_path(&app.store.paths.data, id).exists());
    app.close_preferences_window(&ctx);
    assert!(!image_path(&app.store.paths.data, id).exists());
    assert!(source.exists());
}

#[test]
fn reset_ignores_an_import_which_has_not_been_applied_yet() {
    let (root, mut app, ctx) = fixture();
    let source = root.path().join("source.png");
    sample(&source, image::ImageFormat::Png);
    app.open_preferences_window();
    click(&mut app, &ctx, "Custom");
    let session = app.preferences_window.as_ref().unwrap().session;
    app.theme_assets
        .start_import(&ctx, session, source, app.store.paths.data.clone())
        .unwrap();
    click(&mut app, &ctx, "Reset");
    wait_import(&mut app, &ctx);
    let draft = app.preferences_window.as_ref().unwrap().interface;
    assert_eq!(draft.custom_theme, CustomTheme::default());
    assert_eq!(draft.appearance, Appearance::Dark);
    assert_eq!(
        std::fs::read_dir(app.store.paths.data.join("themes"))
            .unwrap()
            .count(),
        0
    );
}
