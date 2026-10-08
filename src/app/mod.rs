//! Global state and event dispatch.
//!
//! Shortcuts: everything in Cairn hangs off the `Ctrl+G` prefix, so every
//! other key reaches the shell or program untouched.

pub mod complete;
pub mod overlay;
pub mod tab;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::app::overlay::{
    Form, FormAction, FormKind, Item, Overlay, Picker, PickerAction, PickerKind,
};
use crate::app::tab::{CmdState, Focus, Tab, short_path};
use crate::edit::editor::{Editor, EditorAction};
use crate::event::{AppEvent, EventSink};
use crate::store::Store;
use crate::term::keys;
use crate::term::session::Session;
use crate::term::shell;

const MESSAGE_TTL: Duration = Duration::from_secs(5);

/// Open completion list: the options and the prompt span replaced while
/// cycling through them.
pub struct CompletionState {
    pub items: Vec<crate::app::complete::Candidate>,
    /// Option currently inserted (none until the first `Tab` or arrow).
    pub selected: Option<usize>,
    /// Prompt span (in chars) taken by the completed word.
    start: usize,
    end: usize,
}

/// Cairn command typed at the prompt starting with `/`.
pub struct SlashCommand {
    pub name: &'static str,
    /// Accepted argument: `<required>`, `[optional]` or empty.
    pub args: &'static str,
    pub desc: &'static str,
}

pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "help",
        args: "",
        desc: "help: version, commands and shortcuts",
    },
    SlashCommand {
        name: "history",
        args: "",
        desc: "command history",
    },
    SlashCommand {
        name: "new",
        args: "",
        desc: "new tab",
    },
    SlashCommand {
        name: "close",
        args: "",
        desc: "close tab",
    },
    SlashCommand {
        name: "editor",
        args: "[file]",
        desc: "open file / show or hide editor",
    },
    SlashCommand {
        name: "snippets",
        args: "",
        desc: "snippets",
    },
    SlashCommand {
        name: "profiles",
        args: "",
        desc: "profiles",
    },
    SlashCommand {
        name: "clear",
        args: "",
        desc: "clear blocks",
    },
    SlashCommand {
        name: "copy",
        args: "",
        desc: "copy last output",
    },
    SlashCommand {
        name: "update",
        args: "",
        desc: "install the latest release",
    },
    SlashCommand {
        name: "quit",
        args: "",
        desc: "quit",
    },
];

/// Keys of a shortcut, one per keycap: `↑↓` is two; word keys (`Esc`,
/// `Tab`) are lowercased.
pub fn keycaps(key: &str) -> Vec<String> {
    if key.chars().all(|c| "↑↓←→".contains(c)) {
        return key.chars().map(String::from).collect();
    }
    match key {
        "Esc" | "Tab" => vec![key.to_lowercase()],
        _ => vec![key.to_string()],
    }
}

/// `/cmd args` if `cmd` is a Cairn command.
fn slash_command(text: &str) -> Option<(&'static str, &str)> {
    let rest = text.strip_prefix('/')?;
    let (word, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let c = COMMANDS
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(word))?;
    Some((c.name, args.trim()))
}

/// Items of the `/help` modal: Cairn commands, run when chosen, and at the end
/// the data files (informational only).
fn help_items(paths: &crate::store::Paths) -> Vec<Item> {
    let item = |label: String, detail: &str, value: String| Item {
        label,
        detail: detail.to_string(),
        value,
    };
    let mut items: Vec<Item> = COMMANDS
        .iter()
        .map(|c| {
            let label = if c.args.is_empty() {
                format!("/{}", c.name)
            } else {
                format!("/{} {}", c.name, c.args)
            };
            item(label, c.desc, format!("/{}", c.name))
        })
        .collect();
    items.push(item(
        "edit <file>".into(),
        "open file in the side editor",
        String::new(),
    ));
    let file = |p: std::path::PathBuf| short_path(&p.to_string_lossy());
    items.push(item(
        "config.toml".into(),
        &format!(
            "profiles and snippets, at {}; can be edited by hand",
            file(paths.config_file())
        ),
        String::new(),
    ));
    items.push(item(
        "history.jsonl".into(),
        &format!(
            "history, one JSON line per command, at {}",
            file(paths.history_file())
        ),
        String::new(),
    ));
    items.push(item(
        "updates".into(),
        "checked once a day at startup; /update or `cairn update` installs \
         the latest; `update_check = false` in config.toml turns it off",
        String::new(),
    ));
    items
}

pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub store: Store,
    pub overlay: Option<Overlay>,
    /// `Ctrl+G` was pressed and the action key is awaited.
    pub prefix: bool,
    pub quit: bool,
    quit_confirm: bool,
    pub message: Option<(String, Instant)>,
    pub size: Rect,
    /// Selected row in the `/` command list.
    pub palette_sel: usize,
    /// Open completion list (several options after `Tab`).
    pub completion: Option<CompletionState>,
    /// A newer release found by the startup check (or `None`).
    pub update_available: Option<crate::update::Version>,
    /// `/update` is running on its thread.
    updating: bool,
    sink: EventSink,
    next_id: u64,
}

impl App {
    pub fn new(store: Store, sink: EventSink, size: Rect) -> Self {
        crate::app::complete::warm_up();
        if store.update_check() {
            crate::update::spawn_startup_check(sink.clone(), store.paths().state_dir.clone());
        }
        Self {
            tabs: Vec::new(),
            active: 0,
            store,
            overlay: None,
            prefix: false,
            quit: false,
            quit_confirm: false,
            message: None,
            size,
            palette_sel: 0,
            completion: None,
            update_available: None,
            updating: false,
            sink,
            next_id: 1,
        }
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.message = Some((msg.into(), Instant::now()));
    }

    /// Opens a new tab. Without a shell uses `$SHELL`; without a directory, `$HOME`.
    pub fn new_tab(
        &mut self,
        shell_path: Option<String>,
        cwd: Option<String>,
        profile: Option<String>,
    ) -> bool {
        let shell_path = shell_path
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(shell::default_shell);
        let home = dirs::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into());
        let cwd = cwd
            .map(|c| expand_home(&c))
            .filter(|c| std::path::Path::new(c).is_dir())
            .unwrap_or(home);
        let id = self.next_id;
        self.next_id += 1;
        // Provisional size until the window lays out the tab (resize_active).
        let size = (
            self.size.height.saturating_sub(1).max(2),
            self.size.width.saturating_sub(2).max(10),
        );
        match Session::spawn(&shell_path, Some(&cwd), size, id, self.sink.clone()) {
            Ok(session) => {
                self.tabs.push(Tab::new(id, session, cwd, profile));
                self.active = self.tabs.len() - 1;
                self.palette_sel = 0;
                self.refresh_git(id);
                true
            }
            Err(e) => {
                self.flash(format!("could not launch {shell_path}: {e}"));
                false
            }
        }
    }

    /// Recomputes a tab's git stats in the background.
    fn refresh_git(&mut self, id: u64) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if tab.branch.is_none() {
            tab.git = None;
            return;
        }
        if tab.git_pending {
            tab.git_dirty = true;
            return;
        }
        tab.git_pending = true;
        let cwd = tab.cwd.clone();
        let sink = self.sink.clone();
        std::thread::spawn(move || {
            sink(AppEvent::Git(id, crate::git::diff_stat(&cwd)));
        });
    }

    pub fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.quit = true;
            return;
        }
        if self.active >= self.tabs.len() || self.active > idx {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
    }

    pub fn tick(&mut self) {
        let mut notices = Vec::new();
        for tab in &mut self.tabs {
            tab.tick();
            if let Some(n) = tab.notice.take() {
                notices.push(n);
            }
        }
        if let Some(n) = notices.pop() {
            self.flash(n);
        }
        if self
            .message
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() > MESSAGE_TTL)
        {
            self.message = None;
        }
    }

    /// Refresh interval: fast while there is a spinner to animate.
    pub fn tick_rate(&self) -> Duration {
        if self
            .tabs
            .iter()
            .any(|t| t.busy() || t.state == CmdState::Starting)
        {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(1000)
        }
    }

    pub fn handle(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Term(Event::Key(k)) if k.kind != KeyEventKind::Release => self.on_key(k),
            AppEvent::Term(Event::Paste(s)) => self.on_paste(&s),
            AppEvent::Term(_) => {}
            AppEvent::Pty(id, data) => {
                let refresh = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == id)
                    .is_some_and(|tab| tab.on_output(&data));
                if refresh {
                    self.refresh_git(id);
                }
            }
            AppEvent::Git(id, stat) => {
                let again = match self.tabs.iter_mut().find(|t| t.id == id) {
                    Some(tab) => {
                        tab.git = stat;
                        tab.git_pending = false;
                        std::mem::take(&mut tab.git_dirty)
                    }
                    None => false,
                };
                if again {
                    self.refresh_git(id);
                }
            }
            AppEvent::PtyClosed(id) => {
                if let Some(idx) = self.tabs.iter().position(|t| t.id == id) {
                    self.close_tab(idx);
                }
            }
            AppEvent::Update(notice) => self.on_update(notice),
        }
    }

    fn on_update(&mut self, notice: crate::update::Notice) {
        use crate::update::Notice;
        match notice {
            Notice::Available(v) => {
                self.flash(format!("Cairn v{v} is available · /update installs it"));
                self.update_available = Some(v);
            }
            Notice::Progress(text) => self.flash(text),
            Notice::Done(result) => {
                self.updating = false;
                match result {
                    Ok(msg) => {
                        self.update_available = None;
                        self.flash(msg);
                    }
                    Err(msg) => self.flash(msg),
                }
            }
        }
    }

    /// `/update`: installs the latest release on a thread; the messages come
    /// back as `AppEvent::Update`.
    fn start_update(&mut self) {
        if self.updating {
            self.flash("an update is already running");
            return;
        }
        self.updating = true;
        self.flash("looking for the latest release…");
        crate::update::spawn_update(self.sink.clone(), self.store.paths().state_dir.clone());
    }

    // ── Keyboard ─────────────────────────────────────────────────────────────

    fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if self.prefix {
            self.prefix = false;
            self.prefix_key(k);
            return;
        }
        if ctrl && k.code == KeyCode::Char('g') {
            self.prefix = true;
            return;
        }
        if self.overlay.is_some() {
            self.overlay_key(k);
            return;
        }
        self.quit_confirm = false;

        let page = self.size.height.saturating_sub(5) as usize;
        let tab = self.tab_mut();

        if tab.has_editor() && tab.focus == Focus::Editor {
            let editor = tab.editor.as_mut().expect("editor visible");
            match editor.handle_key(k, page.max(1)) {
                EditorAction::None => {}
                EditorAction::Unfocus => tab.focus = Focus::Terminal,
                EditorAction::Close => {
                    tab.editor = None;
                    tab.editor_visible = false;
                    tab.focus = Focus::Terminal;
                }
            }
            return;
        }

        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if tab.busy() {
            // With a command running, keys go to the program, except
            // Shift+PgUp/PgDn to scroll the blocks.
            match k.code {
                KeyCode::PageUp if shift && !tab.fullscreen() => tab.scroll += page,
                KeyCode::PageDown if shift && !tab.fullscreen() => {
                    tab.scroll = tab.scroll.saturating_sub(page)
                }
                _ => {
                    // Typing jumps back to the bottom if browsing the scrollback.
                    if tab.session.vt.screen().scrollback() > 0 {
                        tab.session.vt.screen_mut().set_scrollback(0);
                    }
                    let app_cursor = tab.session.vt.screen().application_cursor();
                    let bytes = keys::encode_key(k, app_cursor);
                    if !bytes.is_empty() {
                        tab.session.write(&bytes);
                    }
                }
            }
            return;
        }

        if self.completion.is_some() && self.completion_key(k) {
            return;
        }
        if let Some(list) = self.palette() {
            if self.palette_key(k, &list) {
                return;
            }
            self.palette_sel = 0;
        }
        let tab = self.tab_mut();
        match k.code {
            // Empty prompt with the editor visible: focus moves to the editor.
            KeyCode::Tab if tab.input.text.is_empty() && tab.has_editor() => {
                tab.focus = Focus::Editor
            }
            KeyCode::Tab => self.complete(),
            KeyCode::Enter => {
                if tab.state == CmdState::Starting {
                    self.flash("the shell is still starting…");
                } else {
                    self.submit();
                }
            }
            KeyCode::Up => {
                if tab.local_hist.is_empty() {
                    return;
                }
                let pos = match tab.hist_pos {
                    None => tab.local_hist.len() - 1,
                    Some(p) => p.saturating_sub(1),
                };
                tab.hist_pos = Some(pos);
                let text = tab.local_hist[pos].clone();
                tab.input.set(&text);
            }
            KeyCode::Down => {
                if let Some(p) = tab.hist_pos {
                    if p + 1 < tab.local_hist.len() {
                        tab.hist_pos = Some(p + 1);
                        let text = tab.local_hist[p + 1].clone();
                        tab.input.set(&text);
                    } else {
                        tab.hist_pos = None;
                        tab.input.clear();
                    }
                }
            }
            KeyCode::PageUp => tab.scroll += page,
            KeyCode::PageDown => tab.scroll = tab.scroll.saturating_sub(page),
            KeyCode::Char('c') if ctrl => tab.input.clear(),
            KeyCode::Char('l') if ctrl => {
                tab.blocks.clear();
                tab.scroll = 0;
            }
            KeyCode::Char('d') if ctrl && tab.input.text.is_empty() => {
                let idx = self.active;
                self.close_tab(idx);
            }
            KeyCode::Char('r') if ctrl => self.open_picker(PickerKind::History),
            _ => {
                tab.input.handle_key(k);
            }
        }
    }

    fn submit(&mut self) {
        let tab = self.tab_mut();
        let raw = tab.input.take();
        let cmd = raw.trim().to_string();
        tab.hist_pos = None;
        if cmd.is_empty() {
            return;
        }
        // `/cmd args` with the list already closed (by the space).
        if let Some((name, args)) = slash_command(&cmd) {
            self.run_slash(name, args);
            return;
        }
        let tab = self.tab_mut();
        if tab.local_hist.last() != Some(&cmd) {
            tab.local_hist.push(cmd.clone());
        }
        let (profile, cwd) = (tab.profile_name.clone(), tab.cwd.clone());
        if let Err(e) = self.store.add_history(profile.as_deref(), &cmd, Some(&cwd)) {
            self.flash(format!("could not save history: {e}"));
        }

        let mut words = cmd.split_whitespace();
        match words.next() {
            Some("clear" | "cls") if words.next().is_none() => {
                let tab = self.tab_mut();
                tab.blocks.clear();
                tab.scroll = 0;
            }
            Some("edit") => {
                let rest = cmd["edit".len()..].trim();
                self.open_editor(rest);
            }
            _ => {
                let live = interactive(&cmd);
                let tab = self.tab_mut();
                let send = match ssh_rewrite(&cmd) {
                    // Only from the local machine and with a shell that has the function.
                    Some(s) if tab.remote.is_none() && !tab.session.kind.implicit_start() => s,
                    _ => cmd.clone(),
                };
                tab.run(&cmd, &send, live);
            }
        }
    }

    fn open_editor(&mut self, arg: &str) {
        let tab = self.tab_mut();
        if arg.is_empty() {
            if tab.editor.is_some() {
                tab.editor_visible = true;
                tab.focus = Focus::Editor;
            } else {
                self.flash("usage: edit <file>");
            }
            return;
        }
        let path = resolve_path(arg, &tab.cwd);
        if tab
            .editor
            .as_ref()
            .is_some_and(|e| e.dirty && e.path != path)
        {
            self.flash("the editor has unsaved changes: save them (Ctrl+S) or close it (Ctrl+X)");
            return;
        }
        match Editor::open(path) {
            Ok(editor) => {
                tab.editor = Some(editor);
                tab.editor_visible = true;
                tab.focus = Focus::Editor;
            }
            Err(e) => self.flash(e),
        }
    }

    /// `Tab` at the prompt: completes the word before the cursor (paths or
    /// commands; failing that, history commands with the same prefix).
    fn complete(&mut self) {
        let tab = self.tab();
        if tab.remote.is_some() {
            self.flash("completion not available in remote sessions");
            return;
        }
        let cursor = tab.input.cursor();
        let before: String = tab.input.text.chars().take(cursor).collect();
        let result = crate::app::complete::complete(&before, &tab.cwd);
        let (start, items) = if result.candidates.is_empty() {
            (0, self.history_candidates(&before))
        } else {
            (result.start, result.candidates)
        };
        match items.len() {
            0 => self.flash("no matches"),
            1 => {
                let insert = items[0].insert.clone();
                self.tab_mut().input.replace_chars(start, cursor, &insert);
            }
            _ => {
                // Complete the common part and open the list to choose.
                let common = crate::app::complete::common_prefix(&items);
                let mut end = cursor;
                if common.chars().count() > cursor - start {
                    self.tab_mut().input.replace_chars(start, cursor, &common);
                    end = start + common.chars().count();
                }
                self.completion = Some(CompletionState {
                    items,
                    selected: None,
                    start,
                    end,
                });
            }
        }
    }

    /// History commands starting with the typed text (deduplicated).
    fn history_candidates(&self, before: &str) -> Vec<crate::app::complete::Candidate> {
        let typed = before.trim_start();
        if typed.is_empty() {
            return Vec::new();
        }
        let mut seen = std::collections::HashSet::new();
        self.store
            .history()
            .filter(|h| h.command.starts_with(typed) && h.command != typed)
            .filter(|h| seen.insert(h.command.as_str()))
            .take(30)
            .map(|h| crate::app::complete::Candidate {
                label: h.command.clone(),
                insert: h.command.clone(),
                dir: false,
            })
            .collect()
    }

    /// Keys with the completion list open. Returns `false` if the key is not
    /// its own: the list closes and the key is processed normally.
    fn completion_key(&mut self, k: KeyEvent) -> bool {
        let Some(c) = self.completion.as_mut() else {
            return false;
        };
        let n = c.items.len();
        let next = match k.code {
            KeyCode::Tab | KeyCode::Down => Some(c.selected.map_or(0, |i| (i + 1) % n)),
            KeyCode::BackTab | KeyCode::Up => Some(c.selected.map_or(n - 1, |i| (i + n - 1) % n)),
            KeyCode::Enter => {
                // Accept the insertion (without running the command).
                let had = c.selected.is_some();
                self.completion = None;
                return had;
            }
            KeyCode::Esc => {
                self.completion = None;
                return true;
            }
            _ => {
                self.completion = None;
                return false;
            }
        };
        if let Some(i) = next {
            c.selected = Some(i);
            let insert = c.items[i].insert.clone();
            let (start, end) = (c.start, c.end);
            c.end = start + insert.chars().count();
            self.tab_mut().input.replace_chars(start, end, &insert);
        }
        true
    }

    /// Mouse wheel over the modal or the open list (commands, suggestions):
    /// one selection step, without wrapping at the end. Returns `false` if
    /// none is open.
    pub fn scroll_list(&mut self, down: bool) -> bool {
        let arrow = KeyEvent::new(
            if down { KeyCode::Down } else { KeyCode::Up },
            KeyModifiers::NONE,
        );
        if let Some(Overlay::Picker(p)) = self.overlay.as_mut() {
            p.handle_key(arrow);
            return true;
        }
        if self.overlay.is_some() {
            return false;
        }
        if let Some(list) = self.palette() {
            let sel = self.palette_sel.min(list.len() - 1);
            self.palette_sel = if down {
                (sel + 1).min(list.len() - 1)
            } else {
                sel.saturating_sub(1)
            };
            return true;
        }
        if let Some(c) = self.completion.as_ref() {
            let at_end = match c.selected {
                Some(i) if down => i + 1 == c.items.len(),
                Some(i) => i == 0,
                None => !down,
            };
            if !at_end {
                self.completion_key(arrow);
            }
            return true;
        }
        false
    }

    /// Picks a list option with the mouse: inserts it and closes the list.
    pub fn pick_completion(&mut self, i: usize) {
        let Some(c) = self.completion.take() else {
            return;
        };
        if let Some(item) = c.items.get(i) {
            self.tab_mut()
                .input
                .replace_chars(c.start, c.end, &item.insert);
        }
    }

    /// Command name being typed: the prompt is `/something` without spaces.
    pub fn typing_command(&self) -> Option<&str> {
        let tab = self.tabs.get(self.active)?;
        if self.overlay.is_some() || self.prefix || tab.busy() {
            return None;
        }
        let rest = tab.input.text.strip_prefix('/')?;
        (!rest.contains(char::is_whitespace)).then_some(rest)
    }

    /// Commands starting with the text after `/` (all of them for a bare `/`).
    /// `None` if no command is being typed or none matches: `/usr/bin/ls` goes
    /// to the shell.
    pub fn palette(&self) -> Option<Vec<&'static SlashCommand>> {
        let word = self.typing_command()?.to_lowercase();
        let list: Vec<_> = COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(&word))
            .collect();
        (!list.is_empty()).then_some(list)
    }

    /// Keys with the list open: ↑↓ and ⇧Tab cycle through it with wrapping, Tab
    /// completes the name and Enter runs (or completes if an argument is missing).
    /// Returns `false` if the key is not its own (the prompt text is edited).
    fn palette_key(&mut self, k: KeyEvent, list: &[&'static SlashCommand]) -> bool {
        let n = list.len();
        let sel = self.palette_sel.min(n - 1);
        match k.code {
            KeyCode::Up if n > 1 => self.palette_sel = (sel + n - 1) % n,
            KeyCode::Down if n > 1 => self.palette_sel = (sel + 1) % n,
            KeyCode::BackTab => self.palette_sel = (sel + n - 1) % n,
            KeyCode::Esc => self.tab_mut().input.clear(),
            KeyCode::Tab => self.tab_mut().input.set(&format!("/{} ", list[sel].name)),
            KeyCode::Enter => {
                // A fully typed name takes precedence over the selection.
                let typed = self.typing_command().unwrap_or("").to_lowercase();
                let c = list.iter().find(|c| c.name == typed).unwrap_or(&list[sel]);
                if c.args.starts_with('<') {
                    self.tab_mut().input.set(&format!("/{} ", c.name));
                } else {
                    self.tab_mut().input.clear();
                    self.palette_sel = 0;
                    self.run_slash(c.name, "");
                }
            }
            _ => return false,
        }
        true
    }

    fn run_slash(&mut self, name: &str, args: &str) {
        match name {
            "help" => self.open_picker(PickerKind::Help),
            "history" => self.open_picker(PickerKind::History),
            "new" => self.command('t'),
            "close" => self.command('w'),
            "snippets" => self.open_picker(PickerKind::Snippets),
            "profiles" => self.open_picker(PickerKind::Profiles),
            "clear" => self.command('k'),
            "copy" => self.copy_last_output(),
            "update" => self.start_update(),
            "quit" => self.command('q'),
            "editor" if args.is_empty() => self.command('e'),
            "editor" => self.open_editor(args),
            _ => {}
        }
    }

    /// Runs a `Ctrl+G` menu action (used by the native window's Cmd
    /// shortcuts).
    pub fn command(&mut self, c: char) {
        self.overlay = None;
        self.prefix = false;
        self.prefix_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }

    /// Resizes the active tab's PTY (native window: every frame).
    pub fn resize_active(&mut self, rows: u16, cols: u16) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.session.resize(rows, cols);
        }
    }

    /// Keyboard focus is on the side editor.
    pub fn editor_focused(&self) -> bool {
        let tab = self.tab();
        tab.has_editor() && tab.focus == Focus::Editor
    }

    /// Title for the native window.
    pub fn window_title(&self) -> String {
        let tab = self.tab();
        match tab.blocks.last().filter(|b| b.running()) {
            Some(b) => format!("{} — {}", b.cmd, tab.title()),
            None => format!("Cairn — {}", short_path(&tab.cwd)),
        }
    }

    fn prefix_key(&mut self, k: KeyEvent) {
        self.completion = None;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('g') => {
                // Ctrl+G Ctrl+G (or g): sends a literal ^G to the program.
                if self.tab().busy() {
                    self.tab_mut().session.write(b"\x07");
                }
            }
            KeyCode::Char('t') => {
                let cwd = self.tab().cwd.clone();
                self.new_tab(None, Some(cwd), None);
            }
            KeyCode::Char('w') => {
                let idx = self.active;
                self.close_tab(idx);
            }
            KeyCode::Char('n') | KeyCode::Right => {
                self.active = (self.active + 1) % self.tabs.len()
            }
            KeyCode::Char('p') | KeyCode::Left => {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            }
            KeyCode::Char(c @ '1'..='9') if !ctrl => {
                let i = c as usize - '1' as usize;
                if i < self.tabs.len() {
                    self.active = i;
                }
            }
            KeyCode::Char('h') => self.open_picker(PickerKind::History),
            KeyCode::Char('s') => self.open_picker(PickerKind::Snippets),
            KeyCode::Char('r') => self.open_picker(PickerKind::Profiles),
            KeyCode::Char('e') => {
                let tab = self.tab_mut();
                if tab.editor.is_some() {
                    tab.editor_visible = !tab.editor_visible;
                    tab.focus = if tab.editor_visible {
                        Focus::Editor
                    } else {
                        Focus::Terminal
                    };
                } else {
                    self.flash("open a file with: edit <file>");
                }
            }
            KeyCode::Tab => {
                let tab = self.tab_mut();
                if tab.has_editor() {
                    tab.focus = if tab.focus == Focus::Editor {
                        Focus::Terminal
                    } else {
                        Focus::Editor
                    };
                }
            }
            KeyCode::Char('k') => {
                let tab = self.tab_mut();
                tab.blocks.clear();
                tab.scroll = 0;
            }
            KeyCode::Char('y') => self.copy_last_output(),
            KeyCode::Char('q') => {
                let dirty = self
                    .tabs
                    .iter()
                    .any(|t| t.editor.as_ref().is_some_and(|e| e.dirty));
                if dirty && !self.quit_confirm {
                    self.quit_confirm = true;
                    self.flash("unsaved changes in the editor · Ctrl+G q again to quit");
                } else {
                    self.quit = true;
                }
            }
            _ => {}
        }
    }

    /// Copies the last block's output to the clipboard.
    fn copy_last_output(&mut self) {
        let Some(block) = self.tab().blocks.last() else {
            self.flash("no output to copy");
            return;
        };
        let text = block.out.plain_text();
        let lines = text.lines().count();
        match self.set_clipboard(&text) {
            Ok(()) => self.flash(format!("copied {lines} lines to the clipboard")),
            Err(e) => self.flash(format!("could not copy: {e}")),
        }
    }

    pub fn set_clipboard(&mut self, text: &str) -> Result<(), String> {
        arboard::Clipboard::new()
            .and_then(|mut c| c.set_text(text.to_string()))
            .map_err(|e| e.to_string())
    }

    // ── Overlays ─────────────────────────────────────────────────────────────

    fn picker_items(&self, kind: PickerKind) -> Vec<Item> {
        match kind {
            PickerKind::History => {
                let mut seen = std::collections::HashSet::new();
                self.store
                    .history()
                    .filter(|h| seen.insert(h.command.as_str()))
                    .map(|h| Item {
                        detail: format!(
                            "{} · {}",
                            h.cwd.as_deref().map(short_path).unwrap_or_default(),
                            relative_time(h.ts)
                        ),
                        value: h.command.clone(),
                        label: h.command.clone(),
                    })
                    .collect()
            }
            PickerKind::Snippets => self
                .store
                .snippets()
                .iter()
                .map(|s| Item {
                    detail: if s.label == s.command {
                        String::new()
                    } else {
                        s.command.clone()
                    },
                    label: s.label.clone(),
                    value: s.command.clone(),
                })
                .collect(),
            PickerKind::Help => help_items(self.store.paths()),
            PickerKind::Profiles => self
                .store
                .profiles()
                .iter()
                .map(|p| Item {
                    detail: format!(
                        "{} · {}",
                        p.shell,
                        if p.cwd.is_empty() { "~" } else { &p.cwd }
                    ),
                    label: p.name.clone(),
                    value: String::new(),
                })
                .collect(),
        }
    }

    fn open_picker(&mut self, kind: PickerKind) {
        let items = self.picker_items(kind);
        self.overlay = Some(Overlay::Picker(Picker::new(kind, items)));
    }

    fn overlay_key(&mut self, k: KeyEvent) {
        match self.overlay.as_mut() {
            Some(Overlay::Picker(p)) => {
                let kind = p.kind;
                let action = p.handle_key(k);
                self.picker_action(kind, action);
            }
            Some(Overlay::Form(f)) => match f.handle_key(k) {
                FormAction::None => {}
                FormAction::Cancel => {
                    let kind = picker_for(f.kind);
                    self.open_picker(kind);
                }
                FormAction::Submit => self.submit_form(),
            },
            None => {}
        }
    }

    fn picker_action(&mut self, kind: PickerKind, action: PickerAction) {
        let Some(Overlay::Picker(p)) = self.overlay.as_ref() else {
            return;
        };
        match action {
            PickerAction::None => {}
            PickerAction::Close => self.overlay = None,
            PickerAction::Choose(i) => {
                let item = &p.items[i];
                match kind {
                    PickerKind::History | PickerKind::Snippets => {
                        let value = item.value.clone();
                        self.overlay = None;
                        let tab = self.tab_mut();
                        tab.input.set(&value);
                        tab.focus = Focus::Terminal;
                    }
                    // Commands run (or wait for their argument); shortcuts
                    // are reference only.
                    PickerKind::Help => {
                        let Some(name) = item.value.strip_prefix('/') else {
                            return;
                        };
                        let name = name.to_string();
                        let needs_arg = item.label.contains('<');
                        self.overlay = None;
                        if needs_arg {
                            self.tab_mut().input.set(&format!("/{name} "));
                        } else {
                            self.run_slash(&name, "");
                        }
                    }
                    PickerKind::Profiles => {
                        let name = item.label.clone();
                        self.overlay = None;
                        if let Some(prof) = self.store.profile(&name).cloned() {
                            self.new_tab(Some(prof.shell), Some(prof.cwd), Some(prof.name));
                        }
                    }
                }
            }
            PickerAction::New => {
                let form = match kind {
                    PickerKind::Profiles => {
                        Form::profile(&shell::default_shell(), &short_path(&self.tab().cwd))
                    }
                    _ => Form::snippet(&self.tab().input.text),
                };
                self.overlay = Some(Overlay::Form(form));
            }
            PickerAction::Delete(i) => {
                let key = p.items[i].label.clone();
                let res = match kind {
                    PickerKind::Profiles => self.store.delete_profile(&key),
                    PickerKind::Snippets => self.store.delete_snippet(&key),
                    _ => Ok(()),
                };
                if let Err(e) = res {
                    self.flash(format!("could not delete: {e}"));
                }
                self.refresh_picker();
            }
            PickerAction::ClearAll => {
                if let Err(e) = self.store.clear_history() {
                    self.flash(format!("could not clear history: {e}"));
                }
                self.refresh_picker();
            }
        }
    }

    /// Reloads the open picker's items, keeping the filter.
    fn refresh_picker(&mut self) {
        let Some(Overlay::Picker(p)) = self.overlay.as_ref() else {
            return;
        };
        let kind = p.kind;
        let items = self.picker_items(kind);
        if let Some(Overlay::Picker(p)) = self.overlay.as_mut() {
            p.items = items;
            p.selected = p.selected.min(p.visible().len().saturating_sub(1));
        }
    }

    fn submit_form(&mut self) {
        let Some(Overlay::Form(f)) = self.overlay.as_mut() else {
            return;
        };
        let res = match f.kind {
            FormKind::Profile => {
                let (name, shell_path, cwd) = (f.value(0), f.value(1), f.value(2));
                if name.is_empty() {
                    f.error = Some("name is required".into());
                    return;
                }
                let shell_path = if shell_path.is_empty() {
                    shell::default_shell()
                } else {
                    shell_path
                };
                self.store.add_profile(&name, &shell_path, &cwd)
            }
            FormKind::Snippet => {
                let (label, command) = (f.value(0), f.value(1));
                if command.is_empty() {
                    f.error = Some("command is required".into());
                    return;
                }
                let label = if label.is_empty() {
                    command.clone()
                } else {
                    label
                };
                self.store.add_snippet(&label, &command)
            }
        };
        let kind = picker_for(f.kind);
        match res {
            Ok(()) => self.open_picker(kind),
            Err(e) => f.error = Some(format!("could not save: {e}")),
        }
    }

    // ── Mouse and paste ──────────────────────────────────────────────────────

    fn on_paste(&mut self, text: &str) {
        match self.overlay.as_mut() {
            Some(Overlay::Picker(p)) => {
                p.filter.insert_str(&text.replace(['\r', '\n'], " "));
                p.selected = 0;
                return;
            }
            Some(Overlay::Form(f)) => {
                let focus = f.focus;
                f.fields[focus]
                    .1
                    .insert_str(&text.replace(['\r', '\n'], " "));
                return;
            }
            None => {}
        }
        let tab = self.tab_mut();
        if tab.has_editor() && tab.focus == Focus::Editor {
            if let Some(ed) = tab.editor.as_mut() {
                ed.insert_str(text);
            }
        } else if tab.busy() {
            let body = text.replace("\r\n", "\r").replace('\n', "\r");
            let bytes = if tab.session.vt.screen().bracketed_paste() {
                format!("\x1b[200~{body}\x1b[201~")
            } else {
                body
            };
            tab.session.write(bytes.as_bytes());
        } else {
            // The prompt is single-line: multiple lines are joined with `;`.
            let joined = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("; ");
            tab.input.insert_str(&joined);
        }
    }
}

/// Interactive commands that do not use the alternate screen (ssh, REPLs,
/// database clients, nested shells…): shown as a full terminal while they
/// run.
fn interactive(cmd: &str) -> bool {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let Some(first) = words.first() else {
        return false;
    };
    let prog = first.rsplit('/').next().unwrap_or(first);
    let args = &words[1..];
    let only_flags = args.iter().all(|a| a.starts_with('-'));
    let has = |opts: &[&str]| args.iter().any(|a| opts.contains(a));
    let tty = has(&["-it", "-ti"]) || (has(&["-i", "--interactive"]) && has(&["-t", "--tty"]));
    match prog {
        "ssh" => ssh_destination(args).is_some_and(|(_, has_cmd)| !has_cmd),
        "mosh" | "telnet" | "sftp" | "ftp" | "psql" | "mysql" | "mariadb" | "sqlite3"
        | "redis-cli" | "mongosh" | "mongo" | "su" => true,
        "docker" | "podman" => matches!(args.first(), Some(&"exec" | &"run")) && tty,
        "kubectl" => args.first() == Some(&"exec") && tty,
        "sudo" => matches!(args.first(), Some(&"-i" | &"-s" | &"su")),
        "bash" | "zsh" | "fish" | "sh" | "dash" | "ksh" | "python" | "python3" | "node" | "irb"
        | "ghci" | "lua" | "iex" | "julia" | "R" | "deno" | "bun" => only_flags,
        _ => false,
    }
}

/// Position of the destination in the `ssh` args and whether a remote command
/// follows it (then it is not an interactive session).
fn ssh_destination(args: &[&str]) -> Option<(usize, bool)> {
    // ssh options that take a value in the next argument.
    const WITH_VALUE: &str = "BbcDEeFIiJLlmOoPpQRSWw";
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if let Some(opt) = a.strip_prefix('-') {
            if opt.len() == 1 && WITH_VALUE.contains(opt) {
                i += 1;
            }
            i += 1;
            continue;
        }
        return Some((i, i + 1 < args.len()));
    }
    None
}

/// `ssh [options] host` → `__cairn_ssh [options] host` (remote integration).
/// `command ssh …` or `ssh host command` are left as is.
fn ssh_rewrite(cmd: &str) -> Option<String> {
    let rest = cmd.strip_prefix("ssh ")?;
    let args: Vec<&str> = rest.split_whitespace().collect();
    match ssh_destination(&args) {
        Some((_, false)) => Some(format!("__cairn_ssh {rest}")),
        _ => None,
    }
}

fn picker_for(kind: FormKind) -> PickerKind {
    match kind {
        FormKind::Profile => PickerKind::Profiles,
        FormKind::Snippet => PickerKind::Snippets,
    }
}

fn expand_home(p: &str) -> String {
    if (p == "~" || p.starts_with("~/"))
        && let Some(home) = dirs::home_dir()
    {
        return format!("{}{}", home.to_string_lossy(), &p[1..]);
    }
    p.to_string()
}

fn resolve_path(arg: &str, cwd: &str) -> PathBuf {
    let arg = arg.trim_matches(|c| c == '"' || c == '\'');
    let p = PathBuf::from(expand_home(arg));
    if p.is_absolute() {
        p
    } else {
        PathBuf::from(cwd).join(p)
    }
}

fn relative_time(ts_ms: i64) -> String {
    let secs = (chrono::Utc::now().timestamp_millis() - ts_ms).max(0) / 1000;
    match secs {
        0..60 => "now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcut_keycaps() {
        use super::keycaps;
        assert_eq!(keycaps("↑↓"), ["↑", "↓"]);
        assert_eq!(keycaps("Esc"), ["esc"]);
        assert_eq!(keycaps("⌃C"), ["⌃C"]);
    }

    #[test]
    fn slash_commands() {
        use super::slash_command;
        assert_eq!(slash_command("/tab 2"), None);
        assert_eq!(slash_command("/Help"), Some(("help", "")));
        assert_eq!(
            slash_command("/editor  notes.md "),
            Some(("editor", "notes.md"))
        );
        // Absolute paths and partial prefixes go to the shell.
        assert_eq!(slash_command("/usr/bin/ls -la"), None);
        assert_eq!(slash_command("/hel"), None);
        assert_eq!(slash_command("ls /tab"), None);
    }

    #[test]
    fn help_lists_commands_and_files() {
        use super::{COMMANDS, help_items};
        let paths = crate::store::Paths::resolve(std::path::Path::new("/home/j"), None, None, None);
        let items = help_items(&paths);
        for c in COMMANDS {
            assert!(
                items.iter().any(|i| i.value == format!("/{}", c.name)),
                "missing /{}",
                c.name
            );
        }
        // Commands only: no keyboard shortcuts.
        assert!(items.iter().all(|i| !i.label.starts_with('⌃')));
        // Files, with their real path, and nothing to run when chosen.
        let file = |name: &str| items.iter().find(|i| i.label == name).unwrap();
        assert!(
            file("config.toml")
                .detail
                .contains("/home/j/.config/cairn/config.toml")
        );
        assert!(
            file("history.jsonl")
                .detail
                .contains("/home/j/.local/share/cairn/history.jsonl")
        );
        assert!(file("config.toml").value.is_empty());
    }

    #[test]
    fn interactive_sessions() {
        use super::interactive;
        assert!(interactive("ssh root@1.2.3.4"));
        assert!(interactive("ssh -p 2222 -i ~/.ssh/k user@host"));
        assert!(!interactive("ssh host uptime"));
        assert!(interactive("docker exec -it web bash"));
        assert!(!interactive("docker exec web ls"));
        assert!(interactive("python3"));
        assert!(!interactive("python3 script.py"));
        assert!(interactive("sudo -i"));
        assert!(!interactive("ls -la"));
    }

    #[test]
    fn ssh_rewriting() {
        use super::ssh_rewrite;
        assert_eq!(
            ssh_rewrite("ssh -p 22 root@h").as_deref(),
            Some("__cairn_ssh -p 22 root@h")
        );
        assert_eq!(ssh_rewrite("ssh h uptime"), None);
        assert_eq!(ssh_rewrite("command ssh h"), None);
    }
}
