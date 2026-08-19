use std::{collections::BTreeSet, time::Duration};

use anyhow::{Context, Result};
use promkit::core::{ContentPosition, CreatedGraphemes, crossterm::event::Event};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

mod tree_select;

#[cfg(target_os = "macos")]
use crate::automation::{ghostty::GhosttyAutomation, iterm2::ITerm2Automation};
use crate::{
    automation::{Terminal, TerminalAutomation, tmux::TmuxAutomation},
    cli::TerminalApplication,
    tui::NavigationDirection,
};
use tree_select::{TreeSelect, TreeSelectItem};

const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalTarget {
    pub(crate) application: TerminalApplication,
    pub(crate) terminal: Terminal,
}

pub(super) struct TerminalTargetSelector {
    select: TreeSelect,
    selected_key: Option<String>,
    applications: BTreeSet<TerminalApplication>,
    refresh_task: Option<JoinHandle<()>>,
    refresh_request_tx: Option<mpsc::Sender<()>>,
    last_targets: Option<Vec<TerminalTarget>>,
    refresh_state: RefreshState,
}

pub(super) enum TerminalTargetUpdate {
    RefreshStarted,
    RefreshFinished(Result<Vec<TerminalTarget>>),
}

enum RefreshState {
    Idle,
    Loading,
    Failed(String),
}

impl TerminalTargetSelector {
    pub(super) fn new(applications: BTreeSet<TerminalApplication>) -> Self {
        Self {
            select: TreeSelect::new(20),
            selected_key: None,
            applications,
            refresh_task: None,
            refresh_request_tx: None,
            last_targets: None,
            refresh_state: RefreshState::Idle,
        }
    }

    pub(super) fn start(&mut self) -> mpsc::Receiver<TerminalTargetUpdate> {
        let (refresh_tx, refresh_rx) = mpsc::channel(2);
        let (refresh_request_tx, refresh_request_rx) = mpsc::channel(1);
        self.refresh_request_tx = Some(refresh_request_tx);
        self.refresh_task = Some(spawn_refresh_task(
            self.applications.clone(),
            refresh_tx,
            refresh_request_rx,
        ));
        refresh_rx
    }

    pub(super) fn request_refresh(&mut self) {
        if self.is_loading() {
            return;
        }
        if self
            .refresh_request_tx
            .as_ref()
            .is_some_and(|tx| tx.try_send(()).is_ok())
        {
            self.refresh_state = RefreshState::Loading;
        }
    }

    pub(super) fn apply_update(&mut self, update: TerminalTargetUpdate) {
        match update {
            TerminalTargetUpdate::RefreshStarted => {
                self.refresh_state = RefreshState::Loading;
            }
            TerminalTargetUpdate::RefreshFinished(result) => match result {
                Ok(targets) => self.apply_refresh(targets),
                Err(error) => {
                    self.refresh_state = RefreshState::Failed(format!("{error:#}"));
                }
            },
        }
    }

    fn apply_refresh(&mut self, targets: Vec<TerminalTarget>) {
        self.refresh_state = RefreshState::Idle;
        if self.last_targets.as_ref() == Some(&targets) {
            return;
        }

        self.select.replace_items(
            targets
                .iter()
                .map(|target| {
                    let terminal = &target.terminal;
                    TreeSelectItem {
                        key: target_key(target),
                        application: terminal_application_label(target.application).to_owned(),
                        window_index: terminal.window_index,
                        tab_index: terminal.tab_index,
                        pane_index: terminal.terminal_index,
                        name: terminal.name.clone(),
                        working_directory: terminal.working_directory.clone(),
                        id: terminal.id.clone(),
                    }
                })
                .collect(),
        );
        if self
            .selected_key
            .as_ref()
            .is_some_and(|key| targets.iter().all(|target| target_key(target) != *key))
        {
            self.selected_key = None;
        }
        self.last_targets = Some(targets);
    }

    pub(super) fn handle_event(&mut self, event: &Event) {
        self.select.handle_event(event);
    }

    pub(super) fn navigate(&mut self, direction: NavigationDirection) -> bool {
        self.select.navigate(direction)
    }

    pub(super) fn move_editor_cursor_to(&mut self, position: ContentPosition) {
        self.select.move_query_cursor_to(position);
    }

    pub(super) fn select_at(&mut self, position: ContentPosition) -> bool {
        self.select.select_at(position)
    }

    pub(super) fn save(&mut self) -> bool {
        let Some(selected_key) = self.select.selected_key() else {
            return false;
        };
        self.selected_key = Some(selected_key.to_owned());
        true
    }

    pub(super) fn selected(&self) -> Option<&TerminalTarget> {
        let selected_key = self.selected_key.as_deref()?;
        self.last_targets
            .as_ref()?
            .iter()
            .find(|target| target_key(target) == selected_key)
    }

    pub(super) fn value_label(&self) -> String {
        self.selected()
            .map_or_else(|| "not set".to_owned(), terminal_label)
    }

    pub(super) fn editor_graphemes(&self) -> CreatedGraphemes {
        self.select.query_graphemes()
    }

    pub(super) fn candidates_graphemes(&self) -> CreatedGraphemes {
        self.select.list_graphemes()
    }

    pub(super) const fn is_loading(&self) -> bool {
        matches!(self.refresh_state, RefreshState::Loading)
    }

    pub(super) fn status_message(&self) -> Option<String> {
        match &self.refresh_state {
            RefreshState::Loading => Some("Refreshing terminal panes…".to_owned()),
            RefreshState::Failed(error) => Some(format!(
                "Failed to refresh terminal panes: {error}\nPress Ctrl+R to retry"
            )),
            RefreshState::Idle if self.last_targets.as_ref().is_some_and(Vec::is_empty) => {
                Some("No terminal panes found. Press Ctrl+R or wait for refresh…".to_owned())
            }
            RefreshState::Idle => None,
        }
    }
}

impl Drop for TerminalTargetSelector {
    fn drop(&mut self) {
        if let Some(task) = self.refresh_task.take() {
            task.abort();
        }
    }
}

fn spawn_refresh_task(
    applications: BTreeSet<TerminalApplication>,
    refresh_tx: mpsc::Sender<TerminalTargetUpdate>,
    mut refresh_request_rx: mpsc::Receiver<()>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = interval(REFRESH_INTERVAL);
        ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = ticks.tick() => {}
                request = refresh_request_rx.recv() => {
                    if request.is_none() {
                        break;
                    }
                }
            }
            if refresh_tx
                .send(TerminalTargetUpdate::RefreshStarted)
                .await
                .is_err()
            {
                break;
            }
            let result = list_targets(&applications).await;
            if refresh_tx
                .send(TerminalTargetUpdate::RefreshFinished(result))
                .await
                .is_err()
            {
                break;
            }
            while refresh_request_rx.try_recv().is_ok() {}
            ticks.reset();
        }
    })
}

async fn list_targets(applications: &BTreeSet<TerminalApplication>) -> Result<Vec<TerminalTarget>> {
    let mut targets = Vec::new();
    for &application in applications {
        let terminals = match application {
            #[cfg(target_os = "macos")]
            TerminalApplication::Ghostty => GhosttyAutomation.list_targets().await,
            #[cfg(target_os = "macos")]
            TerminalApplication::Iterm2 => ITerm2Automation.list_targets().await,
            TerminalApplication::Tmux => TmuxAutomation.list_targets().await,
        }
        .with_context(|| format!("failed to list {} targets", application.name()))?;
        targets.extend(terminals.into_iter().map(|terminal| TerminalTarget {
            application,
            terminal,
        }));
    }
    targets.sort_by(|left, right| {
        let left_terminal = &left.terminal;
        let right_terminal = &right.terminal;
        (
            left.application,
            left_terminal.window_index,
            left_terminal.tab_index,
            left_terminal.terminal_index,
            &left_terminal.id,
        )
            .cmp(&(
                right.application,
                right_terminal.window_index,
                right_terminal.tab_index,
                right_terminal.terminal_index,
                &right_terminal.id,
            ))
    });

    Ok(targets)
}

fn target_key(target: &TerminalTarget) -> String {
    format!("{}:{}", target.application.name(), target.terminal.id)
}

fn terminal_label(target: &TerminalTarget) -> String {
    let terminal = &target.terminal;
    format!(
        "{} › Window {} › Tab {} › Pane {} · {} · {}",
        terminal_application_label(target.application),
        terminal.window_index,
        terminal.tab_index,
        terminal.terminal_index,
        terminal.name,
        terminal.working_directory,
    )
}

const fn terminal_application_label(application: TerminalApplication) -> &'static str {
    match application {
        #[cfg(target_os = "macos")]
        TerminalApplication::Ghostty => "Ghostty",
        #[cfg(target_os = "macos")]
        TerminalApplication::Iterm2 => "iTerm2",
        TerminalApplication::Tmux => "tmux",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use promkit::core::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn target(id: &str, terminal_index: usize) -> TerminalTarget {
        TerminalTarget {
            application: TerminalApplication::Tmux,
            terminal: Terminal {
                id: id.into(),
                name: format!("pane-{id}"),
                working_directory: "/workspace".into(),
                window_index: 1,
                tab_index: 1,
                terminal_index,
            },
        }
    }

    #[test]
    fn selects_a_terminal_pane() {
        let mut selector = TerminalTargetSelector::new(BTreeSet::from([TerminalApplication::Tmux]));
        selector.apply_refresh(vec![target("one", 1), target("two", 2)]);

        selector.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert!(selector.save());

        assert_eq!(selector.selected().unwrap().terminal.id, "two");
    }

    #[test]
    fn removes_a_saved_pane_when_it_disappears() {
        let mut selector = TerminalTargetSelector::new(BTreeSet::from([TerminalApplication::Tmux]));
        selector.apply_refresh(vec![target("one", 1)]);
        assert!(selector.save());

        selector.apply_refresh(Vec::new());

        assert!(selector.selected().is_none());
    }

    #[test]
    fn finishes_refreshing_when_the_targets_are_unchanged() {
        let targets = vec![target("one", 1)];
        let mut selector = TerminalTargetSelector::new(BTreeSet::from([TerminalApplication::Tmux]));
        selector.apply_refresh(targets.clone());
        selector.apply_update(TerminalTargetUpdate::RefreshStarted);

        selector.apply_update(TerminalTargetUpdate::RefreshFinished(Ok(targets)));

        assert!(!selector.is_loading());
    }

    #[tokio::test]
    async fn requests_an_immediate_refresh_only_once_while_loading() {
        let mut selector = TerminalTargetSelector::new(BTreeSet::from([TerminalApplication::Tmux]));
        let (refresh_request_tx, mut refresh_request_rx) = mpsc::channel(1);
        selector.refresh_request_tx = Some(refresh_request_tx);

        selector.request_refresh();
        selector.request_refresh();

        assert!(selector.is_loading());
        assert_eq!(refresh_request_rx.recv().await, Some(()));
        assert!(refresh_request_rx.try_recv().is_err());
    }
}
