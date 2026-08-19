//! Automation support for tmux.

use std::collections::HashMap;
use std::process::Output;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::process::Command;

use super::{Terminal, TerminalAutomation};

const FIELD_SEPARATOR: char = '\u{1f}';
const LIST_FORMAT: &str = concat!(
    "#{session_id}\u{1f}",
    "#{session_name}\u{1f}",
    "#{window_id}\u{1f}",
    "#{window_name}\u{1f}",
    "#{pane_id}\u{1f}",
    "#{pane_title}\u{1f}",
    "#{pane_current_command}\u{1f}",
    "#{pane_current_path}",
);

/// Terminal automation provided by tmux.
pub struct TmuxAutomation;

#[async_trait]
impl TerminalAutomation for TmuxAutomation {
    async fn list_targets(&self) -> Result<Vec<Terminal>> {
        let output = execute(&["list-panes", "-a", "-F", LIST_FORMAT])
            .await
            .context("failed to list tmux targets")?;
        if !output.status.success() {
            if is_no_targets_error(&output.stderr) {
                return Ok(Vec::new());
            }
            return Err(command_error(&output.stderr)).context("failed to list tmux targets");
        }

        parse_targets(&output.stdout).context("invalid tmux target response")
    }

    async fn send_text(&self, target: &Terminal, text: &str) -> Result<()> {
        run(&["send-keys", "-t", &target.id, "-l", "--", text])
            .await
            .with_context(|| format!("failed to send text to tmux target {}", target.id))?;
        Ok(())
    }

    async fn send_enter(&self, target: &Terminal) -> Result<()> {
        run(&["send-keys", "-t", &target.id, "Enter"])
            .await
            .with_context(|| format!("failed to send Enter to tmux target {}", target.id))?;
        Ok(())
    }
}

async fn run(arguments: &[&str]) -> Result<Vec<u8>> {
    let output = execute(arguments).await?;
    if !output.status.success() {
        return Err(command_error(&output.stderr));
    }

    Ok(output.stdout)
}

async fn execute(arguments: &[&str]) -> Result<Output> {
    let mut command = Command::new("tmux");
    command.args(arguments).kill_on_drop(true);

    command.output().await.context("failed to run tmux")
}

fn command_error(stderr: &[u8]) -> anyhow::Error {
    anyhow::anyhow!("tmux failed: {}", String::from_utf8_lossy(stderr).trim())
}

fn is_no_targets_error(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();

    stderr == "no current target"
        || stderr == "no sessions"
        || stderr.starts_with("no server running on ")
        || (stderr.starts_with("error connecting to ")
            && (stderr.ends_with("(No such file or directory)")
                || stderr.ends_with("(Connection refused)")))
}

fn parse_targets(output: &[u8]) -> Result<Vec<Terminal>> {
    let output = std::str::from_utf8(output).context("tmux output is not UTF-8")?;
    let mut session_indices = HashMap::<&str, usize>::new();
    let mut window_indices = HashMap::<(&str, &str), usize>::new();
    let mut pane_indices = HashMap::<(&str, &str), usize>::new();
    let mut next_window_indices = HashMap::<&str, usize>::new();
    let mut terminals = Vec::new();

    for (line_index, line) in output.lines().enumerate() {
        let fields = line.split(FIELD_SEPARATOR).collect::<Vec<_>>();
        let [
            session_id,
            session_name,
            window_id,
            window_name,
            pane_id,
            pane_title,
            pane_command,
            working_directory,
        ] = fields.as_slice()
        else {
            anyhow::bail!(
                "expected 8 fields on line {}, got {}",
                line_index + 1,
                fields.len()
            );
        };

        let next_session_index = session_indices.len() + 1;
        let window_index = *session_indices
            .entry(session_id)
            .or_insert(next_session_index);

        let next_tab_index = next_window_indices.entry(session_id).or_insert(1);
        let tab_index = *window_indices
            .entry((session_id, window_id))
            .or_insert_with(|| {
                let index = *next_tab_index;
                *next_tab_index += 1;
                index
            });

        let terminal_index = pane_indices
            .entry((session_id, window_id))
            .and_modify(|index| *index += 1)
            .or_insert(1);

        let pane_label = if pane_title.is_empty() {
            pane_command
        } else {
            pane_title
        };
        let name = if pane_label.is_empty() {
            format!("{session_name}:{window_name}")
        } else {
            format!("{session_name}:{window_name} · {pane_label}")
        };

        terminals.push(Terminal {
            id: (*pane_id).to_owned(),
            name,
            working_directory: (*working_directory).to_owned(),
            window_index,
            tab_index,
            terminal_index: *terminal_index,
        });
    }

    Ok(terminals)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tmux_panes_and_assigns_one_based_tree_indices() {
        let separator = FIELD_SEPARATOR;
        let output = format!(
            "$0{separator}work{separator}@0{separator}editor{separator}%0{separator}vim{separator}nvim{separator}/src\n\
             $0{separator}work{separator}@0{separator}editor{separator}%1{separator}{separator}zsh{separator}/src\n\
             $0{separator}work{separator}@2{separator}server{separator}%2{separator}api{separator}cargo{separator}/src/api\n\
             $3{separator}ops{separator}@4{separator}logs{separator}%9{separator}tail{separator}tail{separator}/var/log\n"
        );

        let targets = parse_targets(output.as_bytes()).unwrap();

        assert_eq!(
            targets,
            vec![
                Terminal {
                    id: "%0".into(),
                    name: "work:editor · vim".into(),
                    working_directory: "/src".into(),
                    window_index: 1,
                    tab_index: 1,
                    terminal_index: 1,
                },
                Terminal {
                    id: "%1".into(),
                    name: "work:editor · zsh".into(),
                    working_directory: "/src".into(),
                    window_index: 1,
                    tab_index: 1,
                    terminal_index: 2,
                },
                Terminal {
                    id: "%2".into(),
                    name: "work:server · api".into(),
                    working_directory: "/src/api".into(),
                    window_index: 1,
                    tab_index: 2,
                    terminal_index: 1,
                },
                Terminal {
                    id: "%9".into(),
                    name: "ops:logs · tail".into(),
                    working_directory: "/var/log".into(),
                    window_index: 2,
                    tab_index: 1,
                    terminal_index: 1,
                },
            ]
        );
    }

    #[test]
    fn rejects_a_malformed_tmux_record() {
        let error = parse_targets(b"$0\x1fwork\x1f%0\n").unwrap_err();

        assert!(error.to_string().contains("expected 8 fields"));
    }

    #[test]
    fn treats_missing_or_empty_tmux_servers_as_no_targets() {
        assert!(is_no_targets_error(b"no current target\n"));
        assert!(is_no_targets_error(b"no sessions\n"));
        assert!(is_no_targets_error(
            b"no server running on /tmp/tmux-501/default\n"
        ));
        assert!(is_no_targets_error(
            b"error connecting to /tmp/tmux-501/default (No such file or directory)\n"
        ));
        assert!(is_no_targets_error(
            b"error connecting to /tmp/tmux-501/default (Connection refused)\n"
        ));
    }

    #[test]
    fn preserves_real_tmux_failures() {
        assert!(!is_no_targets_error(
            b"error connecting to /tmp/tmux-501/default (Permission denied)\n"
        ));
        assert!(!is_no_targets_error(b"protocol version mismatch\n"));
        assert!(!is_no_targets_error(b"unknown option: -Z\n"));
    }
}
