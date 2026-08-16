use std::ffi::OsString;

use anyhow::{Context, Result, bail};
use tokio::{process::Command, sync::mpsc, task::JoinHandle};

pub type CommandTaskResult = (u64, Result<String>);

pub struct CommandTask {
    program: OsString,
    args: Vec<OsString>,
}

impl CommandTask {
    pub fn new(
        program: impl Into<OsString>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    pub fn spawn(
        self,
        id: u64,
        results: mpsc::UnboundedSender<CommandTaskResult>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let result = self.execute().await;
            let _ = results.send((id, result));
        })
    }

    async fn execute(self) -> Result<String> {
        let program = self.program.to_string_lossy().into_owned();
        let output = Command::new(&self.program)
            .args(&self.args)
            .output()
            .await
            .with_context(|| format!("failed to execute {program}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            bail!(
                "{program} exited with {}{}",
                output.status,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
            );
        }

        String::from_utf8(output.stdout)
            .with_context(|| format!("{program} produced non-UTF-8 output"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn executes_a_command_without_a_shell() {
        let output = CommandTask::new("printf", ["first\nsecond\n"])
            .execute()
            .await
            .unwrap();

        assert_eq!(output, "first\nsecond\n");
    }
}
