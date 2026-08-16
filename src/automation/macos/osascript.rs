use anyhow::Result;
use tokio::process::Command;

pub async fn run(script: &str, arguments: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new("osascript");
    command
        .arg("-e")
        .arg(script)
        .arg("--")
        .args(arguments)
        .kill_on_drop(true);

    let output = command.output().await?;

    if !output.status.success() {
        anyhow::bail!(
            "osascript failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(output.stdout)
}
