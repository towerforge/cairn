//! Native egui window: title bar with tabs, blocks as panels, prompt,
//! side editor and overlay windows, with rounded corners, padding, shadows
//! and selectable text.
//!
//! All logic lives in `App`: this only draws and
//! translates egui events into the crossterm events `App` understands.
//! The cell grid is used only where needed: command output and
//! fullscreen programs.
//!
//! This file owns the window: startup, the frame loop and the layout of
//! the panels. Each panel draws itself in its own file, as methods on
//! [`Gui`]:
//!
//! - `titlebar`: the tab bar.
//! - `blocks`: the command blocks.
//! - `prompt`: the input line with its context chips and hints.
//! - `dock`: the `/` command list and `Tab` suggestions above the prompt.
//! - `fullscreen`: programs that take over the view (vim, htop…).
//! - `editor`: the side editor with its Markdown preview.
//! - `overlay`: pickers, forms and the ⌃G menu.
//!
//! Shared by all of them: `theme` (palette, sizes, fonts), `text`
//! (terminal styles as egui text), `widgets` (keycaps, scrollbars, chips)
//! and `input` (keyboard and wheel events). `macos` holds the AppKit
//! tweaks to the window itself.

mod blocks;
mod dock;
mod editor;
mod fullscreen;
mod input;
#[cfg(target_os = "macos")]
mod macos;
mod overlay;
mod prompt;
mod text;
mod theme;
mod titlebar;
mod widgets;

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Frame, Ui, ViewportCommand};

use crate::app::App;
use crate::app::overlay::Overlay;
use crate::event::{AppEvent, EventSink};
use crate::store::Store;

use dock::dock_height;
use fullscreen::GridSel;
use overlay::{dim_background, draw_prefix_menu};
use theme::{BG, PROMPT_H, TEXT_SIZE, TITLEBAR_H, mono, setup_fonts, setup_style};

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
            let startup = store.default_profile().cloned();
            let size = ratatui::layout::Rect::new(0, 0, 120, 40);
            let mut app = App::new(store, sink, size);
            // The startup profile, if set; otherwise the login shell here.
            // From Finder the current directory is "/": $HOME is better.
            let opened = match startup {
                Some(p) => app.new_tab(Some(p.shell), Some(p.cwd), Some(p.name)),
                None => false,
            };
            let cwd = std::env::current_dir()
                .ok()
                .filter(|p| p.as_os_str() != "/")
                .map(|p| p.to_string_lossy().into_owned());
            if !opened && !app.new_tab(None, cwd, None) {
                let msg = app.message.take().map(|(m, _)| m).unwrap_or_default();
                return Err(format!("could not open the first tab: {msg}").into());
            }
            Ok(Box::new(Gui {
                app,
                rx,
                title: String::new(),
                titlebar_drag: false,
                wheel: 0.0,
                #[cfg(target_os = "macos")]
                rounded: false,
                grid_sel: None,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

// ── The window ───────────────────────────────────────────────────────────────

/// The window: the application state, the channel the PTY threads write
/// to, and the little state that only the drawing needs.
struct Gui {
    app: App,
    rx: Receiver<AppEvent>,
    title: String,
    /// Dragging the window from the title bar.
    titlebar_drag: bool,
    /// Wheel accumulated over a list (in points), until a full step.
    wheel: f32,
    /// Rounded corners already applied to the window (macOS).
    #[cfg(target_os = "macos")]
    rounded: bool,
    /// Text selected with the mouse in the fullscreen terminal.
    grid_sel: Option<GridSel>,
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
                self.rounded = macos::round_corners(frame, theme::WINDOW_RADIUS);
            }
            macos::center_traffic_lights(frame, TITLEBAR_H * ctx.zoom_factor());
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
    /// Lays out the panels and draws each one.
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
}
