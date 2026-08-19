mod automation;
mod cli;
mod input;
mod task;
mod template;
mod tui;
mod workflow;

use std::io;

use anyhow::{Context, Result};
use automation::{Terminal, TerminalAutomation, tmux::TmuxAutomation};
#[cfg(target_os = "macos")]
use automation::{ghostty::GhosttyAutomation, iterm2::ITerm2Automation};
use cli::{Cli, TerminalApplication};
use promkit::{
    TerminalModes, TerminalSession,
    core::crossterm::{
        cursor,
        event::{
            KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::supports_keyboard_enhancement,
    },
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let modes = TerminalModes::RAW_MODE
        | TerminalModes::ALTERNATE_SCREEN
        | TerminalModes::HIDDEN_CURSOR
        | TerminalModes::MOUSE_CAPTURE;
    let _terminal_session = TerminalSession::try_new(modes)?;
    let _keyboard_enhancement = KeyboardEnhancementSession::try_new()?;
    let mut screen = tui::InputScreen::from_workflow_path(&cli.workflow, cli.terminals.clone())?;

    loop {
        // Keep each reset form anchored at the same origin while the alternate
        // screen and renderer remain active across submissions.
        execute!(io::stdout(), cursor::MoveTo(0, 0))?;
        let Some(submission) = screen.run().await? else {
            return Ok(());
        };

        let markdown = submission.workflow.render(&submission.values)?;
        match submission.target.application {
            #[cfg(target_os = "macos")]
            TerminalApplication::Ghostty => {
                deliver(&GhosttyAutomation, &submission.target.terminal, &markdown).await
            }
            #[cfg(target_os = "macos")]
            TerminalApplication::Iterm2 => {
                deliver(&ITerm2Automation, &submission.target.terminal, &markdown).await
            }
            TerminalApplication::Tmux => {
                deliver(&TmuxAutomation, &submission.target.terminal, &markdown).await
            }
        }
        .context("failed to deliver rendered Markdown")?;

        screen.reset(&cli.workflow, cli.terminals.clone())?;
    }
}

struct KeyboardEnhancementSession {
    enabled: bool,
}

impl KeyboardEnhancementSession {
    fn try_new() -> io::Result<Self> {
        let enabled = supports_keyboard_enhancement().unwrap_or(false);
        if enabled {
            execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        Ok(Self { enabled })
    }
}

impl Drop for KeyboardEnhancementSession {
    fn drop(&mut self) {
        if self.enabled {
            execute!(io::stdout(), PopKeyboardEnhancementFlags).ok();
        }
    }
}

async fn deliver(
    automation: &impl TerminalAutomation,
    target: &Terminal,
    text: &str,
) -> Result<()> {
    automation.send_text(target, text).await?;
    automation.send_enter(target).await
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct RecordingAutomation {
        calls: Mutex<Vec<String>>,
        fail_text: bool,
    }

    #[async_trait::async_trait]
    impl TerminalAutomation for RecordingAutomation {
        async fn list_targets(&self) -> Result<Vec<Terminal>> {
            unreachable!()
        }

        async fn send_text(&self, _target: &Terminal, text: &str) -> Result<()> {
            self.calls.lock().unwrap().push(format!("text:{text}"));
            if self.fail_text {
                anyhow::bail!("text failed");
            }
            Ok(())
        }

        async fn send_enter(&self, _target: &Terminal) -> Result<()> {
            self.calls.lock().unwrap().push("enter".into());
            Ok(())
        }
    }

    #[tokio::test]
    async fn sends_enter_after_text() {
        let automation = RecordingAutomation {
            calls: Mutex::new(Vec::new()),
            fail_text: false,
        };
        let target = Terminal {
            id: "target".into(),
            name: "shell".into(),
            working_directory: "/workspace".into(),
            window_index: 1,
            tab_index: 1,
            terminal_index: 1,
        };

        deliver(&automation, &target, "hello").await.unwrap();

        assert_eq!(*automation.calls.lock().unwrap(), ["text:hello", "enter"]);
    }

    #[tokio::test]
    async fn does_not_send_enter_when_sending_text_fails() {
        let automation = RecordingAutomation {
            calls: Mutex::new(Vec::new()),
            fail_text: true,
        };
        let target = Terminal {
            id: "target".into(),
            name: "shell".into(),
            working_directory: "/workspace".into(),
            window_index: 1,
            tab_index: 1,
            terminal_index: 1,
        };

        assert!(deliver(&automation, &target, "hello").await.is_err());
        assert_eq!(*automation.calls.lock().unwrap(), ["text:hello"]);
    }
}
