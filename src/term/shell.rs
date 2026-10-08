//! Shell integration: makes the shell mark where each command starts and ends
//! with OSC 133 sequences, and the current directory with OSC 7.
//!
//!   ESC]133;A BEL        prompt ready (the shell is waiting for a command)
//!   ESC]133;C BEL        command output starts
//!   ESC]133;D;<exit> BEL the command has finished
//!   ESC]7;file://host/path BEL
//!
//! The marks are emitted by the shell's own hooks (precmd/preexec in zsh,
//! PROMPT_COMMAND and PS0 in bash, events in fish). Nothing is ever written to
//! standard input, so `read`, `sudo` or a REPL never swallow anything.
//! OSC 133 is a widely supported standard.

use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    /// sh, dash, ksh…: no preexec hook. The prompt marks the end, and the
    /// command is assumed to start when it is sent.
    Other,
}

impl ShellKind {
    pub fn detect(shell: &str) -> Self {
        let base = basename(shell);
        if base.starts_with("zsh") {
            ShellKind::Zsh
        } else if base.starts_with("bash") {
            ShellKind::Bash
        } else if base.starts_with("fish") {
            ShellKind::Fish
        } else {
            ShellKind::Other
        }
    }

    /// The shell does not emit `133;C`: the command starts when sent.
    pub fn implicit_start(self) -> bool {
        self == ShellKind::Other
    }
}

pub fn basename(shell: &str) -> &str {
    shell.rsplit('/').next().unwrap_or(shell)
}

/// The user's shell: `$SHELL` and, if unset (apps launched from Finder or the
/// desktop), their account's shell from `/etc/passwd` / Directory Services.
pub fn default_shell() -> String {
    if let Some(s) = std::env::var("SHELL").ok().filter(|s| !s.is_empty()) {
        return s;
    }
    if let Some(s) = account_shell() {
        return s;
    }
    if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else {
        "/bin/bash"
    }
    .into()
}

fn account_shell() -> Option<String> {
    // SAFETY: getpwuid returns a pointer to libc static memory or NULL;
    // the string is copied immediately.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_shell.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr((*pw).pw_shell)
            .to_string_lossy()
            .into_owned();
        (!shell.is_empty()).then_some(shell)
    }
}

/// Builds the command that launches the shell with the integration loaded.
pub fn command(shell: &str) -> anyhow::Result<(CommandBuilder, ShellKind)> {
    let kind = ShellKind::detect(shell);
    let dir = integration_dir();
    let mut cmd = CommandBuilder::new(shell);

    match kind {
        ShellKind::Zsh => {
            let zdir = dir.join("zsh");
            write(&zdir.join(".zshenv"), ZSHENV)?;
            write(&zdir.join(".zprofile"), ZPROFILE)?;
            write(&zdir.join(".zshrc"), &format!("{ZSHRC}{SSH_POSIX}"))?;
            let user_zdotdir = std::env::var("ZDOTDIR")
                .ok()
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_default();
            cmd.env("CAIRN_USER_ZDOTDIR", user_zdotdir);
            cmd.env("ZDOTDIR", &zdir);
            cmd.arg("-l");
        }
        ShellKind::Bash => {
            let rc = dir.join("cairn.bash");
            write(&rc, &format!("{BASHRC}{SSH_POSIX}"))?;
            cmd.arg("--rcfile");
            cmd.arg(&rc);
            cmd.arg("-i");
        }
        ShellKind::Fish => {
            let rc = dir.join("cairn.fish");
            write(&rc, &format!("{FISH}{SSH_FISH}"))?;
            let quoted = rc
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('\'', "\\'");
            cmd.arg("-l");
            cmd.arg("--init-command");
            cmd.arg(format!("source '{quoted}'"));
        }
        ShellKind::Other => {
            let rc = dir.join("cairn.sh");
            write(&rc, SHRC)?;
            cmd.env("ENV", &rc);
            cmd.arg("-i");
        }
    }

    // Bootstrap script for remote servers (used by __cairn_ssh).
    write(&dir.join("remote.sh"), &remote_script())?;
    cmd.env("CAIRN_INTEGRATION_DIR", &dir);

    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "Cairn");
    cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    // GUI apps don't inherit a locale: without this the shell doesn't handle UTF-8.
    let has_locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .any(|v| std::env::var(v).is_ok_and(|x| !x.is_empty()));
    if !has_locale {
        if cfg!(target_os = "macos") {
            cmd.env("LC_CTYPE", "UTF-8");
        } else {
            cmd.env("LANG", "C.UTF-8");
        }
    }
    cmd.env_remove("TERM_SESSION_ID");
    cmd.env_remove("ITERM_SESSION_ID");
    Ok((cmd, kind))
}

/// This machine's name, to tell a local directory from a remote one in
/// OSC 7 marks (`file://host/path`).
pub fn local_hostname() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let mut buf = [0u8; 256];
        // SAFETY: gethostname writes at most buf.len() bytes into buf.
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        if !ok {
            return String::new();
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    })
}

/// `true` if the host of an OSC 7 mark is another machine.
pub fn is_remote_host(host: &str) -> bool {
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_lowercase();
    !host.is_empty() && host != "localhost" && short(host) != short(local_hostname())
}

fn integration_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("cairn")
        .join("shell")
}

/// Writes the file only if it changed (several tabs start at once).
fn write(path: &Path, content: &str) -> anyhow::Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|c| c == content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

// ── zsh ──────────────────────────────────────────────────────────────────────
// Cairn points ZDOTDIR at its own directory; each file sources the user's
// original and the end of .zshrc restores their ZDOTDIR (so their .zlogin is
// read normally).

const ZSHENV: &str = r#"# Cairn: auto-generated, do not edit.
__cairn_zdotdir="$ZDOTDIR"
ZDOTDIR="$CAIRN_USER_ZDOTDIR"
[[ -f "$ZDOTDIR/.zshenv" ]] && source "$ZDOTDIR/.zshenv"
CAIRN_USER_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$__cairn_zdotdir"
"#;

const ZPROFILE: &str = r#"# Cairn: auto-generated, do not edit.
__cairn_zdotdir="$ZDOTDIR"
ZDOTDIR="$CAIRN_USER_ZDOTDIR"
[[ -f "$ZDOTDIR/.zprofile" ]] && source "$ZDOTDIR/.zprofile"
ZDOTDIR="$__cairn_zdotdir"
"#;

const ZSHRC: &str = r#"# Cairn: auto-generated, do not edit.
ZDOTDIR="$CAIRN_USER_ZDOTDIR"
# zsh set HISTFILE from Cairn's ZDOTDIR at startup: the history is the
# user's (their .zshrc may change it later).
[[ -z "$HISTFILE" || "$HISTFILE" == "$__cairn_zdotdir"/* ]] && HISTFILE="$ZDOTDIR/.zsh_history"
[[ -f "$ZDOTDIR/.zshrc" ]] && source "$ZDOTDIR/.zshrc"
unset __cairn_zdotdir CAIRN_USER_ZDOTDIR
# PROMPT_SP prints a "%" and spaces after each command: useless in Cairn.
unsetopt PROMPT_SP

__cairn_precmd() {
  local ec=$?
  printf '\033]133;D;%s\007\033]7;file://%s%s\007\033]133;A\007' "$ec" "$HOST" "$PWD"
}
__cairn_preexec() {
  printf '\033]133;C\007'
}
# precmd goes first to capture the command's $? before other hooks.
precmd_functions=(__cairn_precmd $precmd_functions)
preexec_functions+=(__cairn_preexec)
"#;

// ── bash ─────────────────────────────────────────────────────────────────────

const BASHRC: &str = r#"# Cairn: auto-generated, do not edit.
# Emulate a login shell, then load the hooks.
[ -f /etc/profile ] && . /etc/profile
if [ -f "$HOME/.bash_profile" ]; then . "$HOME/.bash_profile"
elif [ -f "$HOME/.bash_login" ]; then . "$HOME/.bash_login"
elif [ -f "$HOME/.profile" ]; then . "$HOME/.profile"
elif [ -f "$HOME/.bashrc" ]; then . "$HOME/.bashrc"
fi

__cairn_in_cmd=1
__cairn_precmd() {
  local ec=$?
  __cairn_in_cmd=1
  printf '\033]133;D;%s\007\033]7;file://%s%s\007' "$ec" "$HOSTNAME" "$PWD"
}
__cairn_arm() {
  __cairn_in_cmd=
  printf '\033]133;A\007'
}
if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  PROMPT_COMMAND=(__cairn_precmd "${PROMPT_COMMAND[@]}" __cairn_arm)
else
  PROMPT_COMMAND="__cairn_precmd"$'\n'"${PROMPT_COMMAND}"$'\n'"__cairn_arm"
fi

if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
  PS0=$'\033]133;C\007'"${PS0}"
else
  # bash < 4.4 (the one in macOS) has no PS0: use the DEBUG trap.
  __cairn_debug() {
    [ -n "$__cairn_in_cmd" ] && return
    __cairn_in_cmd=1
    printf '\033]133;C\007'
  }
  trap '__cairn_debug' DEBUG
fi
"#;

// ── fish ─────────────────────────────────────────────────────────────────────

const FISH: &str = r#"# Cairn: auto-generated, do not edit.
function __cairn_preexec --on-event fish_preexec
    printf '\e]133;C\a'
end
function __cairn_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
end
function __cairn_prompt --on-event fish_prompt
    printf '\e]7;file://%s%s\a\e]133;A\a' $hostname $PWD
end
"#;

// ── ssh with remote integration ──────────────────────────────────────────────
// Cairn rewrites `ssh host` as `__cairn_ssh host`: the server receives remote.sh
// in base64, runs it with sh, and it opens the user's shell with the same
// hooks. If the server can't (no sh or base64), its normal shell opens.

const SSH_POSIX: &str = r#"
__cairn_ssh() {
  local b r
  b=$(base64 < "$CAIRN_INTEGRATION_DIR/remote.sh" | tr -d '\n')
  r="sh -c 'S=\$(echo $b | base64 -d 2>/dev/null) && [ -n \"\$S\" ] && eval \"\$S\"; exec \"\${SHELL:-/bin/sh}\" -l'"
  command ssh -t "$@" "$r"
}
"#;

const SSH_FISH: &str = r#"
function __cairn_ssh
    set -l b (base64 < $CAIRN_INTEGRATION_DIR/remote.sh | tr -d '\n')
    set -l r "sh -c 'S=\$(echo $b | base64 -d 2>/dev/null) && [ -n \"\$S\" ] && eval \"\$S\"; exec \"\${SHELL:-/bin/sh}\" -l'"
    command ssh -t $argv $r
end
"#;

/// Remote bootstrap script (sh): writes the integration for the user's shell
/// to a temporary directory and opens the shell with it. The files delete
/// themselves once loaded.
fn remote_script() -> String {
    const EOF: &str = "CAIRN_EOF_7F3A";
    const CLEAN: &str = "\n[ -n \"$CAIRN_TMP\" ] && rm -rf \"$CAIRN_TMP\"; unset CAIRN_TMP\n";
    const CLEAN_FISH: &str =
        "\nif test -n \"$CAIRN_TMP\"; rm -rf $CAIRN_TMP; set -e CAIRN_TMP; end\n";
    format!(
        r#"# Cairn: remote integration, auto-generated.
CAIRN_TMP=$(mktemp -d 2>/dev/null) || CAIRN_TMP=/tmp/cairn.$$
mkdir -p "$CAIRN_TMP" || exit 1
export CAIRN_TMP
s=${{SHELL:-/bin/sh}}
case "$s" in
*zsh)
cat > "$CAIRN_TMP/.zshenv" <<'{EOF}'
{ZSHENV}
{EOF}
cat > "$CAIRN_TMP/.zprofile" <<'{EOF}'
{ZPROFILE}
{EOF}
cat > "$CAIRN_TMP/.zshrc" <<'{EOF}'
{ZSHRC}{CLEAN}
{EOF}
CAIRN_USER_ZDOTDIR=${{ZDOTDIR:-$HOME}}; export CAIRN_USER_ZDOTDIR
ZDOTDIR=$CAIRN_TMP; export ZDOTDIR
exec "$s" -l ;;
*bash)
cat > "$CAIRN_TMP/cairn.bash" <<'{EOF}'
{BASHRC}{CLEAN}
{EOF}
exec "$s" --rcfile "$CAIRN_TMP/cairn.bash" -i ;;
*fish)
cat > "$CAIRN_TMP/cairn.fish" <<'{EOF}'
{FISH}{CLEAN_FISH}
{EOF}
exec "$s" -l --init-command "source $CAIRN_TMP/cairn.fish" ;;
*)
rm -rf "$CAIRN_TMP"
exec "$s" -l ;;
esac
"#
    )
}

// ── sh / dash / ksh ──────────────────────────────────────────────────────────

const SHRC: &str = r#"# Cairn: auto-generated, do not edit.
PS1="$(printf '\033]133;D;')"'$?'"$(printf '\007\033]7;file://')"'$PWD'"$(printf '\007\033]133;A\007')"
"#;
