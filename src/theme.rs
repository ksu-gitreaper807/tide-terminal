use eframe::egui::{self, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};

pub const BG: Color32 = Color32::from_rgb(20, 22, 26);
pub const CHROME: Color32 = Color32::from_rgb(25, 28, 33);
pub const PANEL: Color32 = Color32::from_rgb(29, 32, 38);
pub const BORDER: Color32 = Color32::from_rgb(48, 53, 62);
pub const INK: Color32 = Color32::from_rgb(225, 229, 235);
pub const MUTED: Color32 = Color32::from_rgb(150, 158, 170);
pub const ACCENT: Color32 = Color32::from_rgb(160, 188, 224);
pub const CURSOR: Color32 = Color32::from_rgb(110, 184, 204);
pub const WARNING: Color32 = Color32::from_rgb(234, 204, 139);
pub const DANGER: Color32 = Color32::from_rgb(225, 155, 155);
const TERMINAL_ICONS: &str = "terminal-icons";

pub fn apply(ctx: &egui::Context) {
    apply_with_font(ctx, "").expect("built-in fonts");
}

pub fn validate_terminal_font(path: &str) -> anyhow::Result<Option<Vec<u8>>> {
    use anyhow::Context;
    if path.trim().is_empty() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).with_context(|| format!("Read terminal font {path}"))?;
    ttf_parser::Face::parse(&bytes, 0)
        .map_err(|e| anyhow::anyhow!("Invalid terminal font {path}: {e:?}"))?;
    Ok(Some(bytes))
}

pub fn apply_with_font(ctx: &egui::Context, font_file: &str) -> anyhow::Result<()> {
    let custom = validate_terminal_font(font_file)?;
    // Terminal control punctuation must not trigger egui's global Ctrl+- zoom.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    let mut fonts = egui::FontDefinitions::default();
    for (name, family, paths) in [
        (
            "ui-sans",
            egui::FontFamily::Proportional,
            vec![
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
            ],
        ),
        (
            "terminal-mono",
            egui::FontFamily::Monospace,
            vec![
                "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            ],
        ),
    ] {
        for path in paths {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .entry(family.clone())
                    .or_default()
                    .insert(0, name.into());
                break;
            }
        }
    }
    if let Some(bytes) = custom {
        fonts.font_data.insert(
            "custom-terminal".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "custom-terminal".into());
    }
    let nerd = "nerd-symbols";
    fonts.font_data.insert(
        nerd.into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/SymbolsNerdFontMono-Regular.ttf"
        ))
        .into(),
    );
    for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        let stack = fonts.families.entry(family).or_default();
        // Retain the primary text face and its metrics. Icon glyphs must be
        // checked before older fallbacks with incompatible private-use maps.
        stack.insert(1.min(stack.len()), nerd.into());
    }
    fonts.families.insert(
        egui::FontFamily::Name(TERMINAL_ICONS.into()),
        vec![nerd.into()],
    );
    // Noto Sans Mono and the embedded defaults omit Braille plot glyphs.
    // Keep a bundled fallback so btop and other TUIs work on every install.
    let fallback = "terminal-symbols";
    fonts.font_data.insert(
        fallback.into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/DejaVuSans.ttf")).into(),
    );
    for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(fallback.into());
    }
    let symbols = "unicode-symbols";
    fonts.font_data.insert(
        symbols.into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/NotoSansSymbols2-Regular.ttf"
        ))
        .into(),
    );
    for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(symbols.into());
    }
    let symbols = "unicode-technical-symbols";
    fonts.font_data.insert(
        symbols.into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/NotoSansSymbols-Regular.ttf"
        ))
        .into(),
    );
    for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(symbols.into());
    }
    for path in [
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".into());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (egui::TextStyle::Heading, FontId::proportional(20.)),
        (egui::TextStyle::Body, FontId::proportional(13.)),
        (egui::TextStyle::Button, FontId::proportional(13.)),
        (egui::TextStyle::Small, FontId::proportional(12.)),
        (egui::TextStyle::Monospace, FontId::monospace(13.)),
    ]
    .into();
    style.spacing.button_padding = Vec2::new(12., 7.);
    style.spacing.item_spacing = Vec2::new(10., 8.);
    style.spacing.interact_size.y = 28.;
    style.spacing.window_margin = egui::Margin::same(24);
    style.spacing.slider_width = 180.;
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.spacing.scroll.bar_width = 5.;
    style.spacing.scroll.bar_inner_margin = 8.;
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(INK);
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(22, 25, 30);
    visuals.faint_bg_color = Color32::from_rgb(36, 40, 47);
    visuals.selection.bg_fill = Color32::from_rgb(51, 70, 94);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 2,
        color: Color32::from_black_alpha(100),
    };
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(5);
        widget.bg_stroke = Stroke::new(1.0_f32, BORDER);
        widget.fg_stroke = Stroke::new(1.0_f32, INK);
    }
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(37, 41, 48);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(47, 53, 63);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(78, 91, 109));
    visuals.widgets.active.bg_fill = Color32::from_rgb(49, 65, 86);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals = visuals;
    ctx.set_style(style);
    Ok(())
}

pub fn terminal_icon_font(ctx: &egui::Context, text: &str, cell: Vec2) -> Option<FontId> {
    let c = text.chars().next()?;
    if !matches!(c as u32, 0xe000..=0xf8ff | 0xf0000..=0xffffd | 0x100000..=0x10fffd) {
        return None;
    }
    ctx.fonts(|fonts| {
        let mut font = FontId::new(cell.y, egui::FontFamily::Name(TERMINAL_ICONS.into()));
        if !fonts.has_glyph(&font, c) {
            return None;
        }
        let width = fonts.glyph_width(&font, c).max(1.);
        let height = fonts.row_height(&font).max(1.);
        font.size *= (cell.x / width).min(cell.y / height).min(1.);
        Some(font)
    })
}

#[derive(Clone, Copy)]
pub enum Icon {
    Plus,
    Close,
    Settings,
    Terminal,
    Folder,
}

pub fn icon(ui: &egui::Ui, rect: Rect, kind: Icon, color: Color32) {
    let p = ui.painter();
    let center = rect.center();
    let at = |x, y| center + Vec2::new(x, y);
    let stroke = Stroke::new(1.3_f32, color);
    match kind {
        Icon::Plus => {
            p.line_segment([at(-4.5, 0.), at(4.5, 0.)], stroke);
            p.line_segment([at(0., -4.5), at(0., 4.5)], stroke);
        }
        Icon::Close => {
            p.line_segment([at(-3.5, -3.5), at(3.5, 3.5)], stroke);
            p.line_segment([at(-3.5, 3.5), at(3.5, -3.5)], stroke);
        }
        Icon::Settings => {
            for (y, knob) in [(-4., -2.), (0., 3.), (4., -1.)] {
                p.line_segment([at(-6., y), at(6., y)], stroke);
                p.circle_filled(at(knob, y), 2.4, CHROME);
                p.circle_stroke(at(knob, y), 2., stroke);
            }
        }
        Icon::Terminal => {
            p.line_segment([at(-5., -4.), at(-1., 0.)], stroke);
            p.line_segment([at(-1., 0.), at(-5., 4.)], stroke);
            p.line_segment([at(1., 4.), at(6., 4.)], stroke);
        }
        Icon::Folder => {
            p.add(egui::Shape::closed_line(
                vec![
                    at(-6., -4.),
                    at(-1., -4.),
                    at(1., -2.),
                    at(6., -2.),
                    at(6., 5.),
                    at(-6., 5.),
                ],
                stroke,
            ));
        }
    }
}

pub fn icon_button(ui: &mut egui::Ui, kind: Icon, tooltip: &str) -> egui::Response {
    let response = ui
        .add(
            egui::Button::new("")
                .frame(false)
                .min_size(Vec2::new(28., 28.)),
        )
        .on_hover_text(tooltip);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    icon(
        ui,
        response.rect,
        kind,
        if response.hovered() { INK } else { MUTED },
    );
    response
}

pub fn dialog(ctx: &egui::Context, id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(130))
        .frame(egui::Frame::window(&ctx.style()).inner_margin(24))
}

pub fn heading(ui: &mut egui::Ui, title: &str) -> bool {
    ui.visuals_mut().override_text_color = Some(INK.gamma_multiply(0.9));
    let mut close = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(20.).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close = icon_button(ui, Icon::Close, "Close · Esc").clicked();
        });
    });
    ui.add_space(8.);
    close
}

pub fn hint(text: &str) -> RichText {
    RichText::new(text).color(INK.gamma_multiply(0.55))
}

pub fn help(text: impl Into<String>) -> RichText {
    RichText::new(text).small().color(INK.gamma_multiply(0.7))
}

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.);
    ui.label(RichText::new(title).strong().size(13.));
    ui.add_space(4.);
}

pub fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::from_rgb(229, 238, 250)))
        .fill(Color32::from_rgb(49, 76, 110))
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(75, 105, 142)))
        .min_size(Vec2::new(112., 32.))
}

pub fn short_path(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if path == home {
            return "~".into();
        }
        if let Some(rest) = path
            .strip_prefix(home.as_ref())
            .filter(|s| s.starts_with('/'))
        {
            return format!("~{rest}");
        }
    }
    path.to_owned()
}

pub fn duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!("{}h {:02}m", seconds / 3600, (seconds / 60) % 60)
    } else if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

pub fn separator(ui: &egui::Ui, left: Pos2, right: Pos2) {
    ui.painter()
        .line_segment([left, right], Stroke::new(1.0_f32, BORDER));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(text: &str) -> impl Iterator<Item = char> + '_ {
        text.lines()
            .map(|line| char::from_u32(u32::from_str_radix(line, 16).unwrap()).unwrap())
    }

    #[test]
    fn complete_icon_and_unicode_symbol_catalogues_have_glyphs() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts(|fonts| {
                let font = FontId::monospace(16.);
                for catalogue in [
                    include_str!("../tests/fixtures/nerd-codepoints.txt"),
                    include_str!("../tests/fixtures/eza-codepoints.txt"),
                    include_str!("../tests/fixtures/unicode-symbol-codepoints.txt"),
                ] {
                    for c in codes(catalogue) {
                        // egui 0.31 reserves this very glyph as its replacement
                        // and reports has_glyph(false) even when it is present.
                        if c == '\u{25fb}' {
                            continue;
                        }
                        assert!(fonts.has_glyph(&font, c), "missing U+{:04X}", c as u32);
                    }
                }
            });
        });
    }

    #[test]
    fn private_use_icons_fit_terminal_cells_at_supported_font_sizes() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            for size in [10., 12., 16., 24., 32.] {
                let normal = FontId::monospace(size);
                let cell = ctx.fonts(|f| Vec2::new(f.glyph_width(&normal, 'M'), f.row_height(&normal)));
                assert!(terminal_icon_font(ctx, "A", cell).is_none());
                for c in codes(include_str!("../tests/fixtures/nerd-codepoints.txt")) {
                    if !matches!(c as u32, 0xe000..=0xf8ff | 0xf0000..=0xffffd | 0x100000..=0x10fffd) { continue; }
                    let text = c.to_string();
                    let font = terminal_icon_font(ctx, &text, cell).unwrap();
                    assert_eq!(font.family, egui::FontFamily::Name(TERMINAL_ICONS.into()));
                    let width = ctx.fonts(|f| f.glyph_width(&font, c));
                    let height = ctx.fonts(|f| f.row_height(&font));
                    assert!(width <= cell.x + 0.5 && height <= cell.y + 0.5,
                        "U+{:04X} at {size}px: {width}x{height} exceeds {cell:?}", c as u32);
                }
            }
        });
    }

    #[test]
    fn terminal_font_covers_btop_graphs_borders_and_navigation() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts(|fonts| {
                let font = FontId::monospace(16.);
                for code in (0x2500..0x25a0).chain(0x2800..0x2900) {
                    let glyph = char::from_u32(code).unwrap();
                    assert!(fonts.has_glyph(&font, glyph), "missing U+{code:04X}");
                }
                assert!(fonts.has_glyphs(&font, "↑↓←→↵¹²³⁴⁵°"));
            });
        });
    }
}

#[cfg(test)]
mod custom_font_tests {
    use super::*;
    #[test]
    fn custom_font_is_validated_before_replacing_the_working_stack() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("font.ttf");
        assert!(validate_terminal_font("").unwrap().is_none());
        assert!(validate_terminal_font(path.to_str().unwrap()).is_err());
        std::fs::write(&path, b"not a font").unwrap();
        assert!(validate_terminal_font(path.to_str().unwrap()).is_err());
        std::fs::write(&path, include_bytes!("../assets/fonts/DejaVuSans.ttf")).unwrap();
        assert!(
            validate_terminal_font(path.to_str().unwrap())
                .unwrap()
                .is_some()
        );
    }
}
