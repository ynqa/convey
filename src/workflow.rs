use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_yaml::Mapping;

use crate::input::{
    CommandCandidates, InputDefinition, InputKind, SelectCandidates, SelectDefinition,
};
use crate::template;

#[derive(Clone, Debug)]
pub struct Workflow {
    name: String,
    inputs: Vec<InputDefinition>,
    output: OutputDefinition,
}

#[derive(Clone, Debug)]
struct OutputDefinition {
    template: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflow {
    name: String,
    inputs: Mapping,
    output: RawOutput,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInput {
    #[serde(rename = "type")]
    kind: RawInputKind,
    #[serde(default = "allow_empty_by_default")]
    allow_empty: bool,
    #[serde(default)]
    depends_on: Vec<String>,
    candidates: Option<RawCandidates>,
}

const fn allow_empty_by_default() -> bool {
    true
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawInputKind {
    Select,
    Textarea,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCandidates {
    values: Option<Vec<String>>,
    command: Option<CommandCandidates>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOutput {
    template: String,
}

impl Workflow {
    pub fn load(path: &Path) -> Result<Self> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read workflow {}", path.display()))?;
        Self::from_str(&source)
            .with_context(|| format!("failed to parse workflow {}", path.display()))
    }

    fn from_str(source: &str) -> Result<Self> {
        let raw: RawWorkflow = serde_yaml::from_str(source)?;
        ensure!(
            !raw.name.trim().is_empty(),
            "workflow name must not be empty"
        );

        let mut inputs = Vec::with_capacity(raw.inputs.len());
        let mut seen = BTreeSet::new();
        for (name, definition) in raw.inputs {
            let name = name
                .as_str()
                .context("input names must be strings")?
                .to_owned();
            ensure!(!name.trim().is_empty(), "input name must not be empty");
            ensure!(seen.insert(name.clone()), "duplicate input: {name}");

            let raw_input: RawInput = serde_yaml::from_value(definition)?;
            let kind = match raw_input.kind {
                RawInputKind::Select => {
                    for dependency in &raw_input.depends_on {
                        ensure!(
                            seen.contains(dependency),
                            "input {name} depends on unknown or later input {dependency}"
                        );
                    }
                    let candidates = raw_input
                        .candidates
                        .with_context(|| format!("select input {name} requires candidates"))?;
                    InputKind::Select(SelectDefinition {
                        depends_on: raw_input.depends_on,
                        candidates: candidates.try_into().with_context(|| {
                            format!("invalid candidates for select input {name}")
                        })?,
                    })
                }
                RawInputKind::Textarea => {
                    ensure!(
                        raw_input.candidates.is_none(),
                        "textarea input {name} cannot define candidates"
                    );
                    ensure!(
                        raw_input.depends_on.is_empty(),
                        "textarea input {name} cannot define depends_on"
                    );
                    InputKind::Textarea
                }
            };

            inputs.push(InputDefinition {
                name,
                allow_empty: raw_input.allow_empty,
                kind,
            });
        }
        ensure!(!inputs.is_empty(), "workflow requires at least one input");

        ensure!(
            !raw.output.template.trim().is_empty(),
            "output template must not be empty"
        );

        Ok(Self {
            name: raw.name,
            inputs,
            output: OutputDefinition {
                template: raw.output.template,
            },
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn inputs(&self) -> &[InputDefinition] {
        &self.inputs
    }

    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<String> {
        let mut values = values.clone();
        for input in &self.inputs {
            if input.allow_empty {
                values.entry(input.name.clone()).or_default();
            }
        }
        template::render(&self.output.template, &values).context("failed to render Markdown output")
    }

    #[cfg(test)]
    pub(crate) fn for_test(name: &str, inputs: Vec<InputDefinition>) -> Self {
        Self {
            name: name.to_owned(),
            inputs,
            output: OutputDefinition {
                template: "test".to_owned(),
            },
        }
    }
}

impl TryFrom<RawCandidates> for SelectCandidates {
    type Error = anyhow::Error;

    fn try_from(raw: RawCandidates) -> Result<Self> {
        match (raw.values, raw.command) {
            (Some(values), None) => {
                ensure!(!values.is_empty(), "candidate values must not be empty");
                Ok(Self::Values(values))
            }
            (None, Some(command)) => Ok(Self::Command(command)),
            (Some(_), Some(_)) => {
                bail!("candidates cannot contain both values and command sources")
            }
            (None, None) => bail!("candidates require a values or command source"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow() -> Workflow {
        Workflow::from_str(include_str!("../examples/kubernetes-investigation.yaml")).unwrap()
    }

    #[test]
    fn preserves_input_order() {
        assert_eq!(
            workflow()
                .inputs()
                .iter()
                .map(|input| input.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "context",
                "namespace",
                "resource_kind",
                "resource",
                "request"
            ]
        );
    }

    #[test]
    fn renders_candidate_command_arguments_from_saved_inputs() {
        let workflow = workflow();
        let InputKind::Select(select) = &workflow.inputs()[1].kind else {
            panic!("namespace is not a select input");
        };
        let SelectCandidates::Command(command) = &select.candidates else {
            panic!("namespace does not use command candidates");
        };
        let values = BTreeMap::from([("context".into(), "production".into())]);

        let (program, args) = command.render(&values).unwrap();

        assert_eq!(program, "kubectl");
        assert_eq!(args[0..2], ["--context", "production"]);
    }

    #[test]
    fn allows_empty_inputs_by_default() {
        let workflow = Workflow::from_str(
            r"
name: test
inputs:
  request:
    type: textarea
output:
  template: '{{ inputs.request }}'
",
        )
        .unwrap();

        assert!(workflow.inputs()[0].allow_empty);
    }

    #[test]
    fn kubernetes_investigation_requires_a_request() {
        let workflow = workflow();

        assert!(
            workflow
                .inputs()
                .iter()
                .filter(|input| input.name != "request")
                .all(|input| input.allow_empty)
        );
        assert!(
            !workflow
                .inputs()
                .iter()
                .find(|input| input.name == "request")
                .unwrap()
                .allow_empty
        );
    }

    #[test]
    fn reads_required_inputs() {
        let workflow = Workflow::from_str(
            r"
name: test
inputs:
  request:
    type: textarea
    allow_empty: false
output:
  template: '{{ inputs.request }}'
",
        )
        .unwrap();

        assert!(!workflow.inputs()[0].allow_empty);
    }

    #[test]
    fn renders_markdown_without_html_escaping() {
        let values = BTreeMap::from([
            ("context".into(), "production".into()),
            ("namespace".into(), "payments".into()),
            ("resource_kind".into(), "pod".into()),
            ("resource".into(), "api".into()),
            ("request".into(), "compare a < b & b > 0".into()),
        ]);

        let markdown = workflow().render(&values).unwrap();

        assert!(markdown.contains("compare a < b & b > 0"));
    }

    #[test]
    fn renders_unset_optional_inputs_as_empty() {
        let workflow = Workflow::from_str(
            r"
name: test
inputs:
  request:
    type: textarea
output:
  template: 'Request: {{ inputs.request }}'
",
        )
        .unwrap();

        assert_eq!(workflow.render(&BTreeMap::new()).unwrap(), "Request: ");
    }

    #[test]
    fn rejects_forward_dependencies() {
        let source = r"
name: invalid
inputs:
  namespace:
    type: select
    depends_on: [context]
    candidates:
      values: [default]
output:
  template: test
";

        assert!(Workflow::from_str(source).is_err());
    }

    #[test]
    fn rejects_textarea_dependencies() {
        let source = r"
name: test
inputs:
  context:
    type: select
    candidates:
      values: [default]
  request:
    type: textarea
    depends_on: [context]
output:
  template: test
";

        assert!(Workflow::from_str(source).is_err());
    }
}
