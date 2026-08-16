use anyhow::{Context, Result};
use async_trait::async_trait;

use super::super::{Terminal, TerminalAutomation, macos::osascript};
use super::ITerm2Automation;

const LIST_TARGETS_SCRIPT: &str = include_str!("iterm2.list_targets.applescript");
const SEND_ENTER_SCRIPT: &str = include_str!("iterm2.send_enter.applescript");
const SEND_TEXT_SCRIPT: &str = include_str!("iterm2.send_text.applescript");

#[async_trait]
impl TerminalAutomation for ITerm2Automation {
    async fn list_targets(&self) -> Result<Vec<Terminal>> {
        let output = osascript::run(LIST_TARGETS_SCRIPT, &[])
            .await
            .context("failed to list iTerm2 targets")?;
        Ok(serde_json::from_slice(&output).context("invalid iTerm2 target response")?)
    }

    async fn send_text(&self, target: &Terminal, text: &str) -> Result<()> {
        osascript::run(SEND_TEXT_SCRIPT, &[&target.id, text])
            .await
            .with_context(|| format!("failed to send text to iTerm2 target {}", target.id))?;
        Ok(())
    }

    async fn send_enter(&self, target: &Terminal) -> Result<()> {
        osascript::run(SEND_ENTER_SCRIPT, &[&target.id])
            .await
            .with_context(|| format!("failed to send Enter to iTerm2 target {}", target.id))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_terminal_properties() {
        let json = br#"[{"id":"w0t1p2:session","name":"shell","working_directory":"/src/convey","window_index":1,"tab_index":2,"terminal_index":3}]"#;

        let terminals: Vec<Terminal> = serde_json::from_slice(json).unwrap();

        assert_eq!(
            terminals,
            vec![Terminal {
                id: "w0t1p2:session".into(),
                name: "shell".into(),
                working_directory: "/src/convey".into(),
                window_index: 1,
                tab_index: 2,
                terminal_index: 3,
            }]
        );
    }
}
