use anyhow::{Context, Result};
use async_trait::async_trait;

use super::super::{Terminal, TerminalAutomation, macos::osascript};
use super::GhosttyAutomation;

const LIST_TARGETS_SCRIPT: &str = include_str!("ghostty.list_targets.applescript");
const SEND_ENTER_SCRIPT: &str = include_str!("ghostty.send_enter.applescript");
const SEND_TEXT_SCRIPT: &str = include_str!("ghostty.send_text.applescript");

#[async_trait]
impl TerminalAutomation for GhosttyAutomation {
    async fn list_targets(&self) -> Result<Vec<Terminal>> {
        let output = osascript::run(LIST_TARGETS_SCRIPT, &[])
            .await
            .context("failed to list Ghostty targets")?;
        Ok(serde_json::from_slice(&output).context("invalid Ghostty target response")?)
    }

    async fn send_text(&self, target: &Terminal, text: &str) -> Result<()> {
        osascript::run(SEND_TEXT_SCRIPT, &[&target.id, text])
            .await
            .with_context(|| format!("failed to send text to Ghostty target {}", target.id))?;
        Ok(())
    }

    async fn send_enter(&self, target: &Terminal) -> Result<()> {
        osascript::run(SEND_ENTER_SCRIPT, &[&target.id])
            .await
            .with_context(|| format!("failed to send Enter to Ghostty target {}", target.id))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_terminal_properties() {
        let json = br#"[{"id":"42","name":"claude","working_directory":"/src/convey","window_index":1,"tab_index":2,"terminal_index":3}]"#;

        let terminals: Vec<Terminal> = serde_json::from_slice(json).unwrap();

        assert_eq!(
            terminals,
            vec![Terminal {
                id: "42".into(),
                name: "claude".into(),
                working_directory: "/src/convey".into(),
                window_index: 1,
                tab_index: 2,
                terminal_index: 3,
            }]
        );
    }
}
