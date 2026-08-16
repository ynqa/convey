use promkit::{
    core::{
        ContentPosition, CreatedGraphemes, HeightPolicy, Widget, WidgetLayout,
        crossterm::{
            event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers},
            style::{Color, ContentStyle},
        },
        grapheme::StyledGraphemes,
    },
    widgets::text_editor,
};

use crate::tui::NavigationDirection;

pub(in crate::tui) struct SelectItem<T> {
    key: String,
    label: String,
    value: T,
}

impl<T> SelectItem<T> {
    pub(in crate::tui) fn new(key: impl Into<String>, label: impl Into<String>, value: T) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value,
        }
    }
}

pub(in crate::tui) struct Select<T> {
    query: text_editor::State,
    items: Vec<SelectItem<T>>,
    suggestions: Vec<usize>,
    selected: Option<usize>,
}

impl<T> Select<T> {
    pub(in crate::tui) fn new() -> Self {
        Self {
            query: text_editor::State {
                config: text_editor::Config {
                    prefix: "❯❯ ".into(),
                    prefix_style: ContentStyle {
                        foreground_color: Some(Color::DarkGreen),
                        ..Default::default()
                    },
                    active_char_style: ContentStyle {
                        background_color: Some(Color::DarkCyan),
                        ..Default::default()
                    },
                    lines: Some(1),
                    ..Default::default()
                },
                ..Default::default()
            },
            items: Vec::new(),
            suggestions: Vec::new(),
            selected: None,
        }
    }

    pub(in crate::tui) fn replace_items(&mut self, items: Vec<SelectItem<T>>) {
        let selected_key = self.selected_key().map(str::to_owned);
        self.items = items;
        self.rebuild(selected_key.as_deref());
    }

    pub(in crate::tui) fn selected(&self) -> Option<&T> {
        self.selected_item().map(|item| &item.value)
    }

    pub(in crate::tui) fn selected_key(&self) -> Option<&str> {
        self.selected_item().map(|item| item.key.as_str())
    }

    pub(in crate::tui) fn query_graphemes(&self) -> CreatedGraphemes {
        self.query.create_graphemes()
    }

    pub(in crate::tui) fn list_graphemes(&self) -> CreatedGraphemes {
        let cursor = StyledGraphemes::from("❯ ");
        let mut lines = self
            .suggestions
            .iter()
            .enumerate()
            .filter_map(|(position, item)| {
                let label = &self.items.get(*item)?.label;
                let prefix = if Some(position) == self.selected {
                    &cursor
                } else {
                    &StyledGraphemes::from("  ")
                };
                let line = StyledGraphemes::from_iter([prefix, &StyledGraphemes::from(label)]);
                Some(if Some(position) == self.selected {
                    line.apply_style(ContentStyle {
                        foreground_color: Some(Color::DarkCyan),
                        ..Default::default()
                    })
                } else {
                    line
                })
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            // Keep the candidate pane addressable even when it has no candidates.
            lines.push(StyledGraphemes::from(" "));
        }
        CreatedGraphemes {
            graphemes: StyledGraphemes::from_lines(lines),
            layout: WidgetLayout {
                height_policy: HeightPolicy::FairContent,
                ..Default::default()
            },
            cursor: self.selected.map(|selected| ContentPosition {
                row: selected,
                column: 0,
            }),
        }
    }

    pub(in crate::tui) fn move_query_cursor_to(&mut self, position: ContentPosition) {
        let Some(text_editor::TextEditorHit::Cursor { index }) = self.query.hit_at(position) else {
            return;
        };
        self.query.texteditor.move_to(index);
    }

    pub(in crate::tui) fn select_at(&mut self, position: ContentPosition) -> bool {
        if position.row >= self.suggestions.len() {
            return false;
        }
        self.selected = Some(position.row);
        true
    }

    pub(in crate::tui) fn handle_event(&mut self, event: &Event) {
        match event {
            Event::Key(KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }) => {
                self.navigate(NavigationDirection::Previous);
                return;
            }
            Event::Key(KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }) => {
                self.navigate(NavigationDirection::Next);
                return;
            }
            _ => {}
        }

        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
            && key.state == KeyEventState::NONE
            && self.edit_query(key)
        {
            self.update_result();
        }
    }

    pub(in crate::tui) fn navigate(&mut self, direction: NavigationDirection) -> bool {
        match direction {
            NavigationDirection::Previous => self.move_backward(),
            NavigationDirection::Next => self.move_forward(),
        }
    }

    fn selected_item(&self) -> Option<&SelectItem<T>> {
        self.selected
            .and_then(|selected| self.suggestions.get(selected))
            .and_then(|item| self.items.get(*item))
    }

    fn query_text(&self) -> String {
        self.query.texteditor.text_without_cursor().to_string()
    }

    fn rebuild(&mut self, selected_key: Option<&str>) {
        self.update_result();

        let selected_position = selected_key
            .and_then(|key| self.items.iter().position(|item| item.key == key))
            .and_then(|item| {
                self.suggestions
                    .iter()
                    .position(|candidate| *candidate == item)
            });
        if let Some(position) = selected_position {
            self.selected = Some(position);
        }
    }

    fn update_result(&mut self) {
        let query = self.query_text();
        self.suggestions = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.label.starts_with(&query))
            .map(|(index, _)| index)
            .collect();
        self.selected = (!self.suggestions.is_empty()).then_some(0);
    }

    fn move_backward(&mut self) -> bool {
        if let Some(selected) = self.selected.filter(|selected| *selected > 0) {
            self.selected = Some(selected - 1);
            true
        } else {
            false
        }
    }

    fn move_forward(&mut self) -> bool {
        if let Some(selected) = self
            .selected
            .filter(|selected| selected.saturating_add(1) < self.suggestions.len())
        {
            self.selected = Some(selected + 1);
            true
        } else {
            false
        }
    }

    fn edit_query(&mut self, key: &KeyEvent) -> bool {
        match (key.code, key.modifiers) {
            (KeyCode::Left, KeyModifiers::NONE) => {
                self.query.texteditor.backward();
                false
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                self.query.texteditor.forward();
                false
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.query.texteditor.move_to_head();
                false
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.query.texteditor.move_to_tail();
                false
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                self.query.texteditor.erase();
                true
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.query.texteditor.erase_all();
                true
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                self.query
                    .texteditor
                    .erase_to_previous_nearest(&self.query.config.word_break_chars);
                true
            }
            (KeyCode::Char(character), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.query.texteditor.insert(character);
                true
            }
            _ => false,
        }
    }
}

impl Select<String> {
    pub(in crate::tui) fn value(&self) -> Option<String> {
        self.selected().cloned().or_else(|| self.custom_value())
    }

    pub(in crate::tui) fn custom_value(&self) -> Option<String> {
        if !self.suggestions.is_empty() {
            return None;
        }
        let query = self.query_text();
        (!query.is_empty()).then_some(query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn searches_items_by_prefix() {
        let mut select = Select::new();
        select.replace_items(vec![
            SelectItem::new("one", "first", 1),
            SelectItem::new("two", "second", 2),
        ]);

        for character in "sec".chars() {
            select.handle_event(&key(KeyCode::Char(character)));
        }

        assert_eq!(select.selected(), Some(&2));
    }

    #[test]
    fn preserves_the_defined_item_order() {
        let mut select = Select::new();
        select.replace_items(vec![
            SelectItem::new("pod", "pod", 1),
            SelectItem::new("deployment", "deployment", 2),
            SelectItem::new("daemonset", "daemonset", 3),
        ]);

        assert_eq!(select.selected(), Some(&1));
        select.handle_event(&key(KeyCode::Down));
        assert_eq!(select.selected(), Some(&2));
    }

    #[test]
    fn uses_the_query_when_there_are_no_candidates() {
        let mut select = Select::new();
        select.replace_items(vec![SelectItem::new("known", "known", "known".to_owned())]);

        for character in "custom".chars() {
            select.handle_event(&key(KeyCode::Char(character)));
        }

        assert_eq!(select.selected(), None);
        assert_eq!(select.value().as_deref(), Some("custom"));
    }

    #[test]
    fn does_not_create_a_value_from_an_empty_query() {
        let select = Select::<String>::new();

        assert_eq!(select.value(), None);
    }

    #[test]
    fn keeps_an_empty_candidate_pane_in_the_layout() {
        let select = Select::<String>::new();

        let candidates = select.list_graphemes();

        assert_eq!(candidates.layout.height_policy, HeightPolicy::FairContent);
        assert_eq!(candidates.graphemes.logical_lines().len(), 1);
        assert_eq!(candidates.graphemes.to_string(), " ");
    }

    #[test]
    fn keeps_selection_when_items_are_refreshed() {
        let mut select = Select::new();
        select.replace_items(vec![
            SelectItem::new("one", "first", 1),
            SelectItem::new("two", "second", 2),
        ]);
        select.handle_event(&key(KeyCode::Down));

        select.replace_items(vec![
            SelectItem::new("two", "second", 2),
            SelectItem::new("one", "first", 1),
        ]);

        assert_eq!(select.selected(), Some(&2));
    }

    #[test]
    fn selects_a_clicked_suggestion() {
        let mut select = Select::new();
        select.replace_items(vec![
            SelectItem::new("one", "first", 1),
            SelectItem::new("two", "second", 2),
        ]);

        assert!(select.select_at(ContentPosition { row: 1, column: 20 }));

        assert_eq!(select.selected(), Some(&2));
    }
}
