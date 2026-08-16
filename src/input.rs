use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::template;

#[derive(Clone, Debug)]
pub struct InputDefinition {
    pub name: String,
    pub allow_empty: bool,
    pub kind: InputKind,
}

#[derive(Clone, Debug)]
pub enum InputKind {
    Select(SelectDefinition),
    Textarea,
}

#[derive(Clone, Debug)]
pub struct SelectDefinition {
    pub depends_on: Vec<String>,
    pub candidates: SelectCandidates,
}

#[derive(Clone, Debug)]
pub enum SelectCandidates {
    Values(Vec<String>),
    Command(CommandCandidates),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCandidates {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl CommandCandidates {
    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<(String, Vec<String>)> {
        let program =
            template::render(&self.program, values).context("invalid candidate command program")?;
        let args = self
            .args
            .iter()
            .map(|arg| template::render(arg, values).context("invalid command argument"))
            .collect::<Result<Vec<_>>>()?;
        Ok((program, args))
    }
}

pub fn decode_candidates(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}
