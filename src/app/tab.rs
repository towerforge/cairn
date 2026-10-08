//! Tab: a shell session, its command blocks and the prompt.

use std::time::{Duration, Instant};

use crate::edit::editor::Editor;
use crate::edit::input::LineInput;
use crate::git::DiffStat;
use crate::term::output::Output;
use crate::term::session::{Chunk, Mark, Session};

/// State of the command in the tab's shell.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CmdState {
    /// The shell is starting; it has not emitted its first prompt yet.
    Starting,
    /// Waiting for a command at the Cairn prompt.
    Idle,
    /// Command sent; the shell has not marked its start yet.
    Pending,
    /// The command is running.
    Running,
}

/// Maximum time to wait for the first prompt before treating the shell as ready.
const STARTUP_GRACE: Duration = Duration::from_secs(5);

pub struct CmdBlock {
    pub cmd: String,
    /// Context at launch: remote host, directory, branch and git changes.
    pub host: Option<String>,
    pub cwd: String,
    pub branch: Option<String>,
    pub git: Option<DiffStat>,
    /// Shells without a start hook: the tty echoes the command as the first
    /// output line and it must be stripped.
    strip_echo: bool,
    pub started: Instant,
    /// Exit code (if known) and duration, once finished.
    pub end: Option<(Option<i32>, Duration)>,
    pub out: Output,
    /// Row cache for the native window: (width in columns, version, rows).
    pub rows_cache: Option<(usize, u64, usize)>,
}

impl CmdBlock {
    pub fn running(&self) -> bool {
        self.end.is_none()
    }

    pub fn failed(&self) -> bool {
        matches!(self.end, Some((Some(code), _)) if code != 0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Terminal,
    Editor,
}

pub struct Tab {
    pub id: u64,
    pub session: Session,
    pub blocks: Vec<CmdBlock>,
    pub state: CmdState,
    pub cwd: String,
    pub branch: Option<String>,
    /// Remote host of the current shell (ssh with integration), if not local.
    pub remote: Option<String>,
    /// The running command is an interactive session (ssh, REPL…): shown as a
    /// full terminal even if it does not use the alternate screen.
    pub live: bool,
    /// Git changes of the current directory (computed in the background).
    pub git: Option<DiffStat>,
    /// A git computation is running / pending a rerun.
    pub git_pending: bool,
    pub git_dirty: bool,
    pub profile_name: Option<String>,
    pub input: LineInput,
    /// Block view scroll offset, in rows from the bottom.
    pub scroll: usize,
    /// ↑/↓ history of this tab.
    pub local_hist: Vec<String>,
    pub hist_pos: Option<usize>,
    pub editor: Option<Editor>,
    pub editor_visible: bool,
    pub focus: Focus,
    spawned: Instant,
    /// Pending notice for the message bar.
    pub notice: Option<String>,
}

impl Tab {
    pub fn new(id: u64, session: Session, cwd: String, profile: Option<String>) -> Self {
        Self {
            id,
            session,
            blocks: Vec::new(),
            state: CmdState::Starting,
            branch: crate::git::branch(&cwd),
            remote: None,
            live: false,
            git: None,
            git_pending: false,
            git_dirty: false,
            cwd,
            profile_name: profile,
            input: LineInput::default(),
            scroll: 0,
            local_hist: Vec::new(),
            hist_pos: None,
            editor: None,
            editor_visible: false,
            focus: Focus::Terminal,
            spawned: Instant::now(),
            notice: None,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.state, CmdState::Pending | CmdState::Running)
    }

    /// Full terminal view: a program on the alternate screen (vim, htop,
    /// less…) or an interactive session (ssh, psql…).
    pub fn fullscreen(&self) -> bool {
        self.state == CmdState::Running && (self.live || self.session.fullscreen())
    }

    pub fn has_editor(&self) -> bool {
        self.editor.is_some() && self.editor_visible
    }

    pub fn title(&self) -> String {
        if let Some(name) = &self.profile_name {
            return name.clone();
        }
        folder_name(&self.cwd)
    }

    /// Sends a command to the shell and opens its block. `display` is what the
    /// block shows; `send`, what the shell receives (e.g. `ssh` → `__cairn_ssh`).
    pub fn run(&mut self, display: &str, send: &str, live: bool) {
        let cmd = display;
        self.blocks.push(CmdBlock {
            cmd: cmd.to_string(),
            host: self.remote.clone(),
            cwd: self.cwd.clone(),
            branch: self.branch.clone(),
            git: self.git,
            strip_echo: self.session.kind.implicit_start(),
            started: Instant::now(),
            end: None,
            out: Output::default(),
            rows_cache: None,
        });
        self.scroll = 0;
        self.live = live;
        self.state = if self.session.kind.implicit_start() {
            CmdState::Running
        } else {
            CmdState::Pending
        };
        if live && self.state == CmdState::Running {
            self.session.clear_screen();
        }
        let mut line = send.as_bytes().to_vec();
        line.push(b'\r');
        self.session.write(&line);
    }

    /// Processes PTY output. Returns `true` if git stats should be recomputed
    /// (a command finished or the directory changed).
    pub fn on_output(&mut self, data: &[u8]) -> bool {
        let mut refresh_git = false;
        for chunk in self.session.scan(data) {
            match chunk {
                Chunk::Data(d) => {
                    self.session.process_vt(&d);
                    if self.state == CmdState::Running
                        && let Some(b) = self.blocks.last_mut()
                    {
                        b.out.feed(&d);
                        if b.strip_echo && b.out.row > 0 {
                            b.strip_echo = false;
                            b.out.drop_first_line_if(&b.cmd);
                        }
                    }
                }
                Chunk::Mark(Mark::Prompt) => match self.state {
                    CmdState::Starting => self.state = CmdState::Idle,
                    // Prompt without command end: e.g. a syntax error.
                    CmdState::Pending | CmdState::Running => self.finish(None),
                    CmdState::Idle => {}
                },
                Chunk::Mark(Mark::CommandStart) => {
                    if self.state == CmdState::Pending {
                        self.state = CmdState::Running;
                        if self.live {
                            self.session.clear_screen();
                        }
                    }
                }
                Chunk::Mark(Mark::CommandEnd(code)) => {
                    if self.busy() {
                        self.finish(code);
                        refresh_git = true;
                    }
                }
                Chunk::Mark(Mark::Cwd { host, path }) => {
                    if crate::term::shell::is_remote_host(&host) {
                        // Remote shell with integration: no local git.
                        let short = host.split('.').next().unwrap_or(&host).to_string();
                        self.remote = Some(short);
                        self.branch = None;
                        self.git = None;
                        self.cwd = path;
                    } else if path != self.cwd || self.remote.is_some() {
                        self.remote = None;
                        self.branch = crate::git::branch(&path);
                        self.cwd = path;
                        refresh_git = true;
                    } else if !self.busy() {
                        // Same directory: the branch may have changed (git checkout).
                        self.branch = crate::git::branch(&path);
                    }
                }
            }
        }
        refresh_git
    }

    fn finish(&mut self, code: Option<i32>) {
        if let Some(b) = self.blocks.last_mut()
            && b.end.is_none()
        {
            b.end = Some((code, b.started.elapsed()));
            b.out.version += 1;
        }
        self.state = CmdState::Idle;
        self.live = false;
        self.session.reset_screen();
    }

    /// Periodic tasks: if the shell never announces its prompt (integration not
    /// loaded), treat it as ready anyway so the tab is not blocked.
    pub fn tick(&mut self) {
        if self.state == CmdState::Starting && self.spawned.elapsed() > STARTUP_GRACE {
            self.state = CmdState::Idle;
            self.notice = Some("shell integration not detected: blocks may not close".into());
        }
    }
}

pub fn folder_name(cwd: &str) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    if dirs::home_dir().is_some_and(|h| h.as_os_str() == cwd) {
        return "~".into();
    }
    cwd.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("/")
        .to_string()
}

/// Short path for the context line: `$HOME` as `~`.
pub fn short_path(cwd: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if let Some(rest) = cwd.strip_prefix(home.as_ref())
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return format!("~{rest}");
        }
    }
    cwd.to_string()
}
