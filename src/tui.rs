use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result};
use futures::StreamExt;
use promkit::{
    core::{
        ContentPosition, CreatedGraphemes, HeightPolicy, ScreenPosition, Widget, WidgetLayout,
        WidgetPosition, WidthMode,
        crossterm::{
            event::{
                Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
                MouseEventKind,
            },
            style::{Attribute, Attributes, Color, ContentStyle},
        },
        grapheme::StyledGraphemes,
        render::{Renderer, SharedRenderer},
    },
    widgets::{
        spinner::frame,
        text::{self, Text},
    },
};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

mod input_field;
mod terminal_target;
mod workflow_selector;

#[cfg(test)]
use crate::input::InputDefinition;
use crate::{
    cli::TerminalApplication, input::decode_candidates, task::CommandTaskResult, workflow::Workflow,
};
use input_field::InputField;
use terminal_target::{TerminalTarget, TerminalTargetSelector};
use workflow_selector::WorkflowSelector;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Region {
    TargetHeader,
    TargetValue,
    TargetEditor,
    TargetCandidates,
    TargetStatus,
    WorkflowHeader,
    WorkflowValue,
    WorkflowEditor,
    WorkflowCandidates,
    WorkflowStatus,
    WorkflowSpacer,
    Fields,
    Editor,
    Candidates,
    Status,
    Page,
}

impl Region {
    const ALL: [Self; 16] = [
        Self::TargetHeader,
        Self::TargetValue,
        Self::TargetEditor,
        Self::TargetCandidates,
        Self::TargetStatus,
        Self::WorkflowHeader,
        Self::WorkflowValue,
        Self::WorkflowEditor,
        Self::WorkflowCandidates,
        Self::WorkflowStatus,
        Self::WorkflowSpacer,
        Self::Fields,
        Self::Editor,
        Self::Candidates,
        Self::Status,
        Self::Page,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Focus {
    Target,
    Workflow,
    Field(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Input,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tui) enum NavigationDirection {
    Previous,
    Next,
}

pub struct InputScreen {
    workflow: WorkflowSelector,
    fields: Vec<InputField>,
    focus: Focus,
    page: Page,
    target: TerminalTargetSelector,
    renderer: Option<SharedRenderer<Region>>,
    task_tx: mpsc::UnboundedSender<CommandTaskResult>,
    task_rx: mpsc::UnboundedReceiver<CommandTaskResult>,
    running_task: Option<RunningTask>,
    next_task_id: u64,
    spinner_frame: usize,
    show_required_errors: bool,
}

pub struct Submission {
    pub target: TerminalTarget,
    pub workflow: Workflow,
    pub values: BTreeMap<String, String>,
}

struct RunningTask {
    id: u64,
    field: usize,
    handle: JoinHandle<()>,
}

enum InputScreenAction {
    Redraw,
    Submit,
    Cancel,
}

struct StatusMessage {
    text: String,
    color: Color,
}

impl StatusMessage {
    fn new(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            color,
        }
    }
}

impl InputScreen {
    pub fn from_workflow_path(
        workflow_path: &Path,
        terminals: BTreeSet<TerminalApplication>,
    ) -> Result<Self> {
        Ok(Self::from_workflow_selector(
            WorkflowSelector::from_path(workflow_path)?,
            terminals,
        ))
    }

    fn from_workflow_selector(
        workflow: WorkflowSelector,
        terminals: BTreeSet<TerminalApplication>,
    ) -> Self {
        let (task_tx, task_rx) = mpsc::unbounded_channel();
        let mut screen = Self {
            workflow,
            fields: Vec::new(),
            focus: Focus::Target,
            page: Page::Input,
            target: TerminalTargetSelector::new(terminals),
            renderer: None,
            task_tx,
            task_rx,
            running_task: None,
            next_task_id: 0,
            spinner_frame: 0,
            show_required_errors: false,
        };
        screen.replace_fields();
        screen
    }

    #[cfg(test)]
    fn new(
        name: &str,
        definitions: &[InputDefinition],
        terminals: BTreeSet<TerminalApplication>,
    ) -> Self {
        let workflow = Workflow::for_test(name, definitions.to_vec());
        Self::from_workflow_selector(WorkflowSelector::with_selected(workflow), terminals)
    }

    pub fn reset(
        &mut self,
        workflow_path: &Path,
        terminals: BTreeSet<TerminalApplication>,
    ) -> Result<()> {
        let mut replacement = Self::from_workflow_path(workflow_path, terminals)?;
        replacement.renderer = self.renderer.take();
        *self = replacement;
        Ok(())
    }

    pub async fn run(&mut self) -> Result<Option<Submission>> {
        let mut target_updates = self.target.start();
        self.initialize_renderer().await?;
        self.draw().await?;

        let mut events = EventStream::new();
        let mut spinner_ticks = interval(Duration::from_millis(80));
        spinner_ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                event = events.next() => {
                    let Some(event) = event else {
                        return Ok(None);
                    };
                    let event = event?;
                    let action = self.process_event(&event);
                    match action {
                        InputScreenAction::Redraw => {
                            self.start_active_task();
                            self.draw().await?;
                        }
                        InputScreenAction::Submit => {
                            let target = self
                                .target
                                .selected()
                                .cloned()
                                .context("terminal target is not selected")?;
                            return Ok(Some(Submission {
                                target,
                                workflow: self
                                    .workflow
                                    .selected()
                                    .cloned()
                                    .context("workflow is not selected")?,
                                values: self.saved_values(),
                            }));
                        }
                        InputScreenAction::Cancel => return Ok(None),
                    }
                }
                task_event = self.task_rx.recv() => {
                    let task_event = task_event.context("candidate task event channel closed")?;
                    self.apply_task_event(task_event);
                    self.start_active_task();
                    self.draw().await?;
                }
                target_update = target_updates.recv() => {
                    let target_update = target_update.context("terminal target refresh channel closed")?;
                    self.target.apply_update(target_update);
                    self.draw().await?;
                }
                _ = spinner_ticks.tick(), if self.target.is_loading() || self.fields.iter().any(InputField::is_loading) => {
                    self.spinner_frame = (self.spinner_frame + 1) % frame::DOTS.len();
                    self.draw().await?;
                }
            }
        }
    }

    async fn initialize_renderer(&mut self) -> Result<()> {
        let regions = self.rendered_region_updates();
        if let Some(renderer) = &self.renderer {
            renderer.update(regions);
        } else {
            self.renderer = Some(SharedRenderer::new(
                Renderer::try_new_with_graphemes(regions, false).await?,
            ));
        }
        Ok(())
    }

    fn renderer(&self) -> Result<&SharedRenderer<Region>> {
        self.renderer
            .as_ref()
            .context("input screen renderer is not initialized")
    }

    async fn draw(&self) -> Result<()> {
        self.renderer()?
            .update(self.rendered_region_updates())
            .render()
            .await
    }

    fn rendered_region_updates(&self) -> Vec<(Region, CreatedGraphemes)> {
        let mut regions = Region::ALL
            .into_iter()
            .map(|region| (region, CreatedGraphemes::default()))
            .collect::<BTreeMap<_, _>>();
        regions.extend(self.rendered_regions());
        regions.into_iter().collect()
    }

    fn rendered_regions(&self) -> Vec<(Region, CreatedGraphemes)> {
        if self.page == Page::Help {
            return vec![(Region::Page, Self::help_page_graphemes())];
        }
        vec![
            (Region::TargetHeader, Self::target_header_graphemes()),
            (Region::TargetValue, self.target_value_graphemes()),
            (
                Region::TargetEditor,
                if self.focus == Focus::Target {
                    self.target.editor_graphemes()
                } else {
                    CreatedGraphemes::default()
                },
            ),
            (
                Region::TargetCandidates,
                if self.focus == Focus::Target {
                    self.target.candidates_graphemes()
                } else {
                    CreatedGraphemes::default()
                },
            ),
            (Region::TargetStatus, self.target_status_graphemes()),
            (Region::WorkflowHeader, self.workflow_header_graphemes()),
            (Region::WorkflowValue, self.workflow_value_graphemes()),
            (
                Region::WorkflowEditor,
                if self.focus == Focus::Workflow {
                    self.workflow.editor_graphemes()
                } else {
                    CreatedGraphemes::default()
                },
            ),
            (
                Region::WorkflowCandidates,
                if self.focus == Focus::Workflow {
                    self.workflow.candidates_graphemes()
                } else {
                    CreatedGraphemes::default()
                },
            ),
            (
                Region::WorkflowStatus,
                self.workflow_selection_status_graphemes(),
            ),
            (
                Region::WorkflowSpacer,
                if self.focus == Focus::Workflow {
                    text_graphemes(" ", None)
                } else {
                    CreatedGraphemes::default()
                },
            ),
            (Region::Fields, self.fields_graphemes()),
            (
                Region::Editor,
                match self.focus {
                    Focus::Target | Focus::Workflow => CreatedGraphemes::default(),
                    Focus::Field(index) => self.fields[index].editor_graphemes(),
                },
            ),
            (
                Region::Candidates,
                match self.focus {
                    Focus::Target | Focus::Workflow => CreatedGraphemes::default(),
                    Focus::Field(index) => self.fields[index].candidates_graphemes(),
                },
            ),
            (Region::Status, self.workflow_status_graphemes()),
        ]
    }

    fn target_header_graphemes() -> CreatedGraphemes {
        text_graphemes(
            "Destination\n",
            Some(ContentStyle {
                attributes: Attributes::from(Attribute::Bold),
                ..Default::default()
            }),
        )
    }

    fn target_value_graphemes(&self) -> CreatedGraphemes {
        let marker = if self.focus == Focus::Target {
            "▶"
        } else if self.target.selected().is_some() {
            "✓"
        } else {
            "○"
        };
        text_graphemes(
            format!("{marker} {}\n", compact_value(&self.target.value_label())),
            None,
        )
    }

    fn workflow_header_graphemes(&self) -> CreatedGraphemes {
        text_graphemes(
            "\nWorkflow\n",
            Some(ContentStyle {
                attributes: Attributes::from(Attribute::Bold),
                ..Default::default()
            }),
        )
    }

    fn workflow_value_graphemes(&self) -> CreatedGraphemes {
        let marker = if self.focus == Focus::Workflow {
            "▶"
        } else if self.workflow.selected().is_some() {
            "✓"
        } else {
            "○"
        };
        text_graphemes(
            format!("{marker} {}\n", compact_value(&self.workflow.value_label())),
            None,
        )
    }

    fn fields_graphemes(&self) -> CreatedGraphemes {
        let lines = self
            .fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let focused = self.focus == Focus::Field(index);
                let marker = if focused {
                    "▶"
                } else if field.value().is_some() {
                    "✓"
                } else {
                    "○"
                };
                let value = if focused {
                    field
                        .draft()
                        .as_deref()
                        .map(compact_value)
                        .or_else(|| field.value().map(compact_value))
                        .unwrap_or_else(empty_value)
                } else {
                    field.value().map_or_else(empty_value, compact_value)
                };
                let line = StyledGraphemes::from(format!("{marker} {}: {value}", field.title()));
                if focused {
                    line.apply_style(focused_field_style())
                } else {
                    line
                }
            })
            .chain(std::iter::once(StyledGraphemes::default()));
        CreatedGraphemes {
            graphemes: StyledGraphemes::from_lines(lines),
            layout: WidgetLayout {
                height_policy: HeightPolicy::FairContent,
                width_mode: WidthMode::Truncate,
                ..Default::default()
            },
            cursor: match self.focus {
                Focus::Target | Focus::Workflow => None,
                Focus::Field(index) => Some(ContentPosition {
                    row: index,
                    column: 0,
                }),
            },
        }
    }

    fn target_status_graphemes(&self) -> CreatedGraphemes {
        if self.focus != Focus::Target {
            return CreatedGraphemes::default();
        }
        let Some(status) = self.target_status() else {
            return CreatedGraphemes::default();
        };
        let prefix = if self.target.is_loading() {
            format!("{} ", frame::DOTS[self.spinner_frame])
        } else {
            String::new()
        };
        text_graphemes(
            format!("{prefix}{}\n", status.text),
            Some(ContentStyle {
                foreground_color: Some(status.color),
                ..Default::default()
            }),
        )
    }

    fn target_status(&self) -> Option<StatusMessage> {
        if let Some(message) = self.target.status_message() {
            let color = if self.target.is_loading() {
                Color::DarkCyan
            } else {
                Color::DarkYellow
            };
            return Some(StatusMessage::new(message, color));
        }
        (self.show_required_errors && self.target.selected().is_none())
            .then(|| StatusMessage::new("destination is required", Color::DarkRed))
    }

    fn workflow_selection_status_graphemes(&self) -> CreatedGraphemes {
        if self.focus != Focus::Workflow {
            return CreatedGraphemes::default();
        }
        let message = self
            .workflow
            .error_message()
            .map(|error| StatusMessage::new(format!("✗ {error}"), Color::DarkRed))
            .or_else(|| {
                (self.show_required_errors && self.workflow.selected().is_none())
                    .then(|| StatusMessage::new("workflow is required", Color::DarkRed))
            });
        let Some(message) = message else {
            return CreatedGraphemes::default();
        };
        text_graphemes(
            format!("{}\n", message.text),
            Some(ContentStyle {
                foreground_color: Some(message.color),
                ..Default::default()
            }),
        )
    }

    fn workflow_status_graphemes(&self) -> CreatedGraphemes {
        let Focus::Field(index) = self.focus else {
            return CreatedGraphemes::default();
        };
        let field = &self.fields[index];
        let Some(status) = self.field_status(field) else {
            return CreatedGraphemes::default();
        };
        text_graphemes(
            format!("\n{}", status.text),
            Some(ContentStyle {
                foreground_color: Some(status.color),
                ..Default::default()
            }),
        )
    }

    fn field_status(&self, field: &InputField) -> Option<StatusMessage> {
        if let Some(message) = field.loading_message() {
            return Some(StatusMessage::new(
                format!("{} {message}", frame::DOTS[self.spinner_frame]),
                Color::DarkCyan,
            ));
        }
        if let Some(value) = field.custom_value() {
            let retry = field
                .error_message()
                .map(|_| " · Ctrl+R retries candidates")
                .unwrap_or_default();
            return Some(StatusMessage::new(
                format!(
                    "No candidate — Enter uses “{}”{retry}",
                    compact_value(&value)
                ),
                Color::DarkYellow,
            ));
        }
        if let Some(error) = field.error_message() {
            return Some(StatusMessage::new(
                format!("✗ {error}\nPress Ctrl+R to retry"),
                Color::DarkRed,
            ));
        }
        if self.show_required_errors && field.is_required() && field.value().is_none() {
            return Some(StatusMessage::new(
                format!("{} is required", field.title()),
                Color::DarkRed,
            ));
        }
        let values = self.saved_values();
        let missing_dependencies = field.missing_dependencies(&values);
        if !missing_dependencies.is_empty() {
            return Some(StatusMessage::new(
                format!("Complete {} first", missing_dependencies.join(", ")),
                Color::DarkYellow,
            ));
        }
        None
    }

    fn help_page_graphemes() -> CreatedGraphemes {
        text_graphemes(
            concat!(
                "Convey help\n",
                "\n",
                "Navigation\n",
                "  Tab / Down       Move to the next candidate or section\n",
                "  Shift+Tab / Up   Move to the previous candidate or section\n",
                "  Enter            Save a selection and move\n",
                "  Ctrl+Enter       Save a textarea and move\n",
                "\n",
                "Commands\n",
                "  Ctrl+R           Refresh destinations or retry candidates\n",
                "  Ctrl+S           Submit\n",
                "  Esc / Ctrl+C     Cancel\n",
                "\n",
                "Press Ctrl+G or Esc to return",
            ),
            None,
        )
    }

    fn process_event(&mut self, event: &Event) -> InputScreenAction {
        if self.page == Page::Help {
            return self.process_help_event(event);
        }
        if let Some(action) = self.process_mouse_event(event) {
            return action;
        }

        let Event::Key(key) = event else {
            return InputScreenAction::Redraw;
        };
        if key.kind != KeyEventKind::Press {
            return InputScreenAction::Redraw;
        }

        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('g'), KeyModifiers::CONTROL)
        ) {
            self.page = Page::Help;
            return InputScreenAction::Redraw;
        }

        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Esc, KeyModifiers::NONE) | (KeyCode::Char('c'), KeyModifiers::CONTROL)
        ) {
            return InputScreenAction::Cancel;
        }
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Char('s'), KeyModifiers::CONTROL)
        ) {
            return self.save_focused_and_submit();
        }
        if let Some(direction) = navigation_direction(key.code, key.modifiers) {
            return self.navigate(direction);
        }
        match self.focus {
            Focus::Target => self.process_target_event(event),
            Focus::Workflow => self.process_workflow_event(event),
            Focus::Field(index) => self.process_field_event(index, event),
        }
    }

    fn process_help_event(&mut self, event: &Event) -> InputScreenAction {
        let Event::Key(key) = event else {
            return InputScreenAction::Redraw;
        };
        if key.kind != KeyEventKind::Press {
            return InputScreenAction::Redraw;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('g'), KeyModifiers::CONTROL) | (KeyCode::Esc, KeyModifiers::NONE) => {
                self.page = Page::Input;
                InputScreenAction::Redraw
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => InputScreenAction::Cancel,
            _ => InputScreenAction::Redraw,
        }
    }

    fn process_mouse_event(&mut self, event: &Event) -> Option<InputScreenAction> {
        let Event::Mouse(mouse) = event else {
            return None;
        };
        match mouse {
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            } => {
                let position = self.renderer.as_ref().and_then(|renderer| {
                    renderer.hit_test(ScreenPosition {
                        row: *row,
                        column: *column,
                    })
                });
                Some(position.map_or(InputScreenAction::Redraw, |position| {
                    self.process_click(position)
                }))
            }
            MouseEvent {
                kind: MouseEventKind::ScrollUp | MouseEventKind::ScrollDown,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.handle_scroll_event(event);
                Some(InputScreenAction::Redraw)
            }
            _ => Some(InputScreenAction::Redraw),
        }
    }

    fn handle_scroll_event(&mut self, event: &Event) {
        match self.focus {
            Focus::Target => self.target.handle_event(event),
            Focus::Workflow => self.workflow.handle_event(event),
            Focus::Field(index) => {
                let Event::Mouse(mouse) = event else {
                    return;
                };
                let next = match mouse.kind {
                    MouseEventKind::ScrollUp => index.saturating_sub(1),
                    MouseEventKind::ScrollDown => index
                        .saturating_add(1)
                        .min(self.fields.len().saturating_sub(1)),
                    _ => return,
                };
                self.focus_on(Focus::Field(next));
            }
        }
    }

    fn process_click(&mut self, position: WidgetPosition<Region>) -> InputScreenAction {
        let content_position = position.content_position();
        match position.index {
            Region::TargetValue => self.focus_on(Focus::Target),
            Region::TargetEditor if self.focus == Focus::Target => {
                self.target.move_editor_cursor_to(content_position);
            }
            Region::TargetCandidates if self.focus == Focus::Target => {
                if self.target.select_at(content_position) && self.target.save() {
                    self.focus_next();
                }
            }
            Region::WorkflowValue => self.focus_on(Focus::Workflow),
            Region::WorkflowEditor if self.focus == Focus::Workflow => {
                self.workflow.move_editor_cursor_to(content_position);
            }
            Region::WorkflowCandidates if self.focus == Focus::Workflow => {
                if self.workflow.select_at(content_position) && self.save_workflow() {
                    self.focus_next();
                }
            }
            Region::Fields => {
                if position.row < self.fields.len() {
                    self.focus_on(Focus::Field(position.row));
                }
            }
            Region::Editor => {
                if let Focus::Field(index) = self.focus {
                    self.fields[index].move_editor_cursor_to(content_position);
                }
            }
            Region::Candidates => {
                if let Focus::Field(index) = self.focus
                    && self.fields[index].select_at(content_position)
                {
                    return self.save_and_advance(index);
                }
            }
            Region::TargetHeader
            | Region::TargetStatus
            | Region::WorkflowHeader
            | Region::WorkflowStatus
            | Region::WorkflowSpacer
            | Region::Status
            | Region::Page
            | Region::TargetEditor
            | Region::TargetCandidates
            | Region::WorkflowEditor
            | Region::WorkflowCandidates => {}
        }
        InputScreenAction::Redraw
    }

    fn focus_on(&mut self, focus: Focus) {
        if self.focus == focus {
            return;
        }
        let can_leave = match self.focus {
            Focus::Target => self.target.save(),
            Focus::Workflow => self.save_workflow(),
            Focus::Field(index) => self.save_field(index),
        };
        if !can_leave {
            return;
        }
        self.focus = focus;
    }

    fn process_field_event(&mut self, index: usize, event: &Event) -> InputScreenAction {
        let Event::Key(key) = event else {
            return InputScreenAction::Redraw;
        };
        match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::CONTROL) if self.fields[index].is_textarea() => {
                self.save_and_advance(index)
            }
            (KeyCode::Enter, KeyModifiers::NONE) if !self.fields[index].is_textarea() => {
                self.save_and_advance(index)
            }
            (KeyCode::Char('r'), KeyModifiers::CONTROL)
                if self.fields[index].error_message().is_some() =>
            {
                self.fields[index].retry();
                InputScreenAction::Redraw
            }
            _ if self.fields[index].is_loading() => InputScreenAction::Redraw,
            _ => {
                self.fields[index].handle_event(event);
                InputScreenAction::Redraw
            }
        }
    }

    fn process_target_event(&mut self, event: &Event) -> InputScreenAction {
        let Event::Key(key) = event else {
            return InputScreenAction::Redraw;
        };
        match (key.code, key.modifiers) {
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                self.target.request_refresh();
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.target.save() {
                    self.focus_next();
                }
            }
            _ => {
                self.target.handle_event(event);
            }
        }
        InputScreenAction::Redraw
    }

    fn process_workflow_event(&mut self, event: &Event) -> InputScreenAction {
        let Event::Key(key) = event else {
            return InputScreenAction::Redraw;
        };
        match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.save_workflow() {
                    self.focus_next();
                }
            }
            _ => self.workflow.handle_event(event),
        }
        InputScreenAction::Redraw
    }

    fn navigate(&mut self, direction: NavigationDirection) -> InputScreenAction {
        let moved_within_control = match self.focus {
            Focus::Target => self.target.navigate(direction),
            Focus::Workflow => self.workflow.navigate(direction),
            Focus::Field(index) => self.fields[index].navigate(direction),
        };
        if moved_within_control {
            return InputScreenAction::Redraw;
        }

        let saved = match self.focus {
            Focus::Target => self.target.save(),
            Focus::Workflow => self.save_workflow(),
            Focus::Field(index) => self.save_field(index),
        };
        if saved {
            match direction {
                NavigationDirection::Previous => self.focus_previous(),
                NavigationDirection::Next => self.focus_next(),
            }
        }
        InputScreenAction::Redraw
    }

    fn save_and_advance(&mut self, index: usize) -> InputScreenAction {
        if !self.save_field(index) {
            return InputScreenAction::Redraw;
        }
        self.focus_next();
        InputScreenAction::Redraw
    }

    fn save_focused_and_submit(&mut self) -> InputScreenAction {
        self.show_required_errors = true;
        let saved = match self.focus {
            Focus::Target => self.target.save(),
            Focus::Workflow => self.save_workflow(),
            Focus::Field(index) => self.save_field(index),
        };
        if !saved {
            return InputScreenAction::Redraw;
        }
        self.submit()
    }

    fn submit(&mut self) -> InputScreenAction {
        if self.target.selected().is_none() {
            self.focus = Focus::Target;
            InputScreenAction::Redraw
        } else if self.workflow.selected().is_none() {
            self.focus = Focus::Workflow;
            InputScreenAction::Redraw
        } else {
            if self.all_fields_can_submit() {
                return InputScreenAction::Submit;
            }
            if let Some(index) = self
                .fields
                .iter()
                .position(|field| field.value().is_none() && !field.allow_empty())
            {
                self.focus = Focus::Field(index);
            }
            InputScreenAction::Redraw
        }
    }

    fn save_field(&mut self, index: usize) -> bool {
        let Some(outcome) = self.fields[index].save() else {
            return false;
        };
        if outcome.changed {
            let name = self.fields[index].name().to_owned();
            self.invalidate_dependents(&name);
        }
        true
    }

    fn save_workflow(&mut self) -> bool {
        let Some(changed) = self.workflow.save() else {
            return false;
        };
        if changed {
            self.replace_fields();
        }
        true
    }

    fn replace_fields(&mut self) {
        if let Some(task) = self.running_task.take() {
            task.handle.abort();
        }
        self.fields = self
            .workflow
            .selected()
            .into_iter()
            .flat_map(Workflow::inputs)
            .map(InputField::from_definition)
            .collect();
        self.show_required_errors = false;
    }

    fn invalidate_dependents(&mut self, changed: &str) {
        let mut invalidated = BTreeSet::from([changed.to_owned()]);
        for (index, field) in self.fields.iter_mut().enumerate() {
            if invalidated
                .iter()
                .any(|dependency| field.depends_on(dependency))
            {
                invalidated.insert(field.name().to_owned());
                field.invalidate();
                if self
                    .running_task
                    .as_ref()
                    .is_some_and(|task| task.field == index)
                    && let Some(task) = self.running_task.take()
                {
                    task.handle.abort();
                }
            }
        }
    }

    fn focus_next(&mut self) {
        self.focus = match self.focus {
            Focus::Target => Focus::Workflow,
            Focus::Workflow if self.fields.is_empty() => Focus::Target,
            Focus::Workflow => Focus::Field(0),
            Focus::Field(index) if index + 1 < self.fields.len() => Focus::Field(index + 1),
            Focus::Field(_) => Focus::Target,
        };
    }

    fn focus_previous(&mut self) {
        self.focus = match self.focus {
            Focus::Target if self.fields.is_empty() => Focus::Workflow,
            Focus::Target => Focus::Field(self.fields.len() - 1),
            Focus::Workflow => Focus::Target,
            Focus::Field(0) => Focus::Workflow,
            Focus::Field(index) => Focus::Field(index - 1),
        };
    }

    fn all_fields_can_submit(&self) -> bool {
        self.fields
            .iter()
            .all(|field| field.value().is_some() || field.allow_empty())
    }

    fn saved_values(&self) -> BTreeMap<String, String> {
        self.fields
            .iter()
            .filter_map(|field| {
                field
                    .value()
                    .map(|value| (field.name().to_owned(), value.to_owned()))
            })
            .collect()
    }

    fn start_active_task(&mut self) {
        let Focus::Field(index) = self.focus else {
            return;
        };
        let values = self.saved_values();
        if self.running_task.is_some()
            || !self.fields[index].needs_load()
            || !self.fields[index].missing_dependencies(&values).is_empty()
        {
            return;
        }
        match self.fields[index].command_task(&values) {
            Ok(Some(task)) => {
                let id = self.next_task_id;
                self.next_task_id = self.next_task_id.wrapping_add(1);
                let handle = task.spawn(id, self.task_tx.clone());
                self.running_task = Some(RunningTask {
                    id,
                    field: index,
                    handle,
                });
            }
            Ok(None) => {}
            Err(error) => self.fields[index].finish_load(Err(error)),
        }
    }

    fn apply_task_event(&mut self, (id, result): CommandTaskResult) {
        let Some(task) = self.running_task.take() else {
            return;
        };
        if task.id != id {
            self.running_task = Some(task);
            return;
        }
        let values = result.map(|output| decode_candidates(&output));
        self.fields[task.field].finish_load(values);
    }
}

impl Drop for InputScreen {
    fn drop(&mut self) {
        if let Some(task) = self.running_task.take() {
            task.handle.abort();
        }
    }
}

fn navigation_direction(code: KeyCode, modifiers: KeyModifiers) -> Option<NavigationDirection> {
    match (code, modifiers) {
        (KeyCode::Tab | KeyCode::Down, KeyModifiers::NONE) => Some(NavigationDirection::Next),
        (KeyCode::BackTab, KeyModifiers::NONE | KeyModifiers::SHIFT)
        | (KeyCode::Up, KeyModifiers::NONE) => Some(NavigationDirection::Previous),
        _ => None,
    }
}

fn compact_value(value: &str) -> String {
    const LIMIT: usize = 48;

    let mut compact = String::with_capacity(LIMIT);
    let mut length = 0;
    for character in value.trim().chars() {
        let replacement = if character == '\n' {
            &[' ', '↵', ' '][..]
        } else {
            std::slice::from_ref(&character)
        };
        for &character in replacement {
            if length == LIMIT {
                compact.push('…');
                return compact;
            }
            compact.push(character);
            length += 1;
        }
    }

    if compact.is_empty() {
        "—".to_owned()
    } else {
        compact
    }
}

fn empty_value() -> String {
    "—".to_owned()
}

fn focused_field_style() -> ContentStyle {
    ContentStyle {
        background_color: Some(Color::DarkCyan),
        ..Default::default()
    }
}

fn text_graphemes(value: impl Into<String>, style: Option<ContentStyle>) -> CreatedGraphemes {
    text::State {
        text: Text::from(value.into()),
        config: text::Config {
            style,
            ..Default::default()
        },
    }
    .create_graphemes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        automation::Terminal,
        input::{CommandCandidates, InputKind, SelectCandidates, SelectDefinition},
    };
    use promkit::core::{crossterm::event::KeyEvent, render::RendererLayout};

    fn definitions() -> Vec<InputDefinition> {
        vec![
            InputDefinition {
                name: "context".into(),
                allow_empty: true,
                kind: InputKind::Select(SelectDefinition {
                    depends_on: Vec::new(),
                    candidates: SelectCandidates::Values(vec!["local".into(), "production".into()]),
                }),
            },
            InputDefinition {
                name: "namespace".into(),
                allow_empty: true,
                kind: InputKind::Select(SelectDefinition {
                    depends_on: vec!["context".into()],
                    candidates: SelectCandidates::Values(vec!["default".into()]),
                }),
            },
            InputDefinition {
                name: "request".into(),
                allow_empty: true,
                kind: InputKind::Textarea,
            },
        ]
    }

    fn terminal() -> Terminal {
        Terminal {
            id: "42".into(),
            name: "agent".into(),
            working_directory: "/workspace".into(),
            window_index: 1,
            tab_index: 1,
            terminal_index: 1,
        }
    }

    fn target() -> TerminalTarget {
        TerminalTarget {
            application: TerminalApplication::Ghostty,
            terminal: terminal(),
        }
    }

    fn input_screen(definitions: &[InputDefinition]) -> InputScreen {
        InputScreen::new(
            "test",
            definitions,
            BTreeSet::from([TerminalApplication::Ghostty]),
        )
    }

    fn textarea_definitions(count: usize) -> Vec<InputDefinition> {
        (1..=count)
            .map(|index| InputDefinition {
                name: format!("field_{index}"),
                allow_empty: true,
                kind: InputKind::Textarea,
            })
            .collect()
    }

    fn select_definitions(field_count: usize, candidate_count: usize) -> Vec<InputDefinition> {
        let candidates = (1..=candidate_count)
            .map(|index| format!("candidate_{index}"))
            .collect::<Vec<_>>();
        (1..=field_count)
            .map(|index| InputDefinition {
                name: format!("field_{index}"),
                allow_empty: true,
                kind: InputKind::Select(SelectDefinition {
                    depends_on: Vec::new(),
                    candidates: SelectCandidates::Values(candidates.clone()),
                }),
            })
            .collect()
    }

    fn pane_heights(screen: &InputScreen, terminal_height: u16) -> Vec<usize> {
        RendererLayout::default()
            .layout(screen.rendered_regions(), 80, terminal_height)
            .unwrap()
            .panes()
            .iter()
            .map(Vec::len)
            .collect()
    }

    fn make_target_available(screen: &mut InputScreen) {
        screen
            .target
            .apply_update(terminal_target::TerminalTargetUpdate::RefreshFinished(Ok(
                vec![target()],
            )));
    }

    fn advance_to_first_field(screen: &mut InputScreen) {
        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(screen.focus, Focus::Workflow);
        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(screen.focus, Focus::Field(0));
    }

    fn click(region: Region, row: usize) -> WidgetPosition<Region> {
        WidgetPosition {
            index: region,
            row,
            column: 0,
        }
    }

    fn scroll(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn compacts_values_without_scanning_past_the_display_limit() {
        assert_eq!(compact_value(""), "—");
        assert_eq!(compact_value("  \n"), "—");
        assert_eq!(compact_value("a\nb"), "a ↵ b");
        assert_eq!(compact_value(&"x".repeat(48)), "x".repeat(48));
        assert_eq!(
            compact_value(&"x".repeat(49)),
            format!("{}…", "x".repeat(48))
        );
    }

    #[test]
    fn direct_workflow_file_starts_selected_with_its_fields_loaded() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/kubernetes-investigation.yaml");

        let screen =
            InputScreen::from_workflow_path(&path, BTreeSet::from([TerminalApplication::Ghostty]))
                .unwrap();

        assert_eq!(
            screen.workflow.selected().unwrap().name(),
            "kubernetes-investigation"
        );
        assert_eq!(screen.fields.len(), 5);
        assert!(
            screen
                .workflow_value_graphemes()
                .graphemes
                .to_string()
                .starts_with('✓')
        );
    }

    #[test]
    fn workflow_directory_starts_with_the_selector_pending() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");

        let screen =
            InputScreen::from_workflow_path(&path, BTreeSet::from([TerminalApplication::Ghostty]))
                .unwrap();

        assert!(screen.workflow.selected().is_none());
        assert!(screen.fields.is_empty());
        assert!(
            screen
                .workflow_value_graphemes()
                .graphemes
                .to_string()
                .starts_with('○')
        );
    }

    #[test]
    fn workflow_selection_leaves_one_row_before_the_input_fields() {
        let mut screen = input_screen(&definitions());
        screen.focus = Focus::Workflow;

        let regions = screen.rendered_regions();
        let spacer = regions
            .iter()
            .find(|(region, _)| *region == Region::WorkflowSpacer)
            .map(|(_, graphemes)| graphemes)
            .unwrap();

        assert_eq!(spacer.graphemes.logical_lines().len(), 1);
        assert_eq!(spacer.graphemes.to_string(), " ");
    }

    #[test]
    fn control_g_toggles_the_help_page_and_escape_also_returns_to_inputs() {
        let mut screen = input_screen(&definitions());

        let open = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(open, InputScreenAction::Redraw));
        assert_eq!(screen.page, Page::Help);
        let regions = screen.rendered_regions();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].0, Region::Page);
        assert!(regions[0].1.graphemes.to_string().contains("Convey help"));
        let updates = screen.rendered_region_updates();
        assert!(
            updates
                .iter()
                .find(|(region, _)| *region == Region::TargetHeader)
                .unwrap()
                .1
                .graphemes
                .to_string()
                .is_empty()
        );

        let close = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(close, InputScreenAction::Redraw));
        assert_eq!(screen.page, Page::Input);

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )));
        let close =
            screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));

        assert!(matches!(close, InputScreenAction::Redraw));
        assert_eq!(screen.page, Page::Input);
        let updates = screen.rendered_region_updates();
        assert!(
            updates
                .iter()
                .find(|(region, _)| *region == Region::Page)
                .unwrap()
                .1
                .graphemes
                .to_string()
                .is_empty()
        );
        assert!(
            screen
                .rendered_regions()
                .iter()
                .all(|(region, _)| *region != Region::Page)
        );
    }

    #[test]
    fn control_c_cancels_from_the_help_page() {
        let mut screen = input_screen(&definitions());
        screen.page = Page::Help;

        let action = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(action, InputScreenAction::Cancel));
    }

    #[test]
    fn caps_fields_at_half_when_textarea_content_is_short() {
        let definitions = textarea_definitions(64);
        let mut screen = input_screen(&definitions);
        screen.focus = Focus::Field(63);

        assert_eq!(pane_heights(&screen, 24), [2, 2, 3, 2, 8, 1]);
        assert_eq!(pane_heights(&screen, 40), [2, 2, 3, 2, 16, 1]);
    }

    #[test]
    fn shares_available_height_between_fields_and_select_candidates() {
        let definitions = select_definitions(64, 64);
        let mut screen = input_screen(&definitions);
        screen.focus = Focus::Field(63);

        assert_eq!(pane_heights(&screen, 24), [2, 2, 3, 2, 7, 1, 7]);
        assert_eq!(pane_heights(&screen, 40), [2, 2, 3, 2, 15, 1, 15]);
    }

    #[test]
    fn caps_fields_at_half_when_select_candidates_are_empty() {
        let definitions = select_definitions(64, 0);
        let mut screen = input_screen(&definitions);
        screen.focus = Focus::Field(63);

        assert_eq!(pane_heights(&screen, 24), [2, 2, 3, 2, 7, 1, 1]);
        assert_eq!(pane_heights(&screen, 40), [2, 2, 3, 2, 15, 1, 1]);
    }

    #[test]
    fn tab_and_arrows_navigate_seamlessly_across_candidates_and_sections() {
        let mut screen = input_screen(&definitions());
        make_target_available(&mut screen);

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(screen.focus, Focus::Workflow);
        assert_eq!(screen.target.selected(), Some(&target()));

        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Field(0));

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(screen.focus, Focus::Field(0));
        assert_eq!(screen.fields[0].draft().as_deref(), Some("production"));
        assert_eq!(screen.fields[0].value(), None);

        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Field(1));
        assert_eq!(screen.fields[0].value(), Some("production"));
        assert_eq!(
            screen.saved_values().get("context"),
            Some(&"production".to_owned())
        );
        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Field(0));

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )));
        assert_eq!(screen.focus, Focus::Field(0));
        assert_eq!(screen.fields[0].draft().as_deref(), Some("local"));

        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Workflow);
        assert_eq!(screen.fields[0].value(), Some("local"));
    }

    #[test]
    fn tab_and_arrows_traverse_textarea_lines_before_leaving_the_field() {
        let mut screen = input_screen(&definitions());
        screen.focus = Focus::Field(2);
        for code in [KeyCode::Char('a'), KeyCode::Enter, KeyCode::Char('b')] {
            screen.process_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        }

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )));
        assert_eq!(screen.focus, Focus::Field(2));

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(screen.focus, Focus::Field(2));

        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Target);
        assert_eq!(screen.fields[2].value(), Some("a\nb"));
    }

    #[test]
    fn clicking_a_field_moves_focus_and_saves_the_current_field() {
        let mut screen = input_screen(&definitions());
        make_target_available(&mut screen);
        screen.process_click(click(Region::Fields, 2));
        screen.fields[2].handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));

        screen.process_click(click(Region::Fields, 0));

        assert_eq!(screen.focus, Focus::Field(0));
        assert_eq!(screen.fields[2].value(), Some("x"));
        assert_eq!(screen.saved_values().get("request"), Some(&"x".to_owned()));
    }

    #[test]
    fn clicking_a_select_candidate_saves_it_and_advances() {
        let mut screen = input_screen(&definitions());
        screen.focus = Focus::Field(0);

        screen.process_click(click(Region::Candidates, 1));

        assert_eq!(screen.fields[0].value(), Some("production"));
        assert_eq!(
            screen.saved_values().get("context"),
            Some(&"production".to_owned())
        );
        assert_eq!(screen.focus, Focus::Field(1));
    }

    #[test]
    fn mouse_wheel_moves_between_input_fields() {
        let mut screen = input_screen(&definitions());
        screen.focus = Focus::Field(0);

        screen.process_event(&scroll(MouseEventKind::ScrollDown));

        assert_eq!(screen.focus, Focus::Field(1));
        assert_eq!(screen.fields[0].value(), Some("local"));

        screen.process_event(&scroll(MouseEventKind::ScrollUp));

        assert_eq!(screen.focus, Focus::Field(0));
        assert_eq!(screen.fields[0].draft().as_deref(), Some("local"));
    }

    #[test]
    fn mouse_wheel_does_not_wrap_past_the_field_list() {
        let mut screen = input_screen(&definitions());
        screen.focus = Focus::Field(0);

        screen.process_event(&scroll(MouseEventKind::ScrollUp));
        assert_eq!(screen.focus, Focus::Field(0));

        screen.focus = Focus::Field(screen.fields.len() - 1);
        screen.process_event(&scroll(MouseEventKind::ScrollDown));
        assert_eq!(screen.focus, Focus::Field(screen.fields.len() - 1));
    }

    #[test]
    fn focused_field_has_a_background_and_drives_the_fields_viewport() {
        use promkit::core::render::RendererLayout;

        let definitions = (1..=64)
            .map(|index| InputDefinition {
                name: format!("field_{index:02}"),
                allow_empty: true,
                kind: InputKind::Textarea,
            })
            .collect::<Vec<_>>();
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);
        assert!(screen.target.save());
        screen.focus = Focus::Field(63);

        let fields = screen.fields_graphemes();
        assert_eq!(fields.cursor, Some(ContentPosition { row: 63, column: 0 }));
        let focused = fields.graphemes.logical_lines()[63].clone();
        let expected = StyledGraphemes::from_str("▶ field 64: —", focused_field_style());
        assert_eq!(
            focused.styled_display().to_string(),
            expected.styled_display().to_string()
        );

        let prepared = RendererLayout::default()
            .layout(screen.rendered_regions(), 80, 24)
            .unwrap();
        assert!(
            prepared
                .panes()
                .iter()
                .flatten()
                .any(|row| { row.to_string() == "▶ field 64: —" })
        );
    }

    #[test]
    fn clicking_a_destination_candidate_saves_it_and_advances() {
        let mut screen = input_screen(&definitions());
        screen
            .target
            .apply_update(terminal_target::TerminalTargetUpdate::RefreshFinished(Ok(
                vec![
                    target(),
                    TerminalTarget {
                        application: TerminalApplication::Ghostty,
                        terminal: Terminal {
                            id: "43".into(),
                            name: "other".into(),
                            ..terminal()
                        },
                    },
                ],
            )));

        screen.process_click(click(Region::TargetCandidates, 4));

        assert_eq!(
            screen
                .target
                .selected()
                .map(|target| target.terminal.id.as_str()),
            Some("43")
        );
        assert_eq!(screen.focus, Focus::Workflow);
    }

    #[test]
    fn control_enter_saves_a_textarea() {
        let mut screen = input_screen(&definitions());
        screen.focus = Focus::Field(2);
        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )));

        assert_eq!(screen.fields[2].value(), Some("x"));
    }

    #[test]
    fn only_control_s_submits_the_complete_form() {
        let definitions = vec![InputDefinition {
            name: "target".into(),
            allow_empty: true,
            kind: InputKind::Select(SelectDefinition {
                depends_on: Vec::new(),
                candidates: SelectCandidates::Values(vec!["local".into()]),
            }),
        }];
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);

        let enter = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(matches!(enter, InputScreenAction::Redraw));
        assert_eq!(screen.focus, Focus::Workflow);

        let enter = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(matches!(enter, InputScreenAction::Redraw));
        assert_eq!(screen.focus, Focus::Field(0));

        let enter = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(matches!(enter, InputScreenAction::Redraw));
        assert_eq!(screen.fields[0].value(), Some("local"));

        let submit = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));
        assert!(matches!(submit, InputScreenAction::Submit));
    }

    #[test]
    fn empty_optional_input_can_be_submitted() {
        let definitions = vec![InputDefinition {
            name: "request".into(),
            allow_empty: true,
            kind: InputKind::Textarea,
        }];
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);
        advance_to_first_field(&mut screen);

        let submit = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(submit, InputScreenAction::Submit));
        assert_eq!(screen.fields[0].value(), None);
        assert!(!screen.saved_values().contains_key("request"));
    }

    #[test]
    fn empty_required_inputs_do_not_block_focus_navigation() {
        let definitions = vec![
            InputDefinition {
                name: "request".into(),
                allow_empty: false,
                kind: InputKind::Textarea,
            },
            InputDefinition {
                name: "category".into(),
                allow_empty: false,
                kind: InputKind::Select(SelectDefinition {
                    depends_on: Vec::new(),
                    candidates: SelectCandidates::Values(Vec::new()),
                }),
            },
            InputDefinition {
                name: "details".into(),
                allow_empty: false,
                kind: InputKind::Textarea,
            },
        ];
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);
        advance_to_first_field(&mut screen);

        assert_eq!(screen.focus, Focus::Field(0));
        assert!(screen.field_status(&screen.fields[0]).is_none());

        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Field(1));

        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Field(2));

        screen.process_event(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        assert_eq!(screen.focus, Focus::Target);

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )));
        assert_eq!(screen.focus, Focus::Field(2));

        screen.focus_on(Focus::Field(0));
        assert_eq!(screen.focus, Focus::Field(0));

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )));
        assert_eq!(screen.focus, Focus::Field(1));
    }

    #[test]
    fn empty_required_input_prevents_submit_and_shows_required() {
        let definitions = vec![InputDefinition {
            name: "request".into(),
            allow_empty: false,
            kind: InputKind::Textarea,
        }];
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);
        advance_to_first_field(&mut screen);

        assert!(screen.field_status(&screen.fields[0]).is_none());

        let submit = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(submit, InputScreenAction::Redraw));
        assert_eq!(screen.focus, Focus::Field(0));
        assert_eq!(
            screen
                .field_status(&screen.fields[0])
                .map(|status| status.text),
            Some("request is required".to_owned())
        );
        assert!(!screen.saved_values().contains_key("request"));
    }

    #[test]
    fn submit_saves_only_the_focused_field() {
        let definitions = vec![
            InputDefinition {
                name: "environment".into(),
                allow_empty: false,
                kind: InputKind::Select(SelectDefinition {
                    depends_on: Vec::new(),
                    candidates: SelectCandidates::Values(vec!["production".into()]),
                }),
            },
            InputDefinition {
                name: "request".into(),
                allow_empty: false,
                kind: InputKind::Textarea,
            },
        ];
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);
        advance_to_first_field(&mut screen);

        let submit = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(submit, InputScreenAction::Redraw));
        assert_eq!(screen.fields[0].value(), Some("production"));
        assert_eq!(screen.fields[1].value(), None);
        assert_eq!(
            screen.saved_values().get("environment").map(String::as_str),
            Some("production")
        );
        assert!(!screen.saved_values().contains_key("request"));
        assert_eq!(screen.focus, Focus::Field(1));
        assert_eq!(
            screen
                .field_status(&screen.fields[1])
                .map(|status| status.text),
            Some("request is required".to_owned())
        );
    }

    #[test]
    fn submit_saves_the_current_draft() {
        let definitions = vec![InputDefinition {
            name: "request".into(),
            allow_empty: false,
            kind: InputKind::Textarea,
        }];
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);
        advance_to_first_field(&mut screen);
        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )));

        let submit = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(submit, InputScreenAction::Submit));
        assert_eq!(screen.focus, Focus::Field(0));
        assert_eq!(
            screen.saved_values().get("request").map(String::as_str),
            Some("x")
        );
    }

    #[test]
    fn changing_a_value_invalidates_dependent_fields() {
        let mut screen = input_screen(&definitions());
        screen.focus = Focus::Field(0);
        assert!(screen.save_field(0));
        screen.focus = Focus::Field(1);
        assert!(screen.save_field(1));
        assert_eq!(
            screen.saved_values().get("namespace"),
            Some(&"default".to_owned())
        );

        screen.focus = Focus::Field(0);
        screen.fields[0].handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert!(screen.save_field(0));

        assert_eq!(
            screen.saved_values().get("context"),
            Some(&"production".to_owned())
        );
        assert!(!screen.saved_values().contains_key("namespace"));
        assert!(screen.fields[1].value().is_none());
    }

    #[test]
    fn plain_r_remains_available_after_a_candidate_task_failure() {
        let definitions = vec![InputDefinition {
            name: "target".into(),
            allow_empty: true,
            kind: InputKind::Select(SelectDefinition {
                depends_on: Vec::new(),
                candidates: SelectCandidates::Command(CommandCandidates {
                    program: "false".into(),
                    args: Vec::new(),
                }),
            }),
        }];
        let mut screen = input_screen(&definitions);
        screen.focus = Focus::Field(0);
        screen.fields[0].finish_load(Err(anyhow::anyhow!("failed")));

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
        )));

        assert_eq!(screen.fields[0].draft().as_deref(), Some("r"));
        assert!(screen.fields[0].error_message().is_some());

        screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
        )));

        assert!(screen.fields[0].needs_load());
    }

    #[test]
    fn target_selection_can_be_cancelled() {
        let mut screen = input_screen(&definitions());
        assert!(matches!(
            screen.process_event(&Event::Key(
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE,)
            )),
            InputScreenAction::Cancel
        ));
        assert!(matches!(
            screen.process_event(&Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            ))),
            InputScreenAction::Cancel
        ));
    }

    #[test]
    fn submitting_without_a_destination_shows_required() {
        let mut screen = input_screen(&definitions());

        assert!(screen.target_status().is_none());

        let submit = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(submit, InputScreenAction::Redraw));
        assert_eq!(screen.focus, Focus::Target);
        assert_eq!(
            screen.target_status().map(|status| status.text),
            Some("destination is required".to_owned())
        );
        assert!(screen.target.selected().is_none());
    }

    #[test]
    fn submit_saves_the_focused_destination() {
        let definitions = vec![InputDefinition {
            name: "request".into(),
            allow_empty: true,
            kind: InputKind::Textarea,
        }];
        let mut screen = input_screen(&definitions);
        make_target_available(&mut screen);

        let submit = screen.process_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));

        assert!(matches!(submit, InputScreenAction::Submit));
        assert_eq!(screen.target.selected(), Some(&target()));
        assert_eq!(screen.fields[0].value(), None);
    }
}
