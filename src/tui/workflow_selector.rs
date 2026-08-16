use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use promkit::core::{ContentPosition, CreatedGraphemes, crossterm::event::Event};

use super::input_field::select::{Select, SelectItem};
use crate::{tui::NavigationDirection, workflow::Workflow};

pub(super) struct WorkflowSelector {
    select: Select<PathBuf>,
    selection: Option<WorkflowSelection>,
    error: Option<String>,
}

struct WorkflowSelection {
    path: PathBuf,
    workflow: Workflow,
}

impl WorkflowSelector {
    pub(super) fn from_path(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("failed to inspect workflow path {}", path.display()))?;
        if metadata.is_dir() {
            Self::from_directory(path)
        } else if metadata.is_file() {
            Self::from_file(path)
        } else {
            bail!(
                "workflow path {} is neither a file nor a directory",
                path.display()
            )
        }
    }

    pub(super) fn from_directory(directory: &Path) -> Result<Self> {
        let mut paths = fs::read_dir(directory)
            .with_context(|| format!("failed to read workflow directory {}", directory.display()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .filter(|file_type| file_type.is_file())
                    .map(|_| entry.path())
            })
            .filter(|path| is_yaml(path))
            .collect::<Vec<_>>();
        paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
        ensure!(
            !paths.is_empty(),
            "workflow directory {} contains no YAML files",
            directory.display()
        );

        let mut select = Select::new();
        select.replace_items(
            paths
                .into_iter()
                .map(|path| {
                    let label = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    SelectItem::new(path.display().to_string(), label, path)
                })
                .collect(),
        );
        Ok(Self {
            select,
            selection: None,
            error: None,
        })
    }

    fn from_file(path: &Path) -> Result<Self> {
        let workflow = Workflow::load(path)?;
        let label = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let path = path.to_owned();
        let mut select = Select::new();
        select.replace_items(vec![SelectItem::new(
            path.display().to_string(),
            label,
            path.clone(),
        )]);
        Ok(Self {
            select,
            selection: Some(WorkflowSelection { path, workflow }),
            error: None,
        })
    }

    pub(super) fn selected(&self) -> Option<&Workflow> {
        self.selection.as_ref().map(|selection| &selection.workflow)
    }

    pub(super) fn value_label(&self) -> String {
        self.selection.as_ref().map_or_else(
            || "not set".to_owned(),
            |selection| {
                let filename = selection
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_else(|| selection.path.as_os_str().to_string_lossy());
                format!("{} — {filename}", selection.workflow.name())
            },
        )
    }

    pub(super) fn save(&mut self) -> Option<bool> {
        let path = self.select.selected()?.clone();
        if self
            .selection
            .as_ref()
            .is_some_and(|selection| selection.path == path)
        {
            self.error = None;
            return Some(false);
        }
        match Workflow::load(&path) {
            Ok(workflow) => {
                self.selection = Some(WorkflowSelection { path, workflow });
                self.error = None;
                Some(true)
            }
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                None
            }
        }
    }

    pub(super) fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn editor_graphemes(&self) -> CreatedGraphemes {
        self.select.query_graphemes()
    }

    pub(super) fn candidates_graphemes(&self) -> CreatedGraphemes {
        self.select.list_graphemes()
    }

    pub(super) fn handle_event(&mut self, event: &Event) {
        self.error = None;
        self.select.handle_event(event);
    }

    pub(super) fn navigate(&mut self, direction: NavigationDirection) -> bool {
        self.error = None;
        self.select.navigate(direction)
    }

    pub(super) fn move_editor_cursor_to(&mut self, position: ContentPosition) {
        self.select.move_query_cursor_to(position);
    }

    pub(super) fn select_at(&mut self, position: ContentPosition) -> bool {
        self.select.select_at(position)
    }

    #[cfg(test)]
    pub(super) fn with_selected(workflow: Workflow) -> Self {
        let path = PathBuf::from("test.yaml");
        let mut select = Select::new();
        select.replace_items(vec![SelectItem::new(
            "test.yaml",
            "test.yaml",
            path.clone(),
        )]);
        Self {
            select,
            selection: Some(WorkflowSelection { path, workflow }),
            error: None,
        }
    }
}

fn is_yaml(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml")
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "convey-workflow-selector-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, contents: &str) {
            fs::write(self.0.join(name), contents).unwrap();
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    const VALID_WORKFLOW: &str = r#"
name: Test workflow
inputs:
  request:
    type: textarea
output:
  template: "{{ inputs.request }}"
"#;

    #[test]
    fn finds_yaml_files_and_loads_the_selected_workflow() {
        let directory = TestDirectory::new();
        directory.write("ignored.txt", VALID_WORKFLOW);
        directory.write("workflow.yml", VALID_WORKFLOW);

        let mut selector = WorkflowSelector::from_directory(&directory.0).unwrap();

        assert_eq!(selector.save(), Some(true));
        assert_eq!(selector.selected().unwrap().name(), "Test workflow");
        assert_eq!(selector.value_label(), "Test workflow — workflow.yml");
    }

    #[test]
    fn directly_selected_file_starts_with_its_workflow_loaded() {
        let directory = TestDirectory::new();
        directory.write("workflow.yaml", VALID_WORKFLOW);

        let selector = WorkflowSelector::from_path(&directory.0.join("workflow.yaml")).unwrap();

        assert_eq!(selector.selected().unwrap().name(), "Test workflow");
        assert_eq!(selector.value_label(), "Test workflow — workflow.yaml");
    }

    #[test]
    fn changes_the_loaded_workflow_when_the_selection_moves() {
        let directory = TestDirectory::new();
        directory.write(
            "first.yaml",
            &VALID_WORKFLOW.replace("Test workflow", "First"),
        );
        directory.write(
            "second.yaml",
            &VALID_WORKFLOW.replace("Test workflow", "Second"),
        );
        let mut selector = WorkflowSelector::from_directory(&directory.0).unwrap();
        assert_eq!(selector.save(), Some(true));
        assert_eq!(selector.selected().unwrap().name(), "First");

        selector.handle_event(&Event::Key(promkit::core::crossterm::event::KeyEvent::new(
            promkit::core::crossterm::event::KeyCode::Down,
            promkit::core::crossterm::event::KeyModifiers::NONE,
        )));

        assert_eq!(selector.save(), Some(true));
        assert_eq!(selector.selected().unwrap().name(), "Second");
    }

    #[test]
    fn reports_invalid_yaml_without_selecting_it() {
        let directory = TestDirectory::new();
        directory.write("invalid.yaml", "not: a workflow");

        let mut selector = WorkflowSelector::from_directory(&directory.0).unwrap();

        assert_eq!(selector.save(), None);
        assert!(selector.selected().is_none());
        assert!(selector.error_message().unwrap().contains("invalid.yaml"));
    }

    #[test]
    fn rejects_a_directory_without_yaml_files() {
        let directory = TestDirectory::new();
        directory.write("ignored.txt", VALID_WORKFLOW);

        let error = WorkflowSelector::from_directory(&directory.0)
            .err()
            .unwrap();

        assert!(error.to_string().contains("contains no YAML files"));
    }
}
