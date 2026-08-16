//! Automation support for Ghostty.

#[cfg(target_os = "macos")]
mod applescript;

/// Terminal automation provided by Ghostty.
pub struct GhosttyAutomation;

#[cfg(not(target_os = "macos"))]
compile_error!("Ghostty automation currently supports only macOS");
