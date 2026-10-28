//! Embedded artwork and bounded, asynchronous imports. Preferences own the draft;
//! workers stay here so dismissing a preview never joins a decoder on the UI thread.
use super::*;
use download_engine::preferences::{CustomTheme, ThemePreset};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read, Write};
use std::path::Path;

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;
/// Artwork opacity at 100% strength, the same for presets and imported images.
const MAX_OPACITY: f32 = 175.0 / 255.0;
/// The lightest dark veil, used whenever the artwork is already dark enough.
const MIN_VEIL: f32 = 45.0 / 255.0;
/// Brightest backdrop allowed behind text, as gamma-space luma. Aurora's sky, the
/// brightest preset, just reaches it at 100%, so presets keep the lightest veil and
/// brighter imported images are darkened instead of being drawn fainter.
const MAX_BACKDROP_LUMA: f32 = 0.34;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Preset(ThemePreset),
    Image([u8; 32]),
}

impl Key {
    fn of(theme: CustomTheme) -> Option<Self> {
        if theme.preset == ThemePreset::OwnImage {
            theme.image.map(Self::Image)
        } else {
            Some(Self::Preset(theme.preset))
        }
    }

    /// Presets keep characters and bright scenery on the right; imports stay centred.
    fn anchor(self) -> f32 {
        match self {
            Self::Preset(_) => 1.0,
            Self::Image(_) => 0.5,
        }
    }
}

/// Decoded pixels and their brightness, both prepared off the UI thread.
#[derive(Debug)]
struct Decoded {
    image: egui::ColorImage,
    /// 90th-percentile luma, 0–1.
    bright: f32,
}

#[derive(Clone)]
struct Art {
    texture: egui::TextureHandle,
    bright: f32,
}

#[derive(Default)]
pub(super) struct State {
    textures: HashMap<Key, Result<Art, String>>,
    loading: Option<(Key, background_io::Task<Decoded>)>,
    importing: Option<(Uuid, background_io::Task<Imported>)>,
    created: Vec<CreatedImage>,
    error: Option<String>,
    /// The unavailable background already reported, so its notice appears once.
    reported: Option<Key>,
}

#[derive(Debug)]
struct Imported {
    id: [u8; 32],
    image: Decoded,
    created: Option<CreatedImage>,
}

/// Uncommitted files are also removed if the app closes before an import is polled.
#[derive(Debug)]
struct CreatedImage {
    id: [u8; 32],
    path: Option<PathBuf>,
}
impl Drop for CreatedImage {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn embedded(preset: ThemePreset) -> &'static [u8] {
    match preset {
        ThemePreset::NeonCourier => include_bytes!("../../assets/themes/neon-courier.jpg"),
        ThemePreset::ReactorNight => include_bytes!("../../assets/themes/reactor-night.jpg"),
        ThemePreset::Aurora => include_bytes!("../../assets/themes/aurora.jpg"),
        ThemePreset::Moonveil => include_bytes!("../../assets/themes/moonveil.jpg"),
        ThemePreset::OwnImage => &[],
    }
}

fn image_path(data: &Path, id: [u8; 32]) -> PathBuf {
    let name: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
    data.join("themes").join(format!("{name}.png"))
}

fn read_bounded(path: &Path) -> anyhow::Result<Vec<u8>> {
    let file = File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "Choose a regular PNG or JPEG file"
    );
    anyhow::ensure!(
        file.metadata()?.len() <= MAX_BYTES,
        "Theme images must be 16 MiB or smaller"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Theme images must be 16 MiB or smaller"
    );
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> anyhow::Result<image::DynamicImage> {
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Theme images must be 16 MiB or smaller"
    );
    let format = image::guess_format(bytes)?;
    anyhow::ensure!(
        matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg),
        "Choose a PNG or JPEG image"
    );
    let reader = || {
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        reader
    };
    let (width, height) = reader().into_dimensions()?;
    anyhow::ensure!(
        u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "Theme images must be 16 megapixels or smaller"
    );
    let image = reader().decode()?;
    Ok(if width > 2048 || height > 2048 {
        image.thumbnail(2048, 2048)
    } else {
        image
    })
}

/// Rec. 709 luma of gamma-encoded sRGB, matching egui's gamma-space blending.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) + 5000) / 10_000) as u8
}

fn prepare(image: &image::DynamicImage) -> Decoded {
    let rgba = image.to_rgba8();
    // Transparent pixels show the dark window surface, so weight luma by alpha.
    let mut histogram = [0_u64; 256];
    for pixel in rgba.pixels() {
        let [r, g, b, a] = pixel.0;
        histogram[usize::from(luma(r, g, b)) * usize::from(a) / 255] += 1;
    }
    let total: u64 = histogram.iter().sum();
    let mut seen = 0;
    let p90 = histogram
        .iter()
        .position(|&count| {
            seen += count;
            seen * 10 >= total * 9
        })
        .unwrap_or(255);
    Decoded {
        image: egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ),
        bright: p90 as f32 / 255.0,
    }
}

/// Darkening needed so the brightest tenth of the artwork, blended over the
/// window surface and then veiled, stays at or below `MAX_BACKDROP_LUMA`.
fn veil(bright: f32, opacity: f32, surface: f32) -> f32 {
    let composite = opacity * bright + (1.0 - opacity) * surface;
    (1.0 - MAX_BACKDROP_LUMA / composite).max(MIN_VEIL)
}

fn opacity(strength: u8) -> f32 {
    f32::from(strength.min(100)) / 100.0 * MAX_OPACITY
}

fn load(key: Key, data: &Path) -> anyhow::Result<Decoded> {
    let bytes = match key {
        Key::Preset(preset) => embedded(preset).to_vec(),
        Key::Image(id) => {
            let bytes = read_bounded(&image_path(data, id))?;
            anyhow::ensure!(
                <[u8; 32]>::from(Sha256::digest(&bytes)) == id,
                "The saved theme image has changed; choose it again"
            );
            bytes
        }
    };
    Ok(prepare(&decode(&bytes)?))
}

fn import(path: &Path, data: &Path) -> anyhow::Result<Imported> {
    let decoded = decode(&read_bounded(path)?)?;
    let mut output = Cursor::new(Vec::new());
    decoded.write_to(&mut output, image::ImageFormat::Png)?;
    let bytes = output.into_inner();
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "The converted theme image is too large"
    );
    let id = Sha256::digest(&bytes).into();
    let destination = image_path(data, id);
    std::fs::create_dir_all(destination.parent().unwrap())?;
    // Content-addressed files are immutable. Never overwrite another saved image.
    let created = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
    {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = std::fs::remove_file(&destination);
                return Err(error.into());
            }
            Some(CreatedImage {
                id,
                path: Some(destination),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::ensure!(
                read_bounded(&destination)? == bytes,
                "The saved theme image is damaged; remove it before importing again"
            );
            None
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Imported {
        id,
        image: prepare(&decoded),
        created,
    })
}

impl State {
    fn poll_texture(&mut self, ctx: &egui::Context) {
        if let Some((key, task)) = &mut self.loading
            && let Some(result) = task.poll()
        {
            let art = result
                .map(|decoded| Art {
                    texture: ctx.load_texture(
                        format!("theme-{key:?}"),
                        decoded.image,
                        egui::TextureOptions::LINEAR,
                    ),
                    bright: decoded.bright,
                })
                .map_err(|error| error.to_string());
            self.textures.insert(*key, art);
            self.loading = None;
        }
    }

    fn art(&mut self, ctx: &egui::Context, key: Key, data: &Path) -> Option<Art> {
        self.poll_texture(ctx);
        if !self.textures.contains_key(&key) && self.loading.is_none() {
            let directory = data.to_owned();
            match background_io::Task::spawn(Some(ctx.clone()), move || load(key, &directory)) {
                Ok(task) => self.loading = Some((key, task)),
                Err(error) => {
                    self.textures.insert(key, Err(error.to_string()));
                }
            }
        }
        self.textures
            .get(&key)
            .and_then(|texture| texture.as_ref().ok())
            .cloned()
    }

    #[cfg(feature = "screenshot")]
    pub(super) fn busy(&self) -> bool {
        self.loading.is_some() || self.importing.is_some()
    }

    pub(super) fn importing(&self) -> bool {
        self.importing.is_some()
    }

    pub(super) fn replace_saved_image(
        &mut self,
        data: &Path,
        previous: Option<[u8; 32]>,
        saved: Option<[u8; 32]>,
    ) {
        if previous != saved
            && let Some(id) = previous
        {
            match std::fs::remove_file(image_path(data, id)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    pub(super) fn discard_imports(&mut self, saved: Option<[u8; 32]>) {
        self.created.retain_mut(|created| {
            if Some(created.id) == saved {
                created.path = None;
                return false;
            }
            match std::fs::remove_file(created.path.as_ref().expect("Uncommitted image path")) {
                Ok(()) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => {
                    self.error = Some(error.to_string());
                    true
                }
            }
        });
    }

    fn start_import(
        &mut self,
        ctx: &egui::Context,
        session: Uuid,
        path: PathBuf,
        data: PathBuf,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.importing.is_none(), "An image is already loading");
        self.error = None;
        self.importing = Some((
            session,
            background_io::Task::spawn(Some(ctx.clone()), move || import(&path, &data))?,
        ));
        Ok(())
    }

    /// Paints the window behind every panel: surface, artwork, then a dark veil
    /// that is just strong enough for the artwork's brightness.
    pub(super) fn backdrop(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        custom: CustomTheme,
        data: &Path,
        p: Palette,
    ) {
        ui.painter().rect_filled(rect, 0, p.surface);
        let Some(key) = Key::of(custom) else {
            return;
        };
        let Some(art) = self.art(ui.ctx(), key, data) else {
            return;
        };
        let opacity = opacity(custom.strength);
        ui.painter().image(
            art.texture.id(),
            rect,
            cover_uv(art.texture.size_vec2(), rect.size(), key.anchor()),
            Color32::from_white_alpha((opacity * 255.0).round() as u8),
        );
        let surface = f32::from(luma(p.surface.r(), p.surface.g(), p.surface.b())) / 255.0;
        let veil = veil(art.bright, opacity, surface);
        ui.painter().rect_filled(
            rect,
            0,
            Color32::from_black_alpha((veil * 255.0).round() as u8),
        );
    }

    /// True once for each theme background that cannot be loaded.
    fn newly_unavailable(&mut self, custom: CustomTheme) -> bool {
        let Some(key) = Key::of(custom) else {
            return false;
        };
        if self.reported != Some(key) && self.textures.get(&key).is_some_and(Result::is_err) {
            self.reported = Some(key);
            return true;
        }
        false
    }

    pub(super) fn gallery(
        &mut self,
        ui: &mut egui::Ui,
        custom: &mut CustomTheme,
        session: &mut Uuid,
        data: &Path,
        picker: &mut folder_picker::FolderPicker,
        initial: &Path,
    ) {
        let p = Palette::for_context(ui.ctx());
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tr("Theme collection")).strong())
            .on_hover_text(tr(
                "Preview a theme. Save to keep it, or Cancel to go back.",
            ));
        let columns = if ui.available_width() >= 500.0 {
            4
        } else if ui.available_width() >= 280.0 {
            2
        } else {
            1
        };
        let width = (ui.available_width() - ui.spacing().item_spacing.x * (columns - 1) as f32)
            / columns as f32;
        for row in ThemePreset::BUILT_IN.chunks(columns) {
            ui.horizontal(|ui| {
                for &preset in row {
                    let key = Key::Preset(preset);
                    let art = self.art(ui.ctx(), key, data);
                    let selected = custom.preset == preset;
                    let card = card(
                        ui,
                        p,
                        width,
                        art.as_ref(),
                        key.anchor(),
                        preset.name(),
                        selected,
                    );
                    if card.clicked() {
                        *session = Uuid::new_v4();
                        custom.preset = preset;
                        custom.accent = preset.accent();
                    }
                }
            });
        }
        // The imported image is a card like the presets, with its controls beside it.
        let mut choose = false;
        let layout = if columns > 1 {
            egui::Layout::left_to_right(egui::Align::Min)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        ui.with_layout(layout, |ui| {
            let art = custom
                .image
                .and_then(|id| self.art(ui.ctx(), Key::Image(id), data));
            let selected = custom.preset == ThemePreset::OwnImage;
            let mut own = card(ui, p, width, art.as_ref(), 0.5, tr("Your image"), selected);
            if custom.image.is_none() {
                // An empty tile is a shortcut for choosing the first image.
                let area = own.rect.with_max_y(own.rect.bottom() - 28.0);
                ui.painter().text(
                    area.center(),
                    egui::Align2::CENTER_CENTER,
                    "+",
                    egui::FontId::proportional(28.0),
                    p.muted,
                );
                own = own.on_hover_text(tr("Choose image…"));
            }
            if own.clicked() {
                if custom.image.is_some() {
                    *session = Uuid::new_v4();
                    custom.preset = ThemePreset::OwnImage;
                } else {
                    choose = true;
                }
            }
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    let idle = self.importing.is_none();
                    choose |= ui
                        .add_enabled(idle, egui::Button::new(tr("Choose image…")))
                        .clicked();
                    if custom.image.is_some()
                        && ui
                            .add_enabled(idle, egui::Button::new(tr("Remove image")))
                            .clicked()
                    {
                        // Save deletes the profile copy; Cancel keeps it.
                        *session = Uuid::new_v4();
                        custom.image = None;
                        if custom.preset == ThemePreset::OwnImage {
                            custom.preset = ThemePreset::default();
                            custom.accent = custom.preset.accent();
                        }
                    }
                    if !idle {
                        ui.spinner();
                        ui.label(tr("Loading image…"));
                    }
                });
                ui.label(
                    egui::RichText::new(tr(
                        "PNG / JPEG · up to 16 MiB and 16 megapixels. Copied to this profile.",
                    ))
                    .small()
                    .color(p.muted),
                );
            });
        });
        if choose && self.importing.is_none() {
            match picker.open_file(
                tr("Choose theme image"),
                initial,
                (tr("PNG or JPEG images"), "*.png;*.jpg;*.jpeg"),
            ) {
                Ok(Some(path)) => {
                    if let Err(error) = self.start_import(ui.ctx(), *session, path, data.to_owned())
                    {
                        self.error = Some(error.to_string());
                    }
                }
                Ok(None) => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("Background strength"));
            ui.scope(|ui| {
                // The default rail is the surface colour and vanishes on this card.
                let visuals = ui.visuals_mut();
                visuals.widgets.inactive.bg_fill = p.track;
                visuals.selection.bg_fill = p.accent;
                ui.add(
                    egui::Slider::new(&mut custom.strength, 0..=100)
                        .suffix("%")
                        .trailing_fill(true),
                );
            });
            ui.label(tr("Accent"));
            ui.color_edit_button_srgb(&mut custom.accent);
            let [r, g, b] = custom.accent;
            if theme::readable_accent(custom.accent) != Color32::from_rgb(r, g, b) {
                ui.label(
                    egui::RichText::new(tr("Lightened for readability"))
                        .small()
                        .color(p.muted),
                );
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(p.warning, error);
        }
        ui.add_space(8.0);
    }
}

/// A theme tile: 16:9 artwork above its name. The art area stays blank while loading.
fn card(
    ui: &mut egui::Ui,
    p: Palette,
    width: f32,
    art: Option<&Art>,
    anchor: f32,
    name: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, width * 0.5625 + 28.0),
        egui::Sense::click(),
    );
    let area = egui::Rect::from_min_size(rect.min, egui::vec2(width, rect.height() - 28.0));
    ui.painter().rect_filled(rect, 6, p.background);
    if let Some(art) = art {
        ui.painter().image(
            art.texture.id(),
            area.shrink(2.0),
            cover_uv(art.texture.size_vec2(), area.size(), anchor),
            Color32::WHITE,
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 9.0, rect.bottom() - 14.0),
        egui::Align2::LEFT_CENTER,
        format!("{}{}", if selected { "✓  " } else { "" }, name),
        egui::FontId::proportional(13.0),
        p.text,
    );
    ui.painter().rect_stroke(
        rect.shrink(1.0),
        6,
        egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected || response.hovered() || response.has_focus() {
                p.accent
            } else {
                p.border
            },
        ),
        egui::StrokeKind::Inside,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, name)
    });
    response
}

/// Crops to fill `target` without stretching. `anchor` places a horizontal crop
/// (0 keeps the left edge, 1 the right); a vertical crop stays centred.
fn cover_uv(image: egui::Vec2, target: egui::Vec2, anchor: f32) -> egui::Rect {
    let image_aspect = image.x / image.y;
    let target_aspect = target.x / target.y.max(1.0);
    let size = if image_aspect > target_aspect {
        egui::vec2(target_aspect / image_aspect, 1.0)
    } else {
        egui::vec2(1.0, image_aspect / target_aspect)
    };
    egui::Rect::from_min_size(
        egui::pos2((1.0 - size.x) * anchor, (1.0 - size.y) * 0.5),
        size,
    )
}

impl Desktop {
    pub(super) fn poll_theme_assets(&mut self, ctx: &egui::Context) {
        self.theme_assets.poll_texture(ctx);
        if let Some((session, task)) = &mut self.theme_assets.importing
            && let Some(result) = task.poll()
        {
            let session = *session;
            self.theme_assets.importing = None;
            match result {
                Ok(imported) => {
                    if let Some(draft) = &mut self.preferences_window
                        && draft.session == session
                    {
                        if let Some(path) = imported.created {
                            self.theme_assets.created.push(path);
                        }
                        draft.interface.custom_theme.image = Some(imported.id);
                        draft.interface.custom_theme.preset = ThemePreset::OwnImage;
                        self.theme_assets.textures.insert(
                            Key::Image(imported.id),
                            Ok(Art {
                                texture: ctx.load_texture(
                                    "own-theme",
                                    imported.image.image,
                                    egui::TextureOptions::LINEAR,
                                ),
                                bright: imported.image.bright,
                            }),
                        );
                        self.configure_appearance(ctx);
                    }
                }
                Err(error)
                    if self
                        .preferences_window
                        .as_ref()
                        .is_some_and(|draft| draft.session == session) =>
                {
                    self.theme_assets.error = Some(error.to_string())
                }
                Err(_) => {}
            }
        }
        let current = self
            .preferences_window
            .as_ref()
            .map_or(self.interface.custom_theme.image, |state| {
                state.interface.custom_theme.image
            });
        let saved = self.interface.custom_theme.image;
        self.theme_assets.textures.retain(|key, _| match key {
            Key::Preset(_) => true,
            Key::Image(id) => Some(*id) == current || Some(*id) == saved,
        });
        // Reported through the shared notice so the page layout does not shift.
        if let Some(custom) = theme::custom(ctx)
            && self.theme_assets.newly_unavailable(custom)
        {
            self.notice = Some(Notice::error(
                tr("Background unavailable. Choose another image in Preferences.").into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests;
