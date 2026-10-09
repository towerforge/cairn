//! Overlays: filterable pickers and creation forms.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::edit::input::LineInput;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    History,
    Snippets,
    Profiles,
    Help,
}

impl PickerKind {
    pub fn title(self) -> &'static str {
        match self {
            PickerKind::History => "History",
            PickerKind::Snippets => "Snippets",
            PickerKind::Profiles => "Profiles",
            PickerKind::Help => concat!("Help · cairn ", env!("CARGO_PKG_VERSION")),
        }
    }

    /// Supports creating, editing and deleting items.
    fn editable(self) -> bool {
        matches!(self, PickerKind::Snippets | PickerKind::Profiles)
    }

    /// Footer shortcuts: (key, action).
    pub fn hints(self) -> &'static [(&'static str, &'static str)] {
        match self {
            PickerKind::History => &[
                ("↵", "insert"),
                ("↑↓", "move"),
                ("⌃K", "clear history"),
                ("Esc", "close"),
            ],
            PickerKind::Snippets => &[
                ("↵", "insert"),
                ("⌃N", "new"),
                ("⌃E", "edit"),
                ("⌃D", "delete"),
                ("Esc", "close"),
            ],
            PickerKind::Profiles => &[
                ("↵", "open tab"),
                ("⌃S", "at startup"),
                ("⌃N", "new"),
                ("⌃E", "edit"),
                ("⌃D", "delete"),
                ("Esc", "close"),
            ],
            PickerKind::Help => &[("↵", "run command"), ("↑↓", "move"), ("Esc", "close")],
        }
    }
}

/// For profiles and snippets, `label` is also their key (name or label).
pub struct Item {
    pub label: String,
    pub detail: String,
    /// Text inserted into the prompt when chosen.
    pub value: String,
}

pub struct Picker {
    pub kind: PickerKind,
    pub filter: LineInput,
    pub items: Vec<Item>,
    pub selected: usize,
    pub confirm: bool,
}

pub enum PickerAction {
    None,
    Close,
    Choose(usize),
    New,
    Edit(usize),
    Delete(usize),
    /// Profiles: make it the startup profile, or stop if it already is.
    ToggleDefault(usize),
    ClearAll,
}

impl Picker {
    pub fn new(kind: PickerKind, items: Vec<Item>) -> Self {
        Self {
            kind,
            filter: LineInput::default(),
            items,
            selected: 0,
            confirm: false,
        }
    }

    /// Indices of the items that pass the filter.
    pub fn visible(&self) -> Vec<usize> {
        let f = self.filter.text.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                f.is_empty()
                    || it.label.to_lowercase().contains(&f)
                    || it.detail.to_lowercase().contains(&f)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let visible = self.visible();
        let was_confirm = std::mem::take(&mut self.confirm);
        match key.code {
            KeyCode::Esc => return PickerAction::Close,
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(visible.len().saturating_sub(1))
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(visible.len().saturating_sub(1))
            }
            KeyCode::Enter => {
                if let Some(&i) = visible.get(self.selected) {
                    return PickerAction::Choose(i);
                }
            }
            KeyCode::Char('n') if ctrl && self.kind.editable() => {
                return PickerAction::New;
            }
            KeyCode::Char('e') if ctrl && self.kind.editable() => {
                if let Some(&i) = visible.get(self.selected) {
                    return PickerAction::Edit(i);
                }
            }
            KeyCode::Char('d') if ctrl && self.kind.editable() => {
                if let Some(&i) = visible.get(self.selected) {
                    return PickerAction::Delete(i);
                }
            }
            KeyCode::Char('s') if ctrl && self.kind == PickerKind::Profiles => {
                if let Some(&i) = visible.get(self.selected) {
                    return PickerAction::ToggleDefault(i);
                }
            }
            KeyCode::Char('k') if ctrl && self.kind == PickerKind::History => {
                if was_confirm {
                    return PickerAction::ClearAll;
                }
                self.confirm = true;
            }
            _ => {
                if self.filter.handle_key(key) {
                    self.selected = 0;
                }
            }
        }
        PickerAction::None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FormKind {
    Profile,
    Snippet,
}

pub struct Form {
    pub kind: FormKind,
    pub fields: Vec<(&'static str, LineInput)>,
    pub focus: usize,
    pub error: Option<String>,
    /// Name (profile) or label (snippet) of the item being edited; `None`
    /// for a new one.
    pub editing: Option<String>,
}

pub enum FormAction {
    None,
    Cancel,
    Submit,
}

impl Form {
    pub fn profile(shell: &str, cwd: &str) -> Self {
        Self {
            kind: FormKind::Profile,
            fields: vec![
                ("Name", LineInput::default()),
                ("Shell", LineInput::with(shell)),
                ("Directory", LineInput::with(cwd)),
            ],
            focus: 0,
            error: None,
            editing: None,
        }
    }

    /// The form of an existing profile, filled in with it.
    pub fn edit_profile(name: &str, shell: &str, cwd: &str) -> Self {
        let mut f = Self::profile(shell, cwd);
        f.fields[0].1 = LineInput::with(name);
        f.editing = Some(name.to_string());
        f
    }

    /// The form of an existing snippet, filled in with it.
    pub fn edit_snippet(label: &str, command: &str) -> Self {
        let mut f = Self::snippet(command);
        f.fields[0].1 = LineInput::with(label);
        f.editing = Some(label.to_string());
        f
    }

    pub fn snippet(command: &str) -> Self {
        Self {
            kind: FormKind::Snippet,
            fields: vec![
                ("Label", LineInput::default()),
                ("Command", LineInput::with(command)),
            ],
            focus: 0,
            error: None,
            editing: None,
        }
    }

    pub fn title(&self) -> &'static str {
        match (self.kind, self.editing.is_some()) {
            (FormKind::Profile, false) => "New profile",
            (FormKind::Profile, true) => "Edit profile",
            (FormKind::Snippet, false) => "New snippet",
            (FormKind::Snippet, true) => "Edit snippet",
        }
    }

    pub fn value(&self, i: usize) -> String {
        self.fields[i].1.text.trim().to_string()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> FormAction {
        let n = self.fields.len();
        match key.code {
            KeyCode::Esc => return FormAction::Cancel,
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % n,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + n - 1) % n,
            KeyCode::Enter => {
                if self.focus + 1 < n {
                    self.focus += 1;
                } else {
                    return FormAction::Submit;
                }
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return FormAction::Submit;
            }
            _ => {
                self.fields[self.focus].1.handle_key(key);
            }
        }
        FormAction::None
    }
}

pub enum Overlay {
    Picker(Picker),
    Form(Form),
}
