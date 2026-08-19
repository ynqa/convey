use std::{collections::BTreeSet, path::PathBuf};

use clap::{Parser, ValueEnum};

#[derive(Debug, Parser)]
#[command(version, about = "Compose Markdown from a workflow definition")]
pub struct Cli {
    /// Terminal applications to query, separated by commas. Defaults to all.
    #[arg(
        short = 't',
        long = "terminal",
        default_value = "all",
        value_parser = parse_terminal_applications,
        value_name = "TERMINAL"
    )]
    pub terminals: BTreeSet<TerminalApplication>,

    /// Workflow definition or directory containing workflow definitions.
    #[arg(value_name = "WORKFLOW")]
    pub workflow: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, ValueEnum)]
pub enum TerminalApplication {
    #[cfg(target_os = "macos")]
    Ghostty,
    #[cfg(target_os = "macos")]
    Iterm2,
    Tmux,
}

impl TerminalApplication {
    pub const fn name(self) -> &'static str {
        match self {
            #[cfg(target_os = "macos")]
            Self::Ghostty => "ghostty",
            #[cfg(target_os = "macos")]
            Self::Iterm2 => "iterm2",
            Self::Tmux => "tmux",
        }
    }
}

fn parse_terminal_applications(value: &str) -> Result<BTreeSet<TerminalApplication>, String> {
    if value.eq_ignore_ascii_case("all") {
        return Ok(TerminalApplication::value_variants()
            .iter()
            .copied()
            .collect());
    }

    value
        .split(',')
        .map(|value| TerminalApplication::from_str(value, false))
        .collect()
}

impl Cli {
    pub fn parse() -> Self {
        <Self as Parser>::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_workflow_path_and_terminal_applications() {
        let cli = Cli::try_parse_from(["convey", "--terminal", "tmux,tmux", "workflows"]).unwrap();

        assert_eq!(cli.workflow, PathBuf::from("workflows"));
        assert_eq!(cli.terminals, BTreeSet::from([TerminalApplication::Tmux]));
    }

    #[test]
    fn defaults_to_all_recognized_terminal_applications() {
        let cli = Cli::try_parse_from(["convey", "workflows"]).unwrap();

        assert_eq!(
            cli.terminals,
            TerminalApplication::value_variants()
                .iter()
                .copied()
                .collect()
        );
    }
}
