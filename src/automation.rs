use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

#[cfg(target_os = "macos")]
pub mod ghostty;
#[cfg(target_os = "macos")]
pub mod iterm2;
pub mod tmux;

#[cfg(target_os = "macos")]
mod macos;

/// A terminal session that can be addressed by an automation implementation.
///
/// A terminal represents a leaf pane and its location within the containing
/// window and tab.
#[allow(clippy::struct_field_names)] // Keep the automation JSON field name unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Terminal {
    /// Identifier used to address the terminal.
    pub id: String,

    /// Name reported by the terminal application.
    pub name: String,

    /// Working directory reported by the terminal application.
    pub working_directory: String,

    /// One-based index of the containing window.
    pub window_index: usize,

    /// One-based index of the containing tab within its window.
    pub tab_index: usize,

    /// One-based index of the terminal within its tab.
    pub terminal_index: usize,
}

/// Discovers terminal sessions and sends text to them.
#[async_trait]
pub trait TerminalAutomation: Send + Sync {
    /// Lists the terminal sessions that can currently receive text.
    ///
    /// A returned terminal may become unavailable after this method returns.
    async fn list_targets(&self) -> Result<Vec<Terminal>>;

    /// Sends `text` to `target` without submitting it.
    ///
    /// `target` must be a terminal returned by [`list_targets`](Self::list_targets)
    /// from the same automation implementation. This method does not send Enter.
    async fn send_text(&self, target: &Terminal, text: &str) -> Result<()>;

    /// Sends Enter to `target`.
    ///
    /// `target` must be a terminal returned by [`list_targets`](Self::list_targets)
    /// from the same automation implementation.
    async fn send_enter(&self, target: &Terminal) -> Result<()>;
}
