//! Native egui window: title bar with tabs, blocks as panels, prompt,
//! side editor and overlay windows, with rounded corners, padding, shadows
//! and selectable text.
//!
//! All logic lives in `App`: this only draws and
//! translates egui events into the crossterm events `App` understands.
//! The cell grid is used only where needed: command output and
//! fullscreen programs.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Id,
    Key, Label, LayerId, Order, Pos2, Rect, Sense, Shadow, Stroke, TextFormat, Ui, Vec2,
    ViewportCommand, text::LayoutJob,
};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::style::{Color, Modifier, Style};
use vt100::MouseProtocolMode;

use crate::app::overlay::{Form, Overlay, Picker};
use crate::app::tab::{CmdBlock, CmdState, Focus, Tab, short_path};
use crate::app::{App, keycaps};
use crate::edit::markdown;
use crate::event::{AppEvent, EventSink};
use crate::store::Store;
use crate::term::keys;
use crate::term::output::{Cell, line_rows, segments};

// ── Theme ────────────────────────────────────────────────────────────────────

const BG: Color32 = Color32::from_rgb(0, 0, 0);
const TITLEBAR: Color32 = Color32::from_rgb(17, 17, 17);
const RAISED: Color32 = Color32::from_rgb(17, 17, 17);
const BORDER: Color32 = Color32::from_rgb(28, 28, 28);
const BORDER2: Color32 = Color32::from_rgb(34, 34, 34);
const RULE: Color32 = Color32::from_rgb(42, 42, 42);
const FIELD_BORDER: Color32 = Color32::from_rgb(51, 51, 51);
const FG: Color32 = Color32::from_rgb(216, 222, 233);
const TEXT2: Color32 = Color32::from_rgb(230, 230, 230);
const BRIGHT: Color32 = Color32::from_rgb(255, 255, 255);
const SUBTLE: Color32 = Color32::from_rgb(139, 147, 163);
const META: Color32 = Color32::from_rgb(107, 107, 107);
const DIM: Color32 = Color32::from_rgb(90, 90, 90);
const KEYCAP: Color32 = Color32::from_rgb(44, 44, 44);
const CHIP_BG: Color32 = Color32::from_rgb(20, 20, 20);
const CHIP_TEXT: Color32 = Color32::from_rgb(160, 160, 160);
const ACCENT: Color32 = Color32::from_rgb(67, 177, 141); // #43B18D, muted version of the logo emerald (#10B981)
const ON_ACCENT: Color32 = Color32::from_rgb(15, 17, 21);
const GREEN: Color32 = Color32::from_rgb(152, 195, 121);
const RED: Color32 = Color32::from_rgb(224, 108, 117);
const YELLOW: Color32 = Color32::from_rgb(229, 192, 123);
const ACTIVE_TAB: Color32 = Color32::from_rgb(38, 38, 38);
const HOVER: Color32 = Color32::from_rgb(26, 26, 26);
const CLOSE_HOVER: Color32 = Color32::from_rgb(58, 58, 58);
const SELECTED: Color32 = Color32::from_rgb(26, 36, 51);
const FAIL_BG: Color32 = Color32::from_rgb(18, 10, 11);
const FIELD: Color32 = Color32::from_rgb(0, 0, 0);
const CURSOR: Color32 = Color32::from_rgba_premultiplied(130, 134, 140, 150);
/// Selection in the fullscreen terminal: painted over the text.
const SELECTION: Color32 = Color32::from_rgba_premultiplied(36, 52, 84, 110);

/// 16-color ANSI palette (One Dark based), in code order.
const ANSI: [Color32; 16] = [
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

const TEXT_SIZE: f32 = 13.5;
const SMALL_SIZE: f32 = 12.0;
/// Window corner radius: that of macOS 26 windows with a toolbar.
#[cfg(target_os = "macos")]
const WINDOW_RADIUS: f64 = 24.0;
/// Tab bar: tall and same background, no separation from the rest.
const TITLEBAR_H: f32 = 52.0;
/// Default space for the macOS close/minimize/zoom buttons
/// (adjusted to the real position when it can be read).
const TRAFFIC_LIGHTS_W: f32 = 84.0;
/// Space for the close × at the end of each tab.
const TAB_CLOSE_W: f32 = 22.0;
const PROMPT_H: f32 = 106.0;
/// Lists above the prompt (commands, suggestions): max visible rows,
/// title height and row height.
const DOCK_ROWS: usize = 6;
const DOCK_HEAD_H: f32 = 30.0;
const DOCK_ROW_H: f32 = 26.0;
/// Wheel amount needed to move a list selection by one row.
const WHEEL_STEP: f32 = 24.0;
const PAD: f32 = 20.0;
/// Block: vertical padding, context line and gap before the output.
const BLOCK_PAD: f32 = 12.0;
const META_H: f32 = 18.0;
const OUT_GAP: f32 = 6.0;

fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

// ── Startup ──────────────────────────────────────────────────────────────────

pub fn run(store: Store) -> anyhow::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Cairn")
        .with_app_id("cairn")
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([560.0, 360.0])
        // macOS: no title bar; the tabs take its place.
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        // Transparent window: the layer clips the rounded corners.
        .with_transparent(cfg!(target_os = "macos"));
    // On macOS eframe puts this icon in the Dock: use the version with Apple's
    // grid margin so it doesn't look bigger than the others.
    let icon: &[u8] = if cfg!(target_os = "macos") {
        include_bytes!("../../assets/icons/icon-macos.png")
    } else {
        include_bytes!("../../assets/icons/icon.png")
    };
    let viewport = match eframe::icon_data::from_png_bytes(icon) {
        Ok(icon) => viewport.with_icon(icon),
        Err(_) => viewport,
    };
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let mut store = Some(store);
    eframe::run_native(
        "Cairn",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            setup_style(&cc.egui_ctx);
            let (tx, rx) = mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            // PTY threads wake the window when sending output.
            let sink: EventSink = Arc::new(move |ev| {
                let ok = tx.send(ev).is_ok();
                ctx.request_repaint();
                ok
            });
            let store = store.take().expect("the app is created only once");
            let size = ratatui::layout::Rect::new(0, 0, 120, 40);
            let mut app = App::new(store, sink, size);
            // From Finder the current directory is "/": $HOME is better.
            let cwd = std::env::current_dir()
                .ok()
                .filter(|p| p.as_os_str() != "/")
                .map(|p| p.to_string_lossy().into_owned());
            if !app.new_tab(None, cwd, None) {
                let msg = app.message.take().map(|(m, _)| m).unwrap_or_default();
                return Err(format!("could not open the first tab: {msg}").into());
            }
            Ok(Box::new(Gui {
                app,
                rx,
                title: String::new(),
                titlebar_drag: false,
                wheel: 0.0,
                lights_right: None,
                rounded: false,
                grid_sel: None,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// System monospace font (with bold) and fallbacks for symbols and CJK.
/// Cascadia Mono, bundled in the binary, if nothing better exists.
fn setup_fonts(ctx: &egui::Context) {
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

fn setup_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.visuals = egui::Visuals::dark();
        s.visuals.panel_fill = BG;
        s.visuals.window_fill = RAISED;
        s.visuals.extreme_bg_color = FIELD;
        s.visuals.override_text_color = Some(FG);
        s.visuals.selection.bg_fill = Color32::from_rgb(52, 74, 110);
        s.visuals.selection.stroke = Stroke::new(1.0, BRIGHT);
        s.spacing.item_spacing = Vec2::ZERO;
        s.interaction.selectable_labels = true;
        for font in s.text_styles.values_mut() {
            *font = mono(TEXT_SIZE);
        }
    });
}

// ── The application ──────────────────────────────────────────────────────────

struct Gui {
    app: App,
    rx: Receiver<AppEvent>,
    title: String,
    /// Dragging the window from the title bar.
    titlebar_drag: bool,
    /// Wheel accumulated over a list (in points), until a full step.
    wheel: f32,
    /// Right edge of the window buttons (macOS), in egui points.
    lights_right: Option<f32>,
    /// Rounded corners already applied to the window (macOS).
    rounded: bool,
    /// Text selected with the mouse in the fullscreen terminal.
    grid_sel: Option<GridSel>,
}

/// Selection in the fullscreen terminal grid, in `(row, column)` cells
/// of the visible screen.
#[derive(Clone, Copy)]
struct GridSel {
    tab: u64,
    anchor: (u16, u16),
    head: (u16, u16),
}

impl GridSel {
    /// Ordered ends: start inclusive, end exclusive.
    fn range(&self) -> ((u16, u16), (u16, u16)) {
        let (a, b) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        (a, (b.0, b.1 + 1))
    }
}

/// Size of a monospace grid cell.
#[derive(Clone, Copy)]
struct CellSize {
    w: f32,
    h: f32,
}

impl eframe::App for Gui {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // macOS: center the window buttons in the tab bar.
        #[cfg(target_os = "macos")]
        {
            if !self.rounded {
                self.rounded = macos::round_corners(frame, WINDOW_RADIUS);
            }
            let zoom = ctx.zoom_factor();
            self.lights_right =
                macos::center_traffic_lights(frame, TITLEBAR_H * zoom).map(|x| x as f32 / zoom);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = frame;

        // 1. PTY output and periodic tasks.
        while let Ok(ev) = self.rx.try_recv() {
            self.app.handle(ev);
        }
        self.app.tick();

        // 2. Keyboard: egui must not keep focus (the keyboard belongs to Cairn).
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        self.handle_input(&ctx);

        // 3. Closing the window with unsaved changes asks for confirmation.
        if ctx.input(|i| i.viewport().close_requested()) && !self.app.quit {
            self.app.command('q');
            if !self.app.quit {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        }
        if self.app.quit {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        let cell = cell_size(&ctx);
        self.draw(ui, cell);

        let title = self.app.window_title();
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        ctx.request_repaint_after(self.app.tick_rate());
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(BG).to_array()
    }
}

fn cell_size(ctx: &egui::Context) -> CellSize {
    let font = mono(TEXT_SIZE);
    let (w, h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    CellSize {
        w: w.max(1.0),
        h: (h + 3.0).round(),
    }
}

impl Gui {
    // ── Input ────────────────────────────────────────────────────────────────

    fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.grid_sel = None;
        self.app
            .handle(AppEvent::Term(Event::Key(KeyEvent::new(code, mods))));
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let (events, shift) = ctx.input(|i| (i.events.clone(), i.modifiers.shift));
        let mac = cfg!(target_os = "macos");
        for ev in events {
            match ev {
                egui::Event::Text(text) => {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.key(KeyCode::Char(c), KeyModifiers::NONE);
                    }
                }
                egui::Event::Paste(text) => {
                    self.grid_sel = None;
                    self.app.handle(AppEvent::Term(Event::Paste(text)));
                }
                // Cmd+C (Ctrl+Shift+C on Linux) with text selected in the
                // fullscreen terminal; in blocks egui copies.
                egui::Event::Copy if self.copy_grid_sel() => {}
                // On Linux Ctrl+C/Ctrl+X arrive as Copy/Cut: send to the terminal.
                egui::Event::Copy if !mac && !shift => {
                    self.key(KeyCode::Char('c'), KeyModifiers::CONTROL)
                }
                egui::Event::Cut if !mac && !shift => {
                    self.key(KeyCode::Char('x'), KeyModifiers::CONTROL)
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => self.key_event(key, modifiers),
                _ => {}
            }
        }
    }

    /// Copies the fullscreen terminal selection, if any.
    fn copy_grid_sel(&mut self) -> bool {
        let tab = self.app.tab();
        let Some(sel) = self
            .grid_sel
            .filter(|s| s.tab == tab.id && tab.fullscreen())
        else {
            return false;
        };
        let ((r0, c0), (r1, c1)) = sel.range();
        let text = tab.session.vt.screen().contents_between(r0, c0, r1, c1);
        if !text.is_empty()
            && let Err(e) = self.app.set_clipboard(&text)
        {
            self.app.flash(format!("could not copy: {e}"));
        }
        true
    }

    fn key_event(&mut self, key: Key, m: egui::Modifiers) {
        let app_mod = if cfg!(target_os = "macos") {
            m.mac_cmd
        } else {
            m.ctrl && m.shift
        };
        if app_mod {
            self.shortcut(key);
            return;
        }
        let mut mods = KeyModifiers::NONE;
        if m.shift {
            mods |= KeyModifiers::SHIFT;
        }
        if m.ctrl {
            mods |= KeyModifiers::CONTROL;
        }
        if m.alt && !cfg!(target_os = "macos") {
            mods |= KeyModifiers::ALT;
        }
        let code = match key {
            Key::Enter => KeyCode::Enter,
            Key::Tab if m.shift => KeyCode::BackTab,
            Key::Tab => KeyCode::Tab,
            Key::Backspace => KeyCode::Backspace,
            Key::Escape => KeyCode::Esc,
            Key::ArrowUp => KeyCode::Up,
            Key::ArrowDown => KeyCode::Down,
            Key::ArrowLeft => KeyCode::Left,
            Key::ArrowRight => KeyCode::Right,
            Key::Home => KeyCode::Home,
            Key::End => KeyCode::End,
            Key::PageUp => KeyCode::PageUp,
            Key::PageDown => KeyCode::PageDown,
            Key::Insert => KeyCode::Insert,
            Key::Delete => KeyCode::Delete,
            Key::F1 => KeyCode::F(1),
            Key::F2 => KeyCode::F(2),
            Key::F3 => KeyCode::F(3),
            Key::F4 => KeyCode::F(4),
            Key::F5 => KeyCode::F(5),
            Key::F6 => KeyCode::F(6),
            Key::F7 => KeyCode::F(7),
            Key::F8 => KeyCode::F(8),
            Key::F9 => KeyCode::F(9),
            Key::F10 => KeyCode::F(10),
            Key::F11 => KeyCode::F(11),
            Key::F12 => KeyCode::F(12),
            // Ctrl combinations (plain text arrives as Event::Text).
            _ if m.ctrl => match key_char(key) {
                Some(c) => KeyCode::Char(c),
                None => return,
            },
            _ => return,
        };
        self.key(code, mods);
    }

    /// App shortcuts: Cmd on macOS, Ctrl+Shift on Linux. Copy, paste
    /// and zoom are handled by egui.
    fn shortcut(&mut self, key: Key) {
        let Some(c) = key_char(key) else { return };
        match c {
            's' if self.app.editor_focused() => {
                self.key(KeyCode::Char('s'), KeyModifiers::CONTROL);
            }
            't' | 'w' | 'k' | 'q' | 's' | 'r' | 'e' | '1'..='9' => self.app.command(c),
            '[' => self.app.command('p'),
            ']' => self.app.command('n'),
            _ => {}
        }
    }

    // ── Drawing ──────────────────────────────────────────────────────────────

    fn draw(&mut self, ui: &mut Ui, cell: CellSize) {
        let ctx = ui.ctx().clone();
        let fullscreen_window = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));

        egui::Panel::top("titlebar")
            .exact_size(TITLEBAR_H)
            .show_separator_line(false)
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| self.draw_titlebar(ui, fullscreen_window));

        if self.app.tab().has_editor() {
            let width = ui.available_width() * 0.45;
            egui::Panel::right("editor")
                .exact_size(width)
                .show_separator_line(false)
                .resizable(false)
                .frame(Frame::new().fill(BG))
                .show(ui, |ui| self.draw_editor(ui, cell));
        }

        let fullscreen = self.app.tab().fullscreen();
        if !fullscreen {
            // The command or suggestion list sits above the prompt
            // and pushes the blocks up.
            let palette = self.app.palette();
            let dock_n = match &palette {
                Some(l) => l.len(),
                None => self.app.completion.as_ref().map_or(0, |c| c.items.len()),
            };
            let dock_h = if dock_n > 0 { dock_height(dock_n) } else { 0.0 };
            egui::Panel::bottom("prompt")
                .exact_size(PROMPT_H + dock_h)
                .show_separator_line(false)
                .frame(Frame::new().fill(BG))
                .show(ui, |ui| {
                    let rect = ui.max_rect();
                    let (top, bottom) = rect.split_top_bottom_at_y(rect.top() + dock_h);
                    if let Some(list) = &palette {
                        self.draw_palette(ui, top, list);
                    } else if dock_h > 0.0 {
                        self.draw_completion(ui, top);
                    }
                    ui.scope_builder(egui::UiBuilder::new().max_rect(bottom), |ui| {
                        self.draw_prompt(ui, cell)
                    });
                });
        }

        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| {
                if fullscreen {
                    self.draw_fullscreen(ui, cell);
                } else {
                    self.draw_blocks(ui, cell);
                }
            });

        if self.app.overlay.is_some() {
            dim_background(&ctx);
        }
        match self.app.overlay {
            Some(Overlay::Picker(_)) => self.draw_picker(&ctx),
            Some(Overlay::Form(_)) => self.draw_form(&ctx),
            None => {}
        }
        if self.app.prefix {
            draw_prefix_menu(&ctx);
        }
    }

    /// Mouse wheel over `rect` (a modal or a list): each step moves the
    /// selection one row. Accumulates fine trackpad deltas.
    fn wheel_over(&mut self, ctx: &egui::Context, rect: Rect) {
        let (delta, pointer) = ctx.input(|i| {
            let delta: f32 = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                        egui::MouseWheelUnit::Point => delta.y,
                        egui::MouseWheelUnit::Line => delta.y * WHEEL_STEP,
                        egui::MouseWheelUnit::Page => delta.y * WHEEL_STEP * DOCK_ROWS as f32,
                    }),
                    _ => None,
                })
                .sum();
            (delta, i.pointer.hover_pos())
        });
        if !pointer.is_some_and(|p| rect.contains(p)) {
            return;
        }
        self.wheel += delta;
        while self.wheel.abs() >= WHEEL_STEP {
            let down = self.wheel < 0.0;
            self.wheel -= WHEEL_STEP.copysign(self.wheel);
            self.app.scroll_list(down);
        }
    }

    fn draw_titlebar(&mut self, ui: &mut Ui, fullscreen_window: bool) {
        let rect = ui.max_rect();
        // Draggable background: moves the window; double click maximizes.
        let bg = ui.interact(rect, Id::new("titlebar-drag"), Sense::click_and_drag());
        if bg.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!max));
        } else if bg.drag_started() && !self.titlebar_drag {
            self.titlebar_drag = true;
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if !bg.dragged() {
            self.titlebar_drag = false;
        }
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0, BORDER),
        );

        let inset = if cfg!(target_os = "macos") && !fullscreen_window {
            TRAFFIC_LIGHTS_W
        } else {
            12.0
        };
        let mut x = rect.left() + inset;
        let cy = rect.center().y;

        let mut clicked = None;
        let mut close = None;
        for (i, tab) in self.app.tabs.iter().enumerate() {
            let active = i == self.app.active;
            let label = tab.title();
            let font = if active {
                bold(TEXT_SIZE)
            } else {
                mono(TEXT_SIZE)
            };
            let (r, pos, galley) = pill_rect(ui, x, cy, &label, font, 14.0, 30.0);
            // Space on the right for the close × (or the busy dot).
            let r = Rect::from_min_size(r.min, Vec2::new(r.width() + TAB_CLOSE_W, r.height()));
            if r.right() > rect.right() - 60.0 {
                break;
            }
            let resp = ui.interact(r, Id::new(("tab", tab.id)), Sense::click());
            let hovered = ui.rect_contains_pointer(r);
            let fill = if active {
                ACTIVE_TAB
            } else if hovered {
                HOVER
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(r, CornerRadius::same(8), fill);
            ui.painter()
                .galley(pos, galley, if active { TEXT2 } else { SUBTLE });

            let slot = Pos2::new(r.right() - TAB_CLOSE_W / 2.0 - 4.0, cy);
            if hovered || (active && !tab.busy()) {
                let x_rect = Rect::from_center_size(slot, Vec2::splat(18.0));
                let x_resp = ui.interact(x_rect, Id::new(("tab-close", tab.id)), Sense::click());
                if x_resp.hovered() {
                    ui.painter()
                        .rect_filled(x_rect, CornerRadius::same(4), CLOSE_HOVER);
                }
                let color = if x_resp.hovered() { TEXT2 } else { META };
                let d = 3.5;
                let stroke = Stroke::new(1.4, color);
                ui.painter()
                    .line_segment([slot + Vec2::new(-d, -d), slot + Vec2::new(d, d)], stroke);
                ui.painter()
                    .line_segment([slot + Vec2::new(-d, d), slot + Vec2::new(d, -d)], stroke);
                if x_resp.on_hover_text("Close tab (⌘W)").clicked() {
                    close = Some(i);
                }
            } else if tab.busy() {
                ui.painter().circle_filled(slot, 3.0, ACCENT);
            }
            if resp.middle_clicked() {
                close = Some(i);
            } else if resp.clicked() {
                clicked = Some(i);
            }
            x = r.right() + 6.0;
        }
        if let Some(i) = close {
            self.app.close_tab(i);
        } else if let Some(i) = clicked {
            self.app.active = i;
        }

        // New tab.
        let plus = Rect::from_center_size(Pos2::new(x + 15.0, cy), Vec2::splat(30.0));
        let resp = ui.interact(plus, Id::new("new-tab"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(plus, CornerRadius::same(8), HOVER);
        }
        ui.painter().text(
            plus.center(),
            Align2::CENTER_CENTER,
            "+",
            mono(15.0),
            SUBTLE,
        );
        if resp.on_hover_text("New tab (⌘T)").clicked() {
            self.app.command('t');
        }

        // Pending ⌃G prefix: a pill on the right.
        if self.app.prefix {
            let galley = ui
                .painter()
                .layout_no_wrap("⌃G".into(), bold(SMALL_SIZE), ON_ACCENT);
            let r = Rect::from_min_size(
                Pos2::new(rect.right() - 14.0 - galley.size().x - 16.0, cy - 11.0),
                Vec2::new(galley.size().x + 16.0, 22.0),
            );
            ui.painter().rect_filled(r, CornerRadius::same(6), ACCENT);
            ui.painter()
                .galley(r.center() - galley.size() / 2.0, galley, ON_ACCENT);
        }
    }

    fn draw_blocks(&mut self, ui: &mut Ui, cell: CellSize) {
        let area = ui.max_rect();
        let cols = (((area.width() - 2.0 * PAD) / cell.w).floor() as usize).max(10);
        let rows = ((area.height() / cell.h).floor() as u16).max(2);
        self.app.resize_active(rows, cols as u16);

        let tab = &mut self.app.tabs[self.app.active];
        if tab.blocks.is_empty() {
            if tab.state == CmdState::Starting {
                ui.painter().text(
                    area.center(),
                    Align2::CENTER_CENTER,
                    "starting shell…",
                    mono(TEXT_SIZE),
                    META,
                );
            }
            return;
        }

        let heights: Vec<f32> = tab
            .blocks
            .iter_mut()
            .map(|b| block_height(b, cols, cell))
            .collect();
        let total: f32 = heights.iter().sum();
        let busy = tab.busy();
        let last = tab.blocks.len() - 1;
        let blink = ((ui.input(|i| i.time) * 2.0) as u64).is_multiple_of(2);

        egui::ScrollArea::vertical()
            .id_salt(("blocks", tab.id))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                // Blocks stack from the bottom, like a log.
                let offset = (viewport.height() - total).max(0.0);
                ui.set_height(total + offset);
                let origin = ui.max_rect().min;
                let view = viewport.translate(origin.to_vec2());
                let mut y = offset;
                for (i, b) in tab.blocks.iter().enumerate() {
                    let h = heights[i];
                    if y + h >= viewport.min.y && y <= viewport.max.y {
                        let rect = Rect::from_min_size(
                            origin + Vec2::new(0.0, y),
                            Vec2::new(area.width(), h),
                        );
                        let place = BlockPlace {
                            rect,
                            cols,
                            cell,
                            view,
                        };
                        draw_block(ui, b, place, busy && i == last, blink);
                    }
                    y += h;
                }
            });
    }

    fn draw_prompt(&mut self, ui: &mut Ui, cell: CellSize) {
        let rect = ui.max_rect();
        let overlay_open = self.app.overlay.is_some() || self.app.prefix;
        let palette_open = self.app.palette().is_some();
        let completion_open = self.app.completion.is_some();
        let message = self.app.message.as_ref().map(|(m, _)| m.clone());
        let tab = &mut self.app.tabs[self.app.active];
        let focused = tab.focus == Focus::Terminal || !tab.has_editor();
        if ui
            .interact(rect, Id::new("prompt"), Sense::click())
            .clicked()
        {
            tab.focus = Focus::Terminal;
        }
        let p = ui.painter().clone();
        p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, RULE));

        // Chips: folder, branch, git changes · shell (or message).
        let cy = rect.top() + 12.0 + 11.0;
        let mut x = rect.left() + PAD;
        // The path gives way: branch, changes and shell always fit.
        let reserved = 140.0
            + tab
                .branch
                .as_ref()
                .map_or(0.0, |b| b.chars().count() as f32 * cell.w + 40.0)
            + if tab.git.is_some_and(|g| g.files > 0) {
                150.0
            } else {
                0.0
            };
        let max_chars = ((rect.width() - 2.0 * PAD - reserved - 40.0) / cell.w).max(8.0) as usize;
        if let Some(host) = &tab.remote {
            x = chip(&p, x, cy, Icon::Remote, host, ACCENT, true) + 8.0;
        }
        let local_path = if tab.remote.is_some() {
            tab.cwd.clone()
        } else {
            short_path(&tab.cwd)
        };
        let path = shorten_path(&local_path, max_chars);
        x = chip(&p, x, cy, Icon::Folder, &path, TEXT2, true) + 8.0;
        if let Some(br) = &tab.branch {
            x = chip(&p, x, cy, Icon::Branch, br, GREEN, true) + 8.0;
        }
        if let Some(g) = tab.git.filter(|g| g.files > 0) {
            diff_chip(&p, x, cy, g);
        }
        let (right, color) = match message {
            Some(m) => (m, YELLOW),
            None => (tab.session.shell_label.clone(), DIM),
        };
        p.text(
            Pos2::new(rect.right() - PAD, cy),
            Align2::RIGHT_CENTER,
            right,
            mono(SMALL_SIZE),
            color,
        );

        // Input line.
        let in_y = rect.top() + 12.0 + 22.0 + 10.0 + 10.0;
        let tx = rect.left() + PAD;
        let cols = (((rect.right() - PAD - tx) / cell.w) as usize).max(4);
        let busy = tab.busy();
        let cursor_at = |cx: u16| {
            Rect::from_min_size(
                Pos2::new(tx + cx as f32 * cell.w, in_y - 9.0),
                Vec2::new(8.0, 18.0),
            )
        };
        if busy {
            p.text(
                Pos2::new(tx, in_y),
                Align2::LEFT_CENTER,
                "Running · keyboard goes to the program",
                mono(TEXT_SIZE),
                META,
            );
        } else if tab.state == CmdState::Starting && tab.input.text.is_empty() {
            p.text(
                Pos2::new(tx, in_y),
                Align2::LEFT_CENTER,
                "starting…",
                mono(TEXT_SIZE),
                META,
            );
        } else {
            let (text, cx) = tab.input.view(cols);
            let show_cursor = focused && !overlay_open;
            if text.is_empty() {
                let x0 = if show_cursor { tx + 12.0 } else { tx };
                p.text(
                    Pos2::new(x0, in_y),
                    Align2::LEFT_CENTER,
                    "Type a command…",
                    mono(TEXT_SIZE),
                    META,
                );
            } else {
                p.text(
                    Pos2::new(tx, in_y),
                    Align2::LEFT_CENTER,
                    text,
                    mono(TEXT_SIZE),
                    TEXT2,
                );
            }
            if show_cursor {
                p.rect_filled(cursor_at(cx), CornerRadius::ZERO, ACCENT);
            }
        }

        // Shortcuts depending on state.
        // With a list open, the footer stays clean.
        let update = self.app.update_available.as_ref().map(|v| format!("v{v}"));
        let mut hints: Vec<(&str, &str)> = if completion_open || palette_open {
            vec![]
        } else if busy {
            vec![("⌃C", "interrupt"), ("⌃G", "menu")]
        } else if tab.has_editor() && tab.input.text.is_empty() {
            vec![("Tab", "focus editor"), ("/help", "commands")]
        } else {
            vec![("/help", "commands"), ("/snippets", ""), ("/profiles", "")]
        };
        // A newer release, until it is installed: next to the rest.
        if let Some(v) = &update
            && !hints.is_empty()
            && !busy
        {
            hints.push(("/update", v));
        }
        let hy = in_y + 10.0 + 10.0 + 8.0;
        paint_hints(&p, Pos2::new(tx, hy), &hints);
    }

    /// `Tab` suggestions, above the prompt with the same style as the
    /// command list; folders in blue.
    fn draw_completion(&mut self, ui: &mut Ui, rect: Rect) {
        let Some(c) = self.app.completion.as_ref() else {
            return;
        };
        let n = c.items.len();
        let rows = n.min(DOCK_ROWS);
        let start = c.selected.map_or(0, |s| s.saturating_sub(rows - 1));
        let p = ui.painter().clone();
        let left = rect.left() + PAD;
        let info = format!("{n} {}", if n == 1 { "match" } else { "matches" });
        paint_dock_head(&p, rect, "Suggestions", &info);

        let mut clicked = None;
        for (row, (i, item)) in c
            .items
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .enumerate()
        {
            let r = Rect::from_min_size(
                Pos2::new(
                    rect.left(),
                    rect.top() + DOCK_HEAD_H + row as f32 * DOCK_ROW_H,
                ),
                Vec2::new(rect.width(), DOCK_ROW_H),
            );
            let resp = ui.interact(r, Id::new(("completion-row", i)), Sense::click());
            let selected = c.selected == Some(i);
            let cy = r.center().y;
            if selected {
                p.text(
                    Pos2::new(left, cy),
                    Align2::LEFT_CENTER,
                    "❯",
                    bold(TEXT_SIZE),
                    ACCENT,
                );
            }
            let (font, color) = if item.dir {
                (bold(TEXT_SIZE), ANSI[12])
            } else if selected {
                (mono(TEXT_SIZE), TEXT2)
            } else if resp.hovered() {
                (mono(TEXT_SIZE), FG)
            } else {
                (mono(TEXT_SIZE), SUBTLE)
            };
            let mut job = LayoutJob::single_section(
                item.label.clone(),
                TextFormat {
                    font_id: font,
                    color,
                    ..Default::default()
                },
            );
            job.wrap.max_width = r.width() - 2.0 * PAD - 18.0;
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let g = p.layout_job(job);
            p.galley(Pos2::new(left + 18.0, cy - g.size().y / 2.0), g, color);
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        paint_dock_scrollbar(&p, rect, n, rows, start);
        if let Some(i) = clicked {
            self.app.pick_completion(i);
        } else {
            self.wheel_over(ui.ctx(), rect);
        }
    }

    /// Commands matching the input, right above the prompt (moon style):
    /// title, no surface of its own, `❯` on the selected one and a scrollbar
    /// on the right if they don't all fit.
    fn draw_palette(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        list: &[&'static crate::app::SlashCommand],
    ) {
        let n = list.len();
        let rows = n.min(DOCK_ROWS);
        let sel = self.app.palette_sel.min(n - 1);
        let start = sel.saturating_sub(rows - 1);
        // What's typed after `/`: in accent; the rest is what would be completed.
        let typed = self.app.typing_command().unwrap_or("").chars().count();
        let p = ui.painter().clone();
        let left = rect.left() + PAD;

        let info = format!("{n} {}", if n == 1 { "command" } else { "commands" });
        paint_dock_head(&p, rect, "Commands", &info);

        // Description column: the same for all commands.
        let name_w = crate::app::COMMANDS
            .iter()
            .map(|c| {
                let label = format!("/{} {}", c.name, c.args);
                p.layout_no_wrap(label, mono(TEXT_SIZE), FG).size().x
            })
            .fold(0.0, f32::max);
        let desc_x = left + 18.0 + name_w + 24.0;

        let mut clicked = None;
        for (row, (i, c)) in list.iter().enumerate().skip(start).take(rows).enumerate() {
            let top = rect.top() + DOCK_HEAD_H + row as f32 * DOCK_ROW_H;
            let r = Rect::from_min_size(
                Pos2::new(rect.left(), top),
                Vec2::new(rect.width(), DOCK_ROW_H),
            );
            let resp = ui.interact(r, Id::new(("palette-row", i)), Sense::click());
            let selected = i == sel;
            let cy = r.center().y;
            if selected {
                p.text(
                    Pos2::new(left, cy),
                    Align2::LEFT_CENTER,
                    "❯",
                    bold(TEXT_SIZE),
                    ACCENT,
                );
            }
            let cut = c
                .name
                .char_indices()
                .nth(typed)
                .map_or(c.name.len(), |(b, _)| b);
            let (head, tail) = c.name.split_at(cut);
            let mut x = left + 18.0;
            x = p
                .text(
                    Pos2::new(x, cy),
                    Align2::LEFT_CENTER,
                    format!("/{head}"),
                    bold(TEXT_SIZE),
                    ACCENT,
                )
                .right();
            x = p
                .text(
                    Pos2::new(x, cy),
                    Align2::LEFT_CENTER,
                    tail,
                    mono(TEXT_SIZE),
                    if selected { SUBTLE } else { META },
                )
                .right();
            if !c.args.is_empty() {
                p.text(
                    Pos2::new(x + 8.0, cy),
                    Align2::LEFT_CENTER,
                    c.args,
                    mono(TEXT_SIZE),
                    SUBTLE,
                );
            }
            let desc_color = if selected {
                TEXT2
            } else if resp.hovered() {
                FG
            } else {
                META
            };
            p.text(
                Pos2::new(desc_x, cy),
                Align2::LEFT_CENTER,
                c.desc,
                mono(TEXT_SIZE),
                desc_color,
            );
            if resp.clicked() {
                clicked = Some(i);
            }
        }

        paint_dock_scrollbar(&p, rect, n, rows, start);

        if let Some(i) = clicked {
            self.app.palette_sel = i;
            self.key(KeyCode::Enter, KeyModifiers::NONE);
        } else {
            self.wheel_over(ui.ctx(), rect);
        }
    }

    fn draw_fullscreen(&mut self, ui: &mut Ui, cell: CellSize) {
        // No bar: the program fills the whole area below the tabs.
        let area = ui.max_rect();
        let live = Rect::from_min_max(
            Pos2::new(area.left() + PAD / 2.0, area.top() + 6.0),
            Pos2::new(area.right() - PAD / 2.0, area.bottom()),
        );
        let cols = ((live.width() / cell.w).floor() as u16).max(10);
        let rows = ((live.height() / cell.h).floor() as u16).max(2);
        self.app.resize_active(rows, cols);

        let tab = &mut self.app.tabs[self.app.active];
        let p = ui.painter().clone();
        let scrolled = tab.session.vt.screen().scrollback();

        // Mouse: to the program if it asked for it; otherwise the wheel sends arrows.
        // With Shift (or Option on macOS) it selects even if the program asked.
        let force = ui.input(|i| i.modifiers.shift || i.modifiers.alt);
        let select =
            force || tab.session.vt.screen().mouse_protocol_mode() == MouseProtocolMode::None;
        forward_mouse(ui, tab, live, cell, !select);

        let screen = tab.session.vt.screen();
        let (vrows, vcols) = screen.size();

        // Mouse selection: drag selects, a click clears it.
        if self.grid_sel.is_some_and(|s| s.tab != tab.id) {
            self.grid_sel = None;
        }
        let resp = ui.interact(live, Id::new("fullscreen-select"), Sense::drag());
        if select {
            let to_cell = |pos: Pos2| {
                let r = ((pos.y - live.top()) / cell.h).clamp(0.0, (vrows - 1) as f32);
                let c = ((pos.x - live.left()) / cell.w).clamp(0.0, (vcols - 1) as f32);
                (r as u16, c as u16)
            };
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            if ui.input(|i| i.pointer.primary_pressed()) && resp.hovered() {
                self.grid_sel = None;
            }
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(at) = ui.input(|i| i.pointer.press_origin()).map(to_cell) {
                    self.grid_sel = Some(GridSel {
                        tab: tab.id,
                        anchor: at,
                        head: at,
                    });
                }
            } else if resp.dragged_by(egui::PointerButton::Primary)
                && let (Some(s), Some(pos)) = (&mut self.grid_sel, resp.interact_pointer_pos())
            {
                s.head = to_cell(pos);
            }
        }
        for r in 0..vrows {
            let y = live.top() + r as f32 * cell.h;
            let mut c = 0;
            while c < vcols {
                let Some(first) = screen.cell(r, c) else {
                    break;
                };
                // Group consecutive cells with the same style.
                let key = vt_style_key(first);
                let start = c;
                let mut text = String::new();
                while c < vcols {
                    let Some(cl) = screen.cell(r, c) else { break };
                    if vt_style_key(cl) != key {
                        break;
                    }
                    if !cl.is_wide_continuation() {
                        let s = cl.contents();
                        text.push_str(if s.is_empty() { " " } else { s });
                    }
                    c += 1;
                }
                let (fg, bg, b, it, ul) = vt_colors(first);
                let x = live.left() + start as f32 * cell.w;
                let run = Rect::from_min_size(
                    Pos2::new(x, y),
                    Vec2::new((c - start) as f32 * cell.w, cell.h),
                );
                if bg != BG {
                    p.rect_filled(run, CornerRadius::ZERO, bg);
                }
                if !text.trim().is_empty() {
                    let job = LayoutJob::single_section(
                        text,
                        TextFormat {
                            font_id: if b { bold(TEXT_SIZE) } else { mono(TEXT_SIZE) },
                            color: fg,
                            italics: it,
                            underline: if ul {
                                Stroke::new(1.0, fg)
                            } else {
                                Stroke::NONE
                            },
                            ..Default::default()
                        },
                    );
                    let g = p.layout_job(job);
                    p.galley(Pos2::new(x, y + 1.5), g, fg);
                }
            }
        }
        if !screen.hide_cursor() && scrolled == 0 {
            let (cr, cc) = screen.cursor_position();
            let r = Rect::from_min_size(
                Pos2::new(
                    live.left() + cc as f32 * cell.w,
                    live.top() + cr as f32 * cell.h,
                ),
                Vec2::new(cell.w, cell.h),
            );
            p.rect_filled(r, CornerRadius::ZERO, CURSOR);
        }

        if let Some(sel) = self.grid_sel {
            let ((r0, c0), (r1, c1)) = sel.range();
            for r in r0..=r1.min(vrows - 1) {
                let from = if r == r0 { c0 } else { 0 };
                let to = if r == r1 { c1 } else { vcols }.min(vcols);
                if to > from {
                    let rect = Rect::from_min_size(
                        Pos2::new(
                            live.left() + from as f32 * cell.w,
                            live.top() + r as f32 * cell.h,
                        ),
                        Vec2::new((to - from) as f32 * cell.w, cell.h),
                    );
                    p.rect_filled(rect, CornerRadius::ZERO, SELECTION);
                }
            }
        }

        // Only while viewing scrollback: a notice at the top right.
        if scrolled > 0 {
            let galley = p.layout_no_wrap(
                format!("history ↑{scrolled} · type to return"),
                mono(SMALL_SIZE),
                YELLOW,
            );
            let r = Rect::from_min_size(
                Pos2::new(
                    area.right() - PAD - galley.size().x - 20.0,
                    area.top() + 10.0,
                ),
                galley.size() + Vec2::new(20.0, 10.0),
            );
            p.rect_filled(r, CornerRadius::same(6), RAISED);
            p.galley(r.center() - galley.size() / 2.0, galley, YELLOW);
        }
    }

    fn draw_editor(&mut self, ui: &mut Ui, cell: CellSize) {
        let overlay_open = self.app.overlay.is_some();
        let tab = &mut self.app.tabs[self.app.active];
        let focused = tab.focus == Focus::Editor;
        let outer = ui.max_rect();
        if ui
            .interact(outer, Id::new("editor-focus"), Sense::click())
            .clicked()
        {
            tab.focus = Focus::Editor;
        }
        let Some(editor) = tab.editor.as_mut() else {
            return;
        };

        let rect = outer;
        let p = ui.painter().clone();
        p.vline(rect.left() + 0.5, rect.y_range(), Stroke::new(1.0, BORDER));
        let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 34.0));
        let footer = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 28.0), rect.max);
        let body = Rect::from_min_max(
            Pos2::new(rect.left(), header.bottom()),
            Pos2::new(rect.right(), footer.top()),
        );
        p.rect_filled(header, CornerRadius::ZERO, TITLEBAR);
        p.hline(
            header.x_range(),
            header.bottom() - 0.5,
            Stroke::new(1.0, if focused { ACCENT } else { BORDER }),
        );
        p.hline(
            footer.x_range(),
            footer.top() + 0.5,
            Stroke::new(1.0, BORDER),
        );

        let name = editor
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let r = p.text(
            Pos2::new(header.left() + 12.0, header.center().y),
            Align2::LEFT_CENTER,
            format!("✎ {name}"),
            bold(TEXT_SIZE),
            BRIGHT,
        );
        if editor.dirty {
            p.circle_filled(Pos2::new(r.right() + 10.0, header.center().y), 3.5, YELLOW);
        }

        // Code / Preview toggle.
        if editor.is_markdown {
            let seg_w: f32 = ["Preview", "Code"]
                .iter()
                .map(|l| {
                    p.layout_no_wrap((*l).into(), mono(SMALL_SIZE), SUBTLE)
                        .size()
                        .x
                        + 20.0
                })
                .sum::<f32>()
                + 6.0;
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(header.right() - 10.0 - seg_w, header.center().y - 13.0),
                    Vec2::new(seg_w + 4.0, 26.0),
                ),
                CornerRadius::same(6),
                BG,
            );
            let mut x = header.right() - 10.0;
            for (label, preview) in [("Preview", true), ("Code", false)] {
                let g = p.layout_no_wrap(label.into(), mono(SMALL_SIZE), SUBTLE);
                let w = g.size().x + 20.0;
                let r = Rect::from_min_size(
                    Pos2::new(x - w, header.center().y - 10.0),
                    Vec2::new(w, 20.0),
                );
                let on = editor.preview == preview;
                if on {
                    p.rect_filled(r, CornerRadius::same(4), ACTIVE_TAB);
                }
                p.galley(
                    r.center() - g.size() / 2.0,
                    g,
                    if on { BRIGHT } else { SUBTLE },
                );
                if ui
                    .interact(r, Id::new(("seg", label)), Sense::click())
                    .clicked()
                {
                    editor.preview = preview;
                }
                x -= w + 2.0;
            }
        }

        // Shortcut footer.
        let mut hints = vec![("⌘S", "save")];
        if editor.is_markdown {
            hints.push(("⌃P", if editor.preview { "code" } else { "preview" }));
        }
        hints.push(("⌃X", "close"));
        hints.push(("Esc", "terminal"));
        let foot = Pos2::new(footer.left() + 12.0, footer.center().y);
        match &editor.status {
            Some(s) => {
                p.text(foot, Align2::LEFT_CENTER, s, mono(SMALL_SIZE), YELLOW);
            }
            None => paint_hints(&p, foot, &hints),
        }

        if editor.preview {
            let inner = body.shrink2(Vec2::new(18.0, 12.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            egui::ScrollArea::vertical()
                .id_salt("md-preview")
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    for line in markdown::render(&editor.content()) {
                        ui.add(Label::new(markdown_job(&line, inner.width())).selectable(true));
                    }
                });
            return;
        }

        // Code: line numbers + text + cursor.
        let num_w = editor.lines.len().to_string().len().max(3) as f32 * cell.w + 16.0;
        let text_x = body.left() + 10.0 + num_w;
        let rows = ((body.height() - 16.0) / cell.h).floor().max(1.0) as usize;
        let cols = ((body.right() - 12.0 - text_x) / cell.w).floor().max(4.0) as usize;
        editor.scroll_into_view(rows, cols);
        for (i, line) in editor.lines.iter().enumerate().skip(editor.top).take(rows) {
            let y = body.top() + 8.0 + (i - editor.top) as f32 * cell.h + cell.h / 2.0;
            p.text(
                Pos2::new(text_x - 14.0, y),
                Align2::RIGHT_CENTER,
                (i + 1).to_string(),
                mono(SMALL_SIZE),
                if i == editor.cy { ACCENT } else { META },
            );
            let shown: String = line.chars().skip(editor.left).take(cols).collect();
            p.text(
                Pos2::new(text_x, y),
                Align2::LEFT_CENTER,
                shown,
                mono(TEXT_SIZE),
                FG,
            );
        }
        if focused && !overlay_open {
            let cx = editor.cursor_col().saturating_sub(editor.left) as f32;
            let cy = (editor.cy - editor.top) as f32;
            let c = Rect::from_min_size(
                Pos2::new(text_x + cx * cell.w, body.top() + 8.0 + cy * cell.h + 1.0),
                Vec2::new(2.0, cell.h - 2.0),
            );
            p.rect_filled(c, CornerRadius::same(1), ACCENT);
        }
    }

    fn draw_picker(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let size = Vec2::new(
            (screen.width() * 0.7).clamp(420.0, 860.0),
            (screen.height() * 0.62).clamp(260.0, 560.0),
        );
        let mut clicked_row = None;
        let mut area = Rect::NOTHING;
        let Some(Overlay::Picker(p)) = self.app.overlay.as_mut() else {
            return;
        };
        overlay_area(
            ctx,
            "picker",
            Align2::CENTER_CENTER,
            Vec2::ZERO,
            size,
            |ui| {
                area = Rect::from_min_size(ui.max_rect().min, size);
                clicked_row = picker_contents(ui, p, size);
            },
        );
        self.wheel_over(ctx, area);
        if let Some(row) = clicked_row {
            if let Some(Overlay::Picker(p)) = self.app.overlay.as_mut() {
                p.selected = row;
            }
            self.key(KeyCode::Enter, KeyModifiers::NONE);
        }
    }

    fn draw_form(&mut self, ctx: &egui::Context) {
        let Some(Overlay::Form(f)) = self.app.overlay.as_ref() else {
            return;
        };
        let size = Vec2::new(560.0, 110.0 + f.fields.len() as f32 * 58.0);
        overlay_area(ctx, "form", Align2::CENTER_CENTER, Vec2::ZERO, size, |ui| {
            form_contents(ui, f, size);
        });
    }
}

// ── Drawing pieces ───────────────────────────────────────────────────────────

fn key_char(key: Key) -> Option<char> {
    let name = key.symbol_or_name();
    let mut chars = name.chars();
    let c = chars.next()?;
    chars.next().is_none().then(|| c.to_ascii_lowercase())
}

/// Height of a list above the prompt with `n` items.
fn dock_height(n: usize) -> f32 {
    DOCK_HEAD_H + n.min(DOCK_ROWS) as f32 * DOCK_ROW_H + 6.0
}

/// Title of a list above the prompt, with info on the right.
fn paint_dock_head(p: &egui::Painter, rect: Rect, title: &str, info: &str) {
    let y = rect.top() + DOCK_HEAD_H / 2.0 + 2.0;
    p.text(
        Pos2::new(rect.left() + PAD, y),
        Align2::LEFT_CENTER,
        title,
        bold(SMALL_SIZE),
        TEXT2,
    );
    p.text(
        Pos2::new(rect.right() - PAD, y),
        Align2::RIGHT_CENTER,
        info,
        mono(SMALL_SIZE),
        META,
    );
}

/// Scrollbar of a list above the prompt, if not all items fit.
fn paint_dock_scrollbar(p: &egui::Painter, rect: Rect, n: usize, rows: usize, start: usize) {
    if n <= rows {
        return;
    }
    let track = Rect::from_min_size(
        Pos2::new(rect.right() - PAD / 2.0 - 2.0, rect.top() + DOCK_HEAD_H),
        Vec2::new(3.0, rows as f32 * DOCK_ROW_H),
    );
    paint_scrollbar(p, track, n, rows, start);
}

/// Thin scrollbar in `track`: `rows` of `n` rows visible,
/// starting at row `start`.
fn paint_scrollbar(p: &egui::Painter, track: Rect, n: usize, rows: usize, start: usize) {
    p.rect_filled(track, CornerRadius::same(2), BORDER2);
    let h = (track.height() * rows as f32 / n as f32).max(12.0);
    let y = track.top() + (track.height() - h) * start as f32 / (n - rows).max(1) as f32;
    let thumb = Rect::from_min_size(Pos2::new(track.left(), y), Vec2::new(track.width(), h));
    p.rect_filled(thumb, CornerRadius::same(2), META);
}

/// Places a widget in `rect`, left-aligned and vertically centered.
fn put_left(ui: &mut Ui, rect: Rect, widget: Label) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| ui.add(widget),
    );
}

/// Rect of a text pill centered on `cy`, starting at `x`.
fn pill_rect(
    ui: &Ui,
    x: f32,
    cy: f32,
    text: &str,
    font: FontId,
    pad: f32,
    h: f32,
) -> (Rect, Pos2, Arc<egui::Galley>) {
    let galley = ui.painter().layout_no_wrap(text.into(), font, FG);
    let r = Rect::from_min_size(
        Pos2::new(x, cy - h / 2.0),
        Vec2::new(galley.size().x + 2.0 * pad, h),
    );
    let pos = Pos2::new(r.left() + pad, cy - galley.size().y / 2.0);
    (r, pos, galley)
}
/// Shortcuts: each key in its box and the dimmed action next to it.
fn paint_hints(p: &egui::Painter, mut pos: Pos2, hints: &[(&str, &str)]) {
    for (key, action) in hints {
        for cap in keycaps(key) {
            let g = p.layout_no_wrap(cap, mono(SMALL_SIZE), TEXT2);
            let w = (g.size().x + 10.0).max(18.0);
            let r = Rect::from_center_size(Pos2::new(pos.x + w / 2.0, pos.y), Vec2::new(w, 18.0));
            p.rect_filled(r, CornerRadius::same(4), KEYCAP);
            p.galley(r.center() - g.size() / 2.0, g, TEXT2);
            pos.x = r.right() + 4.0;
        }
        let r = p.text(
            Pos2::new(pos.x + 3.0, pos.y),
            Align2::LEFT_CENTER,
            *action,
            mono(SMALL_SIZE),
            META,
        );
        pos.x = r.right() + 16.0;
    }
}

/// Shortens a path from the left: `…/scratchpad/demo-repo`.
fn shorten_path(path: &str, max_chars: usize) -> String {
    if path.chars().count() <= max_chars {
        return path.to_string();
    }
    let mut out = String::new();
    for seg in path.rsplit('/') {
        let candidate = if out.is_empty() {
            seg.to_string()
        } else {
            format!("{seg}/{out}")
        };
        if candidate.chars().count() + 2 > max_chars {
            break;
        }
        out = candidate;
    }
    if out.is_empty() {
        let tail: String = path
            .chars()
            .rev()
            .take(max_chars.saturating_sub(1))
            .collect();
        return format!("…{}", tail.chars().rev().collect::<String>());
    }
    format!("…/{out}")
}

#[derive(Clone, Copy)]
enum Icon {
    Folder,
    Branch,
    File,
    Remote,
}

/// 12 px stroke icon (same as the mockup), with its corner at `o`.
fn paint_icon(p: &egui::Painter, o: Pos2, icon: Icon, color: Color32) {
    let k = 12.0 / 16.0;
    let pt = |x: f32, y: f32| o + Vec2::new(x * k, y * k);
    let stroke = Stroke::new(1.2, color);
    match icon {
        Icon::Folder => {
            let pts = vec![
                pt(2.0, 3.5),
                pt(6.5, 3.5),
                pt(8.0, 5.5),
                pt(14.0, 5.5),
                pt(14.0, 12.5),
                pt(2.0, 12.5),
            ];
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        Icon::Branch => {
            for (cx, cy) in [(5.0, 3.5), (5.0, 12.5), (11.0, 5.0)] {
                p.circle_stroke(pt(cx, cy), 1.5 * k, stroke);
            }
            p.line_segment([pt(5.0, 5.0), pt(5.0, 11.0)], stroke);
            p.add(egui::Shape::line(
                vec![pt(11.0, 6.5), pt(10.0, 9.0), pt(5.5, 11.0)],
                stroke,
            ));
        }
        Icon::Remote => {
            let pts = vec![pt(2.0, 3.0), pt(14.0, 3.0), pt(14.0, 11.0), pt(2.0, 11.0)];
            p.add(egui::Shape::closed_line(pts, stroke));
            p.line_segment([pt(8.0, 11.0), pt(8.0, 13.5)], stroke);
            p.line_segment([pt(5.0, 13.5), pt(11.0, 13.5)], stroke);
        }
        Icon::File => {
            let pts = vec![
                pt(4.0, 2.0),
                pt(9.5, 2.0),
                pt(12.5, 5.0),
                pt(12.5, 14.0),
                pt(4.0, 14.0),
            ];
            p.add(egui::Shape::closed_line(pts, stroke));
            p.add(egui::Shape::line(
                vec![pt(9.5, 2.0), pt(9.5, 5.0), pt(12.5, 5.0)],
                stroke,
            ));
        }
    }
}

/// Bordered chip: icon + text. Returns its right edge.
fn chip(
    p: &egui::Painter,
    x: f32,
    cy: f32,
    icon: Icon,
    text: &str,
    color: Color32,
    strong: bool,
) -> f32 {
    let font = if strong {
        bold(SMALL_SIZE)
    } else {
        mono(SMALL_SIZE)
    };
    let g = p.layout_no_wrap(text.to_string(), font, color);
    let w = 7.0 + 12.0 + 6.0 + g.size().x + 7.0;
    let r = Rect::from_min_size(Pos2::new(x, cy - 11.0), Vec2::new(w, 22.0));
    p.rect(
        r,
        CornerRadius::same(5),
        CHIP_BG,
        Stroke::new(1.0, RULE),
        egui::StrokeKind::Inside,
    );
    paint_icon(p, Pos2::new(x + 7.0, cy - 6.0), icon, color);
    p.galley(Pos2::new(x + 25.0, cy - g.size().y / 2.0), g, color);
    r.right()
}

/// Changes chip: `3 • +48 -12` with numbers in green and red.
fn diff_chip(p: &egui::Painter, x: f32, cy: f32, g: crate::git::DiffStat) -> f32 {
    let mut job = LayoutJob::default();
    let f = |color, strong: bool| TextFormat {
        font_id: if strong {
            bold(SMALL_SIZE)
        } else {
            mono(SMALL_SIZE)
        },
        color,
        ..Default::default()
    };
    job.append(&g.files.to_string(), 0.0, f(CHIP_TEXT, false));
    job.append("•", 6.0, f(DIM, false));
    job.append(&format!("+{}", g.insertions), 6.0, f(GREEN, true));
    job.append(&format!("-{}", g.deletions), 6.0, f(RED, true));
    let gal = p.layout_job(job);
    let w = 7.0 + 12.0 + 6.0 + gal.size().x + 7.0;
    let r = Rect::from_min_size(Pos2::new(x, cy - 11.0), Vec2::new(w, 22.0));
    p.rect(
        r,
        CornerRadius::same(5),
        CHIP_BG,
        Stroke::new(1.0, RULE),
        egui::StrokeKind::Inside,
    );
    paint_icon(p, Pos2::new(x + 7.0, cy - 6.0), Icon::File, CHIP_TEXT);
    p.galley(Pos2::new(x + 25.0, cy - gal.size().y / 2.0), gal, CHIP_TEXT);
    r.right()
}

fn dim_background(ctx: &egui::Context) {
    let rect = ctx.content_rect();
    ctx.layer_painter(LayerId::new(Order::Middle, Id::new("dim")))
        .rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(153));
}

/// Overlay window: accent border, rounded corners and shadow.
fn overlay_area(
    ctx: &egui::Context,
    id: &str,
    anchor: Align2,
    offset: Vec2,
    size: Vec2,
    add: impl FnOnce(&mut Ui),
) {
    egui::Area::new(Id::new(id))
        .order(Order::Foreground)
        .anchor(anchor, offset)
        .show(ctx, |ui| {
            Frame::new()
                .fill(RAISED)
                .stroke(Stroke::new(1.0, ACCENT))
                .corner_radius(CornerRadius::same(12))
                .shadow(Shadow {
                    offset: [0, 18],
                    blur: 48,
                    spread: 0,
                    color: Color32::from_black_alpha(140),
                })
                .show(ui, |ui| {
                    ui.set_min_size(size);
                    ui.set_max_size(size);
                    add(ui);
                });
        });
}

/// Picker contents. Returns the (visible) row clicked with the mouse.
fn picker_contents(ui: &mut Ui, p: &mut Picker, size: Vec2) -> Option<usize> {
    let rect = Rect::from_min_size(ui.max_rect().min, size);
    let painter = ui.painter().clone();

    // Header: title + filter field.
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 56.0));
    let r = painter.text(
        Pos2::new(header.left() + 18.0, header.center().y),
        Align2::LEFT_CENTER,
        p.kind.title(),
        bold(TEXT_SIZE),
        ACCENT,
    );
    let field = Rect::from_min_max(
        Pos2::new(r.right() + 14.0, header.center().y - 16.0),
        Pos2::new(header.right() - 16.0, header.center().y + 16.0),
    );
    painter.rect(
        field,
        CornerRadius::same(6),
        FIELD,
        Stroke::new(1.0, FIELD_BORDER),
        egui::StrokeKind::Inside,
    );
    let cell_w = ui.fonts_mut(|f| f.glyph_width(&mono(TEXT_SIZE), 'M'));
    let cols = ((field.width() - 20.0) / cell_w) as usize;
    let (text, cx) = p.filter.view(cols.max(4));
    let tpos = Pos2::new(field.left() + 10.0, field.center().y);
    painter.text(tpos, Align2::LEFT_CENTER, text, mono(TEXT_SIZE), BRIGHT);
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(tpos.x + cx as f32 * cell_w, field.center().y - 8.0),
            Vec2::new(2.0, 16.0),
        ),
        CornerRadius::same(1),
        ACCENT,
    );
    painter.hline(header.x_range(), header.bottom(), Stroke::new(1.0, BORDER2));

    // Footer.
    let footer = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 38.0), rect.max);
    painter.hline(footer.x_range(), footer.top(), Stroke::new(1.0, BORDER2));
    let foot = Pos2::new(footer.left() + 18.0, footer.center().y);
    if p.confirm {
        painter.text(
            foot,
            Align2::LEFT_CENTER,
            "⌃K again to clear history",
            mono(SMALL_SIZE),
            YELLOW,
        );
    } else {
        paint_hints(&painter, foot, p.kind.hints());
    }

    // List.
    let list = Rect::from_min_max(
        Pos2::new(rect.left() + 8.0, header.bottom() + 8.0),
        Pos2::new(rect.right() - 8.0, footer.top() - 8.0),
    );
    let visible = p.visible();
    if visible.is_empty() {
        let msg = if p.items.is_empty() {
            "empty"
        } else {
            "no matches"
        };
        painter.text(
            Pos2::new(list.left() + 12.0, list.top() + 18.0),
            Align2::LEFT_CENTER,
            msg,
            mono(TEXT_SIZE),
            META,
        );
        return None;
    }
    p.selected = p.selected.min(visible.len() - 1);
    let row_h = 34.0;
    let n = ((list.height() / row_h).floor() as usize).max(1);
    let first = p.selected.saturating_sub(n - 1);
    // If not all fit, the scrollbar goes to the right of the rows.
    let overflow = visible.len() > n;
    let row_w = list.width() - if overflow { 12.0 } else { 0.0 };
    if overflow {
        let track = Rect::from_min_size(
            Pos2::new(list.right() - 3.0, list.top()),
            Vec2::new(3.0, n as f32 * row_h - 2.0),
        );
        paint_scrollbar(&painter, track, visible.len(), n, first);
    }
    let mut clicked = None;
    for (row, &idx) in visible.iter().enumerate().skip(first).take(n) {
        let it = &p.items[idx];
        let r = Rect::from_min_size(
            Pos2::new(list.left(), list.top() + (row - first) as f32 * row_h),
            Vec2::new(row_w, row_h - 2.0),
        );
        let resp = ui.interact(r, Id::new(("picker-row", row)), Sense::click());
        let selected = row == p.selected;
        if selected {
            painter.rect_filled(r, CornerRadius::same(6), SELECTED);
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(r.left() + 8.0, r.center().y - 9.0),
                    Vec2::new(3.0, 18.0),
                ),
                CornerRadius::same(2),
                ACCENT,
            );
        } else if resp.hovered() {
            painter.rect_filled(r, CornerRadius::same(6), HOVER);
        }
        let detail_w = if it.detail.is_empty() {
            0.0
        } else {
            let g = painter.layout_no_wrap(it.detail.clone(), mono(SMALL_SIZE), SUBTLE);
            let w = g.size().x;
            painter.galley(
                Pos2::new(r.right() - 12.0 - w, r.center().y - g.size().y / 2.0),
                g,
                SUBTLE,
            );
            w + 24.0
        };
        let mut job = LayoutJob::single_section(
            it.label.clone(),
            TextFormat {
                font_id: if selected {
                    bold(TEXT_SIZE)
                } else {
                    mono(TEXT_SIZE)
                },
                color: if selected { BRIGHT } else { FG },
                ..Default::default()
            },
        );
        job.wrap.max_width = (r.width() - 24.0 - detail_w).max(40.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = painter.layout_job(job);
        painter.galley(
            Pos2::new(r.left() + 20.0, r.center().y - g.size().y / 2.0),
            g,
            FG,
        );
        if resp.clicked() {
            clicked = Some(row);
        }
    }
    clicked
}

fn form_contents(ui: &mut Ui, f: &Form, size: Vec2) {
    let rect = Rect::from_min_size(ui.max_rect().min, size);
    let p = ui.painter().clone();
    p.text(
        Pos2::new(rect.left() + 18.0, rect.top() + 26.0),
        Align2::LEFT_CENTER,
        f.title(),
        bold(TEXT_SIZE),
        ACCENT,
    );
    p.hline(rect.x_range(), rect.top() + 50.0, Stroke::new(1.0, BORDER));
    let cell_w = ui.fonts_mut(|fo| fo.glyph_width(&mono(TEXT_SIZE), 'M'));
    for (i, (label, input)) in f.fields.iter().enumerate() {
        let y = rect.top() + 64.0 + i as f32 * 58.0;
        let focused = i == f.focus;
        p.text(
            Pos2::new(rect.left() + 18.0, y + 8.0),
            Align2::LEFT_CENTER,
            *label,
            mono(SMALL_SIZE),
            if focused { ACCENT } else { SUBTLE },
        );
        let field = Rect::from_min_size(
            Pos2::new(rect.left() + 18.0, y + 20.0),
            Vec2::new(rect.width() - 36.0, 30.0),
        );
        p.rect(
            field,
            CornerRadius::same(6),
            FIELD,
            Stroke::new(1.0, if focused { ACCENT } else { RULE }),
            egui::StrokeKind::Inside,
        );
        let cols = ((field.width() - 20.0) / cell_w) as usize;
        let (text, cx) = input.view(cols.max(4));
        let tpos = Pos2::new(field.left() + 10.0, field.center().y);
        p.text(tpos, Align2::LEFT_CENTER, text, mono(TEXT_SIZE), BRIGHT);
        if focused {
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(tpos.x + cx as f32 * cell_w, field.center().y - 8.0),
                    Vec2::new(2.0, 16.0),
                ),
                CornerRadius::same(1),
                ACCENT,
            );
        }
    }
    let foot = Pos2::new(rect.left() + 18.0, rect.bottom() - 20.0);
    match &f.error {
        Some(e) => {
            p.text(foot, Align2::LEFT_CENTER, e, mono(SMALL_SIZE), RED);
        }
        None => paint_hints(
            &p,
            foot,
            &[("↵", "next/save"), ("Tab", "switch"), ("Esc", "cancel")],
        ),
    }
}

fn draw_prefix_menu(ctx: &egui::Context) {
    const KEYS: [(&str, &str); 13] = [
        ("t", "new tab"),
        ("h", "history"),
        ("w", "close tab"),
        ("s", "snippets"),
        ("n / p", "next/prev tab"),
        ("r", "profiles"),
        ("1–9", "go to tab"),
        ("e", "show/hide editor"),
        ("Tab", "focus terminal/editor"),
        ("k", "clear blocks"),
        ("y", "copy last output"),
        ("g", "send ^G to program"),
        ("q", "quit"),
    ];
    let rows = KEYS.len().div_ceil(2) as f32;
    let size = Vec2::new(600.0, 52.0 + rows * 26.0);
    let frame = Frame::new()
        .fill(RAISED)
        .stroke(Stroke::new(1.0, RULE))
        .shadow(Shadow {
            offset: [0, 18],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(160),
        });
    egui::Area::new(Id::new("prefix"))
        .order(Order::Foreground)
        .anchor(Align2::LEFT_BOTTOM, Vec2::new(PAD, -(PROMPT_H + 18.0)))
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                let rect = Rect::from_min_size(ui.max_rect().min, size);
                let p = ui.painter();
                let r = p.text(
                    Pos2::new(rect.left() + 14.0, rect.top() + 20.0),
                    Align2::LEFT_CENTER,
                    "⌃G",
                    bold(SMALL_SIZE),
                    TEXT2,
                );
                p.text(
                    Pos2::new(r.right() + 10.0, rect.top() + 20.0),
                    Align2::LEFT_CENTER,
                    "press a key",
                    mono(SMALL_SIZE),
                    META,
                );
                p.hline(rect.x_range(), rect.top() + 40.0, Stroke::new(1.0, BORDER2));
                for (i, (k, d)) in KEYS.iter().enumerate() {
                    let col = (i % 2) as f32;
                    let x = rect.left() + 14.0 + col * (rect.width() / 2.0);
                    let y = rect.top() + 58.0 + (i / 2) as f32 * 26.0;
                    p.text(
                        Pos2::new(x + 44.0, y),
                        Align2::RIGHT_CENTER,
                        *k,
                        bold(TEXT_SIZE),
                        ACCENT,
                    );
                    p.text(
                        Pos2::new(x + 58.0, y),
                        Align2::LEFT_CENTER,
                        *d,
                        mono(TEXT_SIZE),
                        FG,
                    );
                }
            });
        });
}

// ── Blocks ───────────────────────────────────────────────────────────────────

/// Block height in points: context + command + output + padding.
fn block_height(b: &mut CmdBlock, cols: usize, cell: CellSize) -> f32 {
    let rows = match b.rows_cache {
        Some((c, v, r)) if c == cols && v == b.out.version => r,
        _ => {
            let n = b.out.visible_len(b.running());
            let r = b.out.lines[..n]
                .iter()
                .map(|l| line_rows(l, cols))
                .sum::<usize>()
                + (b.out.dropped > 0) as usize;
            b.rows_cache = Some((cols, b.out.version, r));
            r
        }
    };
    let out = if rows > 0 {
        OUT_GAP + rows as f32 * cell.h
    } else {
        0.0
    };
    BLOCK_PAD + META_H + cell.h + out + BLOCK_PAD
}

/// Where and how a block is drawn.
struct BlockPlace {
    /// Block rect (full view width).
    rect: Rect,
    cols: usize,
    cell: CellSize,
    /// Visible area, in screen coordinates.
    view: Rect,
}

/// Block context line: `~/path git:(branch) 3 • +48 -12 (0.04s)`.
fn meta_job(b: &CmdBlock) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |color| TextFormat {
        font_id: mono(SMALL_SIZE),
        color,
        ..Default::default()
    };
    let mut text = match &b.host {
        Some(host) => format!("{host}:{}", shorten_path(&b.cwd, 60)),
        None => shorten_path(&short_path(&b.cwd), 60),
    };
    if let Some(br) = &b.branch {
        text.push_str(&format!(" git:({br})"));
    }
    if let Some(g) = b.git.filter(|g| g.files > 0) {
        text.push_str(&format!(
            " {} • +{} -{}",
            g.files, g.insertions, g.deletions
        ));
    }
    job.append(&text, 0.0, fmt(META));
    match b.end {
        None => job.append(&fmt_duration(b.started.elapsed()), 26.0, fmt(ACCENT)),
        Some((code, d)) => {
            job.append(&format!("({})", fmt_duration(d)), 6.0, fmt(META));
            if let Some(c) = code.filter(|c| *c != 0) {
                job.append(&format!("· exit {c}"), 6.0, fmt(RED));
            }
        }
    }
    job
}

fn draw_block(ui: &mut Ui, b: &CmdBlock, place: BlockPlace, want_cursor: bool, blink: bool) {
    let BlockPlace {
        rect,
        cols,
        cell,
        view,
    } = place;
    let p = ui.painter().clone();
    if b.failed() {
        p.rect_filled(rect, CornerRadius::ZERO, FAIL_BG);
        p.rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
            CornerRadius::ZERO,
            RED,
        );
    }
    p.hline(rect.x_range(), rect.top() + 0.5, Stroke::new(1.0, BORDER));

    // Context (and spinner while running).
    let x = rect.left() + PAD;
    let meta_y = rect.top() + BLOCK_PAD;
    let g = p.layout_job(meta_job(b));
    let meta_w = g.size().x;
    let running_dur_w = if b.running() {
        p.layout_no_wrap(fmt_duration(b.started.elapsed()), mono(SMALL_SIZE), ACCENT)
            .size()
            .x
    } else {
        0.0
    };
    p.galley(Pos2::new(x, meta_y + (META_H - g.size().y) / 2.0), g, META);
    if b.running() {
        let spin = Rect::from_center_size(
            Pos2::new(x + meta_w - running_dur_w - 12.0, meta_y + META_H / 2.0),
            Vec2::splat(11.0),
        );
        ui.put(spin, egui::Spinner::new().size(10.0).color(ACCENT));
    }

    // Command.
    let cmd_y = meta_y + META_H;
    let mut job = LayoutJob::single_section(
        b.cmd.clone(),
        TextFormat {
            font_id: bold(TEXT_SIZE),
            color: BRIGHT,
            ..Default::default()
        },
    );
    job.wrap.max_width = rect.width() - 2.0 * PAD;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    put_left(
        ui,
        Rect::from_min_size(
            Pos2::new(x, cmd_y),
            Vec2::new(rect.width() - 2.0 * PAD, cell.h),
        ),
        Label::new(job).selectable(true),
    );

    // Output.
    let n = b.out.visible_len(b.running());
    let out_top = cmd_y + cell.h + OUT_GAP;
    let row_y = |row: usize| out_top + row as f32 * cell.h;
    let visible = |y: f32| y + cell.h >= view.min.y && y <= view.max.y;
    let mut row = 0usize;
    if b.out.dropped > 0 {
        if visible(row_y(0)) {
            p.text(
                Pos2::new(x, row_y(0) + cell.h / 2.0),
                Align2::LEFT_CENTER,
                format!("… {} earlier lines omitted", b.out.dropped),
                mono(SMALL_SIZE),
                META,
            );
        }
        row += 1;
    }
    for (li, line) in b.out.lines[..n].iter().enumerate() {
        let lr = line_rows(line, cols);
        let y0 = row_y(row);
        if y0 > view.max.y {
            break;
        }
        if y0 + lr as f32 * cell.h < view.min.y {
            row += lr;
            continue;
        }
        for (s, e) in segments(line, cols) {
            let y = row_y(row);
            if visible(y) {
                if e > s {
                    put_left(
                        ui,
                        Rect::from_min_size(
                            Pos2::new(x, y),
                            Vec2::new(rect.width() - 2.0 * PAD, cell.h),
                        ),
                        Label::new(cells_job(&line[s..e])).selectable(true).extend(),
                    );
                }
                if want_cursor && li == b.out.row && blink {
                    let col = b.out.col;
                    let in_seg = (col >= s && col < e) || (col >= e && e == line.len());
                    if in_seg {
                        let cx = (col.min(e) - s) + col.saturating_sub(line.len());
                        let c = Rect::from_min_size(
                            Pos2::new(x + cx as f32 * cell.w, y + 1.0),
                            Vec2::new(cell.w, cell.h - 2.0),
                        );
                        p.rect_filled(c, CornerRadius::same(1), CURSOR);
                    }
                }
            }
            row += 1;
        }
    }
}

fn fmt_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", d.as_secs_f32())
    } else {
        format!("{}m{:02}s", ms / 60_000, (ms / 1000) % 60)
    }
}

// ── Colors and styles ────────────────────────────────────────────────────────

fn color32(c: Color, default: Color32) -> Color32 {
    match c {
        Color::Reset => default,
        Color::Black => ANSI[0],
        Color::Red => ANSI[1],
        Color::Green => ANSI[2],
        Color::Yellow => ANSI[3],
        Color::Blue => ANSI[4],
        Color::Magenta => ANSI[5],
        Color::Cyan => ANSI[6],
        Color::Gray => ANSI[7],
        Color::DarkGray => ANSI[8],
        Color::LightRed => ANSI[9],
        Color::LightGreen => ANSI[10],
        Color::LightYellow => ANSI[11],
        Color::LightBlue => ANSI[12],
        Color::LightMagenta => ANSI[13],
        Color::LightCyan => ANSI[14],
        Color::White => ANSI[15],
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        Color::Indexed(i) => xterm256(i),
    }
}

fn xterm256(i: u8) -> Color32 {
    match i {
        0..=15 => ANSI[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

/// egui format for a ratatui style (block output).
fn text_format(style: Style) -> TextFormat {
    let m = style.add_modifier;
    let mut fg = color32(style.fg.unwrap_or(Color::Reset), FG);
    let mut bg = style
        .bg
        .map_or(Color32::TRANSPARENT, |c| color32(c, Color32::TRANSPARENT));
    if m.contains(Modifier::REVERSED) {
        let back = if bg == Color32::TRANSPARENT { BG } else { bg };
        bg = fg;
        fg = back;
    }
    if m.contains(Modifier::DIM) {
        fg = fg.gamma_multiply(0.6);
    }
    if m.contains(Modifier::HIDDEN) {
        fg = Color32::TRANSPARENT;
    }
    let line = |on: bool| {
        if on {
            Stroke::new(1.0, fg)
        } else {
            Stroke::NONE
        }
    };
    TextFormat {
        font_id: if m.contains(Modifier::BOLD) {
            bold(TEXT_SIZE)
        } else {
            mono(TEXT_SIZE)
        },
        color: fg,
        background: bg,
        italics: m.contains(Modifier::ITALIC),
        underline: line(m.contains(Modifier::UNDERLINED)),
        strikethrough: line(m.contains(Modifier::CROSSED_OUT)),
        ..Default::default()
    }
}

fn cells_job(cells: &[Cell]) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut run = String::new();
    let mut style = cells.first().map(|c| c.style).unwrap_or_default();
    for c in cells {
        if c.style != style {
            job.append(&run, 0.0, text_format(style));
            run.clear();
            style = c.style;
        }
        run.push(c.ch);
    }
    if !run.is_empty() {
        job.append(&run, 0.0, text_format(style));
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

/// Markdown preview line: headings larger.
fn markdown_job(line: &ratatui::text::Line, max_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let heading =
        line.style.fg == Some(Color::Cyan) && line.style.add_modifier.contains(Modifier::BOLD);
    let size = match (
        heading,
        line.style.add_modifier.contains(Modifier::UNDERLINED),
    ) {
        (true, true) => 22.0,
        (true, false) => 17.0,
        _ => TEXT_SIZE,
    };
    for span in &line.spans {
        let mut f = text_format(line.style.patch(span.style));
        f.font_id.size = size;
        if heading {
            f.underline = Stroke::NONE;
            f.color = ACCENT;
        }
        f.line_height = Some(size * 1.55);
        job.append(&span.content, 0.0, f);
    }
    if job.sections.is_empty() {
        job.append(
            " ",
            0.0,
            TextFormat {
                font_id: mono(TEXT_SIZE),
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = max_width;
    job
}

type StyleKey = (vt100::Color, vt100::Color, bool, bool, bool, bool, bool);

fn vt_style_key(c: &vt100::Cell) -> StyleKey {
    (
        c.fgcolor(),
        c.bgcolor(),
        c.bold(),
        c.italic(),
        c.underline(),
        c.inverse(),
        c.dim(),
    )
}

/// Colors and attributes of a vt100 cell: (fg, bg, bold,
/// italic, underline).
fn vt_colors(c: &vt100::Cell) -> (Color32, Color32, bool, bool, bool) {
    let conv = |col: vt100::Color, default: Color32| match col {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => xterm256(i),
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    };
    let mut fg = conv(c.fgcolor(), FG);
    let mut bg = conv(c.bgcolor(), BG);
    if c.inverse() {
        std::mem::swap(&mut fg, &mut bg);
    }
    if c.dim() {
        fg = fg.gamma_multiply(0.6);
    }
    (fg, bg, c.bold(), c.italic(), c.underline())
}

/// Mouse over a fullscreen program: sent to it if it asked for it;
/// otherwise the wheel is translated into arrows (for less, man…).
/// Forwards the mouse to the program. Without `buttons` only the wheel:
/// clicks and drags are for selecting.
fn forward_mouse(ui: &Ui, tab: &mut Tab, live: Rect, cell: CellSize, buttons: bool) {
    let mode = tab.session.vt.screen().mouse_protocol_mode();
    let app_cursor = tab.session.vt.screen().application_cursor();
    let (events, pointer, primary_down) = ui.ctx().input(|i| {
        (
            i.events.clone(),
            i.pointer.hover_pos(),
            i.pointer.primary_down(),
        )
    });
    let to_cell = |pos: Pos2| -> Option<(u16, u16)> {
        live.contains(pos).then(|| {
            (
                ((pos.x - live.left()) / cell.w) as u16,
                ((pos.y - live.top()) / cell.h) as u16,
            )
        })
    };
    let send = |tab: &mut Tab, kind: MouseEventKind, (c, r): (u16, u16), motion: bool| {
        let ev = MouseEvent {
            kind,
            column: c,
            row: r,
            modifiers: KeyModifiers::NONE,
        };
        if let Some(bytes) = keys::encode_mouse(&ev, c, r, motion) {
            tab.session.write(&bytes);
        }
    };
    for ev in events {
        match ev {
            egui::Event::MouseWheel { delta, .. } => {
                let Some(at) = pointer.and_then(to_cell) else {
                    continue;
                };
                let up = delta.y > 0.0;
                if mode == MouseProtocolMode::None && !tab.session.fullscreen() {
                    // Interactive session (ssh…): the wheel scrolls its history.
                    let screen = tab.session.vt.screen_mut();
                    let cur = screen.scrollback();
                    screen.set_scrollback(if up { cur + 3 } else { cur.saturating_sub(3) });
                } else if mode == MouseProtocolMode::None {
                    let arrow: &[u8] = match (up, app_cursor) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    tab.session.write(arrow);
                } else {
                    let kind = if up {
                        MouseEventKind::ScrollUp
                    } else {
                        MouseEventKind::ScrollDown
                    };
                    send(tab, kind, at, false);
                }
            }
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                ..
            } if buttons && mode != MouseProtocolMode::None => {
                let Some(at) = to_cell(pos) else { continue };
                let button = match button {
                    egui::PointerButton::Primary => MouseButton::Left,
                    egui::PointerButton::Secondary => MouseButton::Right,
                    egui::PointerButton::Middle => MouseButton::Middle,
                    _ => continue,
                };
                if !pressed && mode == MouseProtocolMode::Press {
                    continue;
                }
                let kind = if pressed {
                    MouseEventKind::Down(button)
                } else {
                    MouseEventKind::Up(button)
                };
                send(tab, kind, at, false);
            }
            egui::Event::PointerMoved(pos)
                if buttons
                    && matches!(
                        mode,
                        MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
                    ) =>
            {
                let Some(at) = to_cell(pos) else { continue };
                let any = mode == MouseProtocolMode::AnyMotion;
                if primary_down {
                    send(tab, MouseEventKind::Drag(MouseButton::Left), at, any);
                } else if any {
                    send(tab, MouseEventKind::Moved, at, true);
                }
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "macos")]
mod macos;
