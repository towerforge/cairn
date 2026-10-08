use ratatui::crossterm::event::Event;

/// Everything that reaches the main loop, through a single channel.
pub enum AppEvent {
    /// Keyboard or paste, translated from the window's events.
    Term(Event),
    /// PTY output from a tab.
    Pty(u64, Vec<u8>),
    /// A tab's shell has exited.
    PtyClosed(u64),
    /// Recomputed git stats for a tab.
    Git(u64, Option<crate::git::DiffStat>),
    /// From the update threads: a newer release, or how `/update` is going.
    Update(crate::update::Notice),
}

/// Sink for the events produced by the PTY threads: the winit event loop
/// proxy, which also wakes the window. Returns `false` if the receiver no longer exists.
pub type EventSink = std::sync::Arc<dyn Fn(AppEvent) -> bool + Send + Sync>;
