//! Native colors, typography and shared widget styling.
use super::*;

pub(super) fn menu_style(style: &mut egui::Style) {
    // Keep the toolbar's hover, pressed and focus visuals; only hide the idle frame.
    style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
}

pub(super) fn menu_config() -> egui::containers::menu::MenuConfig {
    egui::containers::menu::MenuConfig::new().style(|style: &mut egui::Style| {
        menu_style(style);
        style.spacing.button_padding = egui::vec2(6.0, 3.0);
        style.spacing.interact_size.y = 24.0;
    })
}

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub(super) background: Color32,
    pub(super) surface: Color32,
    pub(super) text: Color32,
    pub(super) muted: Color32,
    pub(super) border: Color32,
    pub(super) accent: Color32,
    pub(super) accent_text: Color32,
    pub(super) blue: Color32,
    pub(super) orange: Color32,
    pub(super) selected: Color32,
    pub(super) track: Color32,
    pub(super) success: Color32,
    pub(super) warning: Color32,
}

impl Palette {
    pub(super) fn new(dark: bool) -> Self {
        let rgb = |r, g, b| Color32::from_rgb(r, g, b);
        if dark {
            Self {
                background: rgb(38, 42, 45),
                surface: rgb(34, 38, 40),
                text: rgb(224, 230, 235),
                muted: rgb(163, 173, 183),
                border: rgb(59, 66, 72),
                accent: rgb(104, 211, 169),
                accent_text: rgb(24, 48, 38),
                blue: rgb(125, 182, 232),
                orange: rgb(228, 180, 108),
                selected: rgb(36, 54, 71),
                track: rgb(58, 66, 72),
                success: rgb(104, 211, 169),
                warning: rgb(243, 149, 149),
            }
        } else {
            Self {
                background: rgb(244, 246, 248),
                surface: Color32::WHITE,
                text: rgb(41, 51, 63),
                muted: rgb(110, 122, 136),
                border: rgb(225, 230, 235),
                accent: rgb(22, 124, 87),
                accent_text: Color32::WHITE,
                blue: rgb(44, 114, 180),
                orange: rgb(155, 105, 31),
                selected: rgb(237, 244, 252),
                track: rgb(230, 235, 239),
                success: rgb(22, 124, 87),
                warning: rgb(190, 70, 70),
            }
        }
    }

    pub(super) fn status(self, status: Status) -> Color32 {
        match status {
            Status::Downloading => self.accent,
            Status::Complete => self.success,
            Status::Paused => self.orange,
            Status::Failed => self.warning,
            _ => self.muted,
        }
    }
}

pub fn configure(ctx: &egui::Context) {
    // The Windows frame is themed before first display and repainted explicitly.
    // A late winit SetTheme changes its dark-mode flag without refreshing the
    // caption, leaving light pixels until a minimize/restore or partial redraw.
    #[cfg(windows)]
    ctx.options_mut(|options| options.sync_window_theme = false);
    // Use Windows UI fonts when available; retain bundled fonts as fallbacks.
    #[cfg(windows)]
    if let Some(windows) = std::env::var_os("WINDIR") {
        let mut fonts = egui::FontDefinitions::default();
        for (name, file) in [
            ("windows-ui", "segoeui.ttf"),
            ("windows-symbols", "seguisym.ttf"),
        ] {
            if let Ok(data) = std::fs::read(PathBuf::from(&windows).join("Fonts").join(file)) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(data).into());
                let family = fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default();
                if name == "windows-ui" {
                    family.insert(0, name.into());
                } else {
                    family.push(name.into());
                }
            }
        }
        // Windows language fonts supply CJK, Thai and Arabic glyphs. Use installed
        // fonts at runtime; Microsoft fonts are not redistributed in our package.
        for (name, file) in [
            ("chinese-simplified", "msyh.ttc"),
            ("chinese-traditional", "msjh.ttc"),
            ("chinese-fallback", "simsun.ttc"),
            ("japanese", "YuGothR.ttc"),
            ("japanese-fallback", "meiryo.ttc"),
            ("korean", "malgun.ttf"),
            ("thai", "LeelawUI.ttf"),
            ("arabic-thai-fallback", "tahoma.ttf"),
        ] {
            if let Ok(data) = std::fs::read(PathBuf::from(&windows).join("Fonts").join(file)) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(data).into());
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(family).or_default().push(name.into());
                }
            }
        }
        if let Ok(data) = std::fs::read(PathBuf::from(&windows).join("Fonts/segoeuib.ttf")) {
            fonts.font_data.insert(
                "windows-ui-bold".into(),
                egui::FontData::from_owned(data).into(),
            );
            let mut family = vec!["windows-ui-bold".into()];
            family.extend(fonts.families[&egui::FontFamily::Proportional].clone());
            fonts
                .families
                .insert(egui::FontFamily::Name("bold".into()), family);
        }
        if let Ok(data) = std::fs::read(PathBuf::from(&windows).join("Fonts/segmdl2.ttf")) {
            fonts.font_data.insert(
                "windows-icons".into(),
                egui::FontData::from_owned(data).into(),
            );
            fonts.families.insert(
                egui::FontFamily::Name("icons".into()),
                vec!["windows-icons".into()],
            );
        } else {
            fonts.families.insert(
                egui::FontFamily::Name("icons".into()),
                fonts.families[&egui::FontFamily::Proportional].clone(),
            );
        }
        ctx.set_fonts(fonts);
    }

    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let p = Palette::new(theme == egui::Theme::Dark);
        let mut style = egui::Style {
            visuals: match theme {
                egui::Theme::Light => egui::Visuals::light(),
                egui::Theme::Dark => egui::Visuals::dark(),
            },
            ..Default::default()
        };
        style.visuals.override_text_color = Some(p.text);
        style.visuals.panel_fill = p.background;
        style.visuals.window_fill = p.surface;
        style.visuals.extreme_bg_color = p.background;
        style.visuals.faint_bg_color = p.track;
        style.visuals.selection.bg_fill = p.selected;
        style.visuals.selection.stroke = egui::Stroke::new(1.0, p.accent);
        style.visuals.window_stroke = egui::Stroke::new(1.0, p.border);
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
        style.visuals.widgets.inactive.bg_fill = p.surface;
        style.visuals.widgets.inactive.weak_bg_fill = p.surface;
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, p.border);
        style.visuals.widgets.hovered.bg_fill = p.selected;
        style.visuals.widgets.hovered.weak_bg_fill = p.selected;
        style.visuals.widgets.inactive.corner_radius = 6.into();
        style.visuals.widgets.hovered.corner_radius = 6.into();
        style.visuals.widgets.active.corner_radius = 6.into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        // Makes buttons, dropdowns and number boxes the shared 26px control height.
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        ctx.set_style_of(theme, style);
    }
    ctx.set_theme(egui::ThemePreference::Dark);
}

#[cfg(test)]
mod tests;
