//! Palette, sizes, fonts and the egui style.

use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2};

// ── Colors ───────────────────────────────────────────────────────────────────

pub(super) const BG: Color32 = Color32::from_rgb(0, 0, 0);
pub(super) const TITLEBAR: Color32 = Color32::from_rgb(17, 17, 17);
pub(super) const RAISED: Color32 = Color32::from_rgb(17, 17, 17);
pub(super) const BORDER: Color32 = Color32::from_rgb(28, 28, 28);
pub(super) const BORDER2: Color32 = Color32::from_rgb(34, 34, 34);
pub(super) const RULE: Color32 = Color32::from_rgb(42, 42, 42);
pub(super) const FIELD_BORDER: Color32 = Color32::from_rgb(51, 51, 51);
pub(super) const FG: Color32 = Color32::from_rgb(216, 222, 233);
pub(super) const TEXT2: Color32 = Color32::from_rgb(230, 230, 230);
pub(super) const BRIGHT: Color32 = Color32::from_rgb(255, 255, 255);
pub(super) const SUBTLE: Color32 = Color32::from_rgb(139, 147, 163);
pub(super) const META: Color32 = Color32::from_rgb(107, 107, 107);
pub(super) const DIM: Color32 = Color32::from_rgb(90, 90, 90);
pub(super) const KEYCAP: Color32 = Color32::from_rgb(44, 44, 44);
pub(super) const CHIP_BG: Color32 = Color32::from_rgb(20, 20, 20);
pub(super) const CHIP_TEXT: Color32 = Color32::from_rgb(160, 160, 160);
pub(super) const ACCENT: Color32 = Color32::from_rgb(67, 177, 141); // #43B18D, muted version of the logo emerald (#10B981)
pub(super) const ON_ACCENT: Color32 = Color32::from_rgb(15, 17, 21);
pub(super) const GREEN: Color32 = Color32::from_rgb(152, 195, 121);
pub(super) const RED: Color32 = Color32::from_rgb(224, 108, 117);
pub(super) const YELLOW: Color32 = Color32::from_rgb(229, 192, 123);
pub(super) const ACTIVE_TAB: Color32 = Color32::from_rgb(38, 38, 38);
pub(super) const HOVER: Color32 = Color32::from_rgb(26, 26, 26);
pub(super) const CLOSE_HOVER: Color32 = Color32::from_rgb(58, 58, 58);
pub(super) const SELECTED: Color32 = Color32::from_rgb(26, 36, 51);
pub(super) const FAIL_BG: Color32 = Color32::from_rgb(18, 10, 11);
pub(super) const FIELD: Color32 = Color32::from_rgb(0, 0, 0);
pub(super) const CURSOR: Color32 = Color32::from_rgba_premultiplied(130, 134, 140, 150);
/// Selection in the fullscreen terminal: painted over the text.
pub(super) const SELECTION: Color32 = Color32::from_rgba_premultiplied(36, 52, 84, 110);
/// Selected text in blocks (egui's own selection) and in the prompt.
pub(super) const TEXT_SELECTION: Color32 = Color32::from_rgb(52, 74, 110);

/// 16-color ANSI palette (One Dark based), in code order.
pub(super) const ANSI: [Color32; 16] = [
    Color32::from_rgb(40, 44, 52),
    Color32::from_rgb(224, 108, 117),
    Color32::from_rgb(152, 195, 121),
    Color32::from_rgb(229, 192, 123),
    Color32::from_rgb(97, 175, 239),
    Color32::from_rgb(198, 120, 221),
    Color32::from_rgb(86, 182, 194),
    Color32::from_rgb(171, 178, 191),
    Color32::from_rgb(99, 106, 120),
    Color32::from_rgb(240, 135, 145),
    Color32::from_rgb(170, 215, 140),
    Color32::from_rgb(240, 210, 150),
    Color32::from_rgb(130, 195, 250),
    Color32::from_rgb(215, 145, 235),
    Color32::from_rgb(110, 205, 215),
    Color32::from_rgb(230, 233, 239),
];

// ── Sizes ────────────────────────────────────────────────────────────────────

pub(super) const TEXT_SIZE: f32 = 13.5;
pub(super) const SMALL_SIZE: f32 = 12.0;
/// Window corner radius: that of macOS 26 windows with a toolbar.
#[cfg(target_os = "macos")]
pub(super) const WINDOW_RADIUS: f64 = 24.0;
/// Tab bar: tall and same background, no separation from the rest.
pub(super) const TITLEBAR_H: f32 = 52.0;
/// Default space for the macOS close/minimize/zoom buttons
/// (adjusted to the real position when it can be read).
pub(super) const TRAFFIC_LIGHTS_W: f32 = 84.0;
/// Space for the close × at the end of each tab.
pub(super) const TAB_CLOSE_W: f32 = 22.0;
pub(super) const PROMPT_H: f32 = 106.0;
/// Lists above the prompt (commands, suggestions): max visible rows,
/// title height and row height.
pub(super) const DOCK_ROWS: usize = 6;
pub(super) const DOCK_HEAD_H: f32 = 30.0;
pub(super) const DOCK_ROW_H: f32 = 26.0;
/// Wheel amount needed to move a list selection by one row.
pub(super) const WHEEL_STEP: f32 = 24.0;
pub(super) const PAD: f32 = 20.0;
/// Block: vertical padding, context line and gap before the output.
pub(super) const BLOCK_PAD: f32 = 12.0;
pub(super) const META_H: f32 = 18.0;
pub(super) const OUT_GAP: f32 = 6.0;

// ── Fonts and style ──────────────────────────────────────────────────────────

pub(super) fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub(super) fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

/// System monospace font (with bold) and fallbacks for symbols and CJK.
/// Cascadia Mono, bundled in the binary, if nothing better exists.
pub(super) fn setup_fonts(ctx: &egui::Context) {
    const CASCADIA: &[u8] = include_bytes!("../../assets/fonts/CascadiaMono-Regular.ttf");
    let mut fonts = FontDefinitions::default();
    let load = |path: &str, index: u32| -> Option<FontData> {
        let data = std::fs::read(path).ok()?;
        let mut f = FontData::from_owned(data);
        f.index = index;
        Some(f)
    };

    let (regular, bold_face) = if cfg!(target_os = "macos") {
        (
            load("/System/Library/Fonts/Menlo.ttc", 0),
            load("/System/Library/Fonts/Menlo.ttc", 1),
        )
    } else {
        let first = |paths: &[&str]| paths.iter().find_map(|p| load(p, 0));
        (
            first(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
            ]),
            first(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono-Bold.ttf",
                "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono-Bold.ttf",
            ]),
        )
    };
    fonts
        .font_data
        .insert("cascadia".into(), Arc::new(FontData::from_static(CASCADIA)));
    let mut main = Vec::new();
    if let Some(f) = regular {
        fonts.font_data.insert("main".into(), Arc::new(f));
        main.push("main".to_string());
    }
    main.push("cascadia".into());
    let mut bold_chain = Vec::new();
    if let Some(f) = bold_face {
        fonts.font_data.insert("main-bold".into(), Arc::new(f));
        bold_chain.push("main-bold".to_string());
    }

    // Fallbacks: symbols (⎇ ❯ ⠋ ✓…) and CJK.
    let fallback_paths: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ]
    };
    let mut fallbacks = Vec::new();
    for (i, p) in fallback_paths.iter().enumerate() {
        if let Some(f) = load(p, 0) {
            let name = format!("fallback-{i}");
            fonts.font_data.insert(name.clone(), Arc::new(f));
            fallbacks.push(name);
        }
    }
    // egui's fallbacks (emoji, icons) go last.
    let egui_defaults = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let chain: Vec<String> = main
        .iter()
        .chain(fallbacks.iter())
        .chain(egui_defaults.iter())
        .cloned()
        .collect();
    let bold_full: Vec<String> = bold_chain.iter().chain(chain.iter()).cloned().collect();
    fonts.families.insert(FontFamily::Monospace, chain.clone());
    fonts.families.insert(FontFamily::Proportional, chain);
    fonts
        .families
        .insert(FontFamily::Name("bold".into()), bold_full);
    ctx.set_fonts(fonts);
}

pub(super) fn setup_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.visuals = egui::Visuals::dark();
        s.visuals.panel_fill = BG;
        s.visuals.window_fill = RAISED;
        s.visuals.extreme_bg_color = FIELD;
        s.visuals.override_text_color = Some(FG);
        s.visuals.selection.bg_fill = TEXT_SELECTION;
        s.visuals.selection.stroke = Stroke::new(1.0, BRIGHT);
        s.spacing.item_spacing = Vec2::ZERO;
        s.interaction.selectable_labels = true;
        for font in s.text_styles.values_mut() {
            *font = mono(TEXT_SIZE);
        }
    });
}
