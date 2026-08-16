//! Automation support for iTerm2.

#[cfg(target_os = "macos")]
mod applescript;

/// Terminal automation provided by iTerm2.
pub struct ITerm2Automation;

#[cfg(not(target_os = "macos"))]
compile_error!("iTerm2 automation currently supports only macOS");
