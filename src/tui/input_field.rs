use std::collections::BTreeMap;

use anyhow::{Context, Result};
use promkit::core::{ContentPosition, CreatedGraphemes, crossterm::event::Event};

pub(super) mod select;
mod textarea;

use crate::{
    input::{CommandCandidates, InputDefinition, InputKind, SelectCandidates},
    task::CommandTask,
    tui::NavigationDirection,
};
use select::{Select, SelectItem};
use textarea::Textarea;

pub(super) struct InputField {
    name: String,
    title: String,
    allow_empty: bool,
    value: Option<String>,
    control: FieldControl,
}

enum FieldControl {
    Select(Box<SelectControl>),
    Textarea(Box<Textarea>),
}

struct SelectControl {
    depends_on: Vec<String>,
    select: Select<String>,
    command: Option<CommandCandidates>,
    load_state: LoadState,
}

pub(super) struct SaveOutcome {
    pub(super) changed: bool,
}

enum LoadState {
    Ready,
    Unloaded,
    Loading,
    Failed(String),
}

impl InputField {
    pub(super) fn from_definition(definition: &InputDefinition) -> Self {
        let control = match &definition.kind {
            InputKind::Select(select_definition) => {
                let mut select = Select::new();
                let (command, load_state) = match &select_definition.candidates {
                    SelectCandidates::Values(values) => {
                        select.replace_items(select_items(values.clone()));
                        (None, LoadState::Ready)
                    }
                    SelectCandidates::Command(command) => {
                        (Some(command.clone()), LoadState::Unloaded)
                    }
                };
                FieldControl::Select(Box::new(SelectControl {
                    depends_on: select_definition.depends_on.clone(),
                    select,
                    command,
                    load_state,
                }))
            }
            InputKind::Textarea => FieldControl::Textarea(Box::new(Textarea::new())),
        };
        Self {
            name: definition.name.clone(),
            title: definition.name.replace('_', " "),
            allow_empty: definition.allow_empty,
            value: None,
            control,
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn title(&self) -> &str {
        &self.title
    }

    pub(super) fn depends_on(&self, name: &str) -> bool {
        let FieldControl::Select(select) = &self.control else {
            return false;
        };
        select
            .depends_on
            .iter()
            .any(|dependency| dependency == name)
    }

    pub(super) fn missing_dependencies<'a>(
        &'a self,
        values: &BTreeMap<String, String>,
    ) -> Vec<&'a str> {
        let FieldControl::Select(select) = &self.control else {
            return Vec::new();
        };
        select
            .depends_on
            .iter()
            .filter(|dependency| !values.contains_key(*dependency))
            .map(String::as_str)
            .collect()
    }

    pub(super) fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    pub(super) fn draft(&self) -> Option<String> {
        match &self.control {
            FieldControl::Select(select) if matches!(select.load_state, LoadState::Loading) => None,
            FieldControl::Select(select) => Some(select.select.value().unwrap_or_default()),
            FieldControl::Textarea(textarea) => Some(textarea.value()),
        }
    }

    pub(super) const fn allow_empty(&self) -> bool {
        self.allow_empty
    }

    pub(super) const fn is_required(&self) -> bool {
        !self.allow_empty()
    }

    pub(super) fn custom_value(&self) -> Option<String> {
        match &self.control {
            FieldControl::Select(select) => select.select.custom_value(),
            FieldControl::Textarea(_) => None,
        }
    }

    pub(super) fn save(&mut self) -> Option<SaveOutcome> {
        let draft = self.draft()?;
        let next = (!is_empty(&draft)).then_some(draft);
        let changed = self.value.is_some() && self.value != next;
        self.value = next;
        Some(SaveOutcome { changed })
    }

    pub(super) fn invalidate(&mut self) {
        self.value = None;
        if let FieldControl::Select(select) = &mut self.control
            && select.command.is_some()
        {
            select.select.replace_items(Vec::new());
            select.load_state = LoadState::Unloaded;
        }
    }

    pub(super) fn editor_graphemes(&self) -> CreatedGraphemes {
        match &self.control {
            FieldControl::Select(select) => select.select.query_graphemes(),
            FieldControl::Textarea(textarea) => textarea.graphemes(),
        }
    }

    pub(super) fn candidates_graphemes(&self) -> CreatedGraphemes {
        match &self.control {
            FieldControl::Select(select) => select.select.list_graphemes(),
            FieldControl::Textarea(_) => CreatedGraphemes::default(),
        }
    }

    pub(super) fn handle_event(&mut self, event: &Event) {
        match &mut self.control {
            FieldControl::Select(select) => select.select.handle_event(event),
            FieldControl::Textarea(textarea) => textarea.handle_event(event),
        }
    }

    pub(super) fn navigate(&mut self, direction: NavigationDirection) -> bool {
        match &mut self.control {
            FieldControl::Select(select) => select.select.navigate(direction),
            FieldControl::Textarea(textarea) => textarea.navigate(direction),
        }
    }

    pub(super) fn move_editor_cursor_to(&mut self, position: ContentPosition) {
        match &mut self.control {
            FieldControl::Select(select) => select.select.move_query_cursor_to(position),
            FieldControl::Textarea(textarea) => textarea.move_cursor_to(position),
        }
    }

    pub(super) fn select_at(&mut self, position: ContentPosition) -> bool {
        match &mut self.control {
            FieldControl::Select(select) => select.select.select_at(position),
            FieldControl::Textarea(_) => false,
        }
    }

    pub(super) const fn is_textarea(&self) -> bool {
        matches!(self.control, FieldControl::Textarea(_))
    }

    pub(super) fn needs_load(&self) -> bool {
        matches!(&self.control, FieldControl::Select(select) if matches!(select.load_state, LoadState::Unloaded))
    }

    pub(super) fn is_loading(&self) -> bool {
        matches!(&self.control, FieldControl::Select(select) if matches!(select.load_state, LoadState::Loading))
    }

    pub(super) fn retry(&mut self) {
        if let FieldControl::Select(select) = &mut self.control
            && matches!(&select.load_state, LoadState::Failed(_))
        {
            select.load_state = LoadState::Unloaded;
        }
    }

    pub(super) fn loading_message(&self) -> Option<String> {
        self.is_loading()
            .then(|| format!("Loading {}…", self.title()))
    }

    pub(super) fn error_message(&self) -> Option<&str> {
        let FieldControl::Select(select) = &self.control else {
            return None;
        };
        match &select.load_state {
            LoadState::Failed(message) => Some(message),
            LoadState::Ready | LoadState::Unloaded | LoadState::Loading => None,
        }
    }

    pub(super) fn command_task(
        &mut self,
        values: &BTreeMap<String, String>,
    ) -> Result<Option<CommandTask>> {
        let FieldControl::Select(select) = &mut self.control else {
            return Ok(None);
        };
        let Some(definition) = &select.command else {
            return Ok(None);
        };

        let (program, args) = definition
            .render(values)
            .with_context(|| format!("failed to prepare candidates for {}", self.name))?;
        select.load_state = LoadState::Loading;
        Ok(Some(CommandTask::new(program, args)))
    }

    pub(super) fn finish_load(&mut self, result: Result<Vec<String>>) {
        let FieldControl::Select(select) = &mut self.control else {
            return;
        };
        match result {
            Ok(values) if values.is_empty() => {
                select.load_state = LoadState::Failed("Task returned no candidates".into());
            }
            Ok(values) => {
                select.select.replace_items(select_items(values));
                select.load_state = LoadState::Ready;
            }
            Err(error) => {
                select.load_state = LoadState::Failed(format!("{error:#}"));
            }
        }
    }
}

fn is_empty(value: &str) -> bool {
    value.trim().is_empty()
}

fn select_items(values: Vec<String>) -> Vec<SelectItem<String>> {
    values
        .into_iter()
        .map(|value| SelectItem::new(value.clone(), value.clone(), value))
        .collect()
}
