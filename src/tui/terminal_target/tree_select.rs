use promkit::{
    core::{
        ContentPosition, CreatedGraphemes, Widget, WidgetLayout, WidthMode,
        crossterm::{
            event::{
                Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseEvent,
                MouseEventKind,
            },
            style::{Attribute, Attributes, Color, ContentStyle},
        },
        grapheme::{StyledGrapheme, StyledGraphemes},
    },
    widgets::text_editor,
};

use crate::tui::NavigationDirection;

pub(super) struct TreeSelectItem {
    pub(super) key: String,
    pub(super) application: String,
    pub(super) window_index: usize,
    pub(super) tab_index: usize,
    pub(super) pane_index: usize,
    pub(super) name: String,
    pub(super) working_directory: String,
    pub(super) id: String,
}

struct TreeSelectRow {
    row: TreeRow,
    selectable: bool,
    matched: bool,
    contains_match: bool,
    item_index: Option<usize>,
    root_index: usize,
    ancestor_last: Vec<bool>,
    is_last: bool,
}

struct TreeRow {
    depth: usize,
    label: String,
}

pub(super) struct TreeSelect {
    query: text_editor::State,
    items: Vec<TreeSelectItem>,
    rows: Vec<TreeSelectRow>,
    candidates: Vec<usize>,
    selected: Option<usize>,
    clauses: Vec<QueryClause>,
    query_error: Option<String>,
    visible_lines: usize,
    viewport_start: usize,
    viewport_was_scrolled: bool,
}

impl TreeSelect {
    pub(super) fn new(visible_lines: usize) -> Self {
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
            rows: Vec::new(),
            candidates: Vec::new(),
            selected: None,
            clauses: Vec::new(),
            query_error: None,
            visible_lines,
            viewport_start: 0,
            viewport_was_scrolled: false,
        }
    }

    pub(super) fn replace_items(&mut self, items: Vec<TreeSelectItem>) {
        let selected_key = self.selected_key().map(str::to_owned);
        self.items = items;
        self.rebuild(selected_key.as_deref());
    }

    pub(super) fn selected_key(&self) -> Option<&str> {
        self.selected_item().map(|item| item.key.as_str())
    }

    pub(super) fn query_graphemes(&self) -> CreatedGraphemes {
        self.query.create_graphemes()
    }

    pub(super) fn list_graphemes(&self) -> CreatedGraphemes {
        let selected_item = self.selected_item_index();
        let row_capacity = self.visible_row_capacity();
        let visible_rows = self
            .rows
            .iter()
            .skip(self.viewport_start)
            .take(row_capacity)
            .collect::<Vec<_>>();
        let mut lines = self
            .rows
            .iter()
            .skip(self.viewport_start)
            .take(row_capacity)
            .map(|row| {
                self.render_row(
                    row,
                    selected_item.is_some_and(|item| row.item_index == Some(item)),
                )
            })
            .collect::<Vec<_>>();

        if self.has_query() {
            lines.push(self.render_query_summary());
        }

        let cursor = selected_item.and_then(|item_index| {
            visible_rows
                .iter()
                .position(|row| row.item_index == Some(item_index))
                .map(|row| ContentPosition { row, column: 0 })
        });

        CreatedGraphemes {
            graphemes: StyledGraphemes::from_lines(lines),
            layout: WidgetLayout {
                max_height: Some(self.visible_lines),
                width_mode: WidthMode::Truncate,
                ..Default::default()
            },
            cursor,
        }
    }

    pub(super) fn move_query_cursor_to(&mut self, position: ContentPosition) {
        let Some(text_editor::TextEditorHit::Cursor { index }) = self.query.hit_at(position) else {
            return;
        };
        self.query.texteditor.move_to(index);
    }

    pub(super) fn select_at(&mut self, position: ContentPosition) -> bool {
        if position.row >= self.visible_row_count() {
            return false;
        }
        let Some(row) = self.rows.get(self.viewport_start + position.row) else {
            return false;
        };
        if !row.selectable {
            return false;
        }
        let Some(item_index) = row.item_index else {
            return false;
        };
        let Some(candidate) = self
            .candidates
            .iter()
            .position(|candidate| *candidate == item_index)
        else {
            return false;
        };
        self.selected = Some(candidate);
        true
    }

    pub(super) fn handle_event(&mut self, event: &Event) {
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
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                modifiers: KeyModifiers::NONE,
                ..
            }) => {
                self.scroll_backward();
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
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                modifiers: KeyModifiers::NONE,
                ..
            }) => {
                self.scroll_forward();
                return;
            }
            _ => {}
        }

        if let Event::Key(key) = event
            && key.kind == KeyEventKind::Press
            && key.state == KeyEventState::NONE
            && self.edit_query(key)
        {
            self.rebuild(None);
            self.ensure_selected_visible();
        }
    }

    pub(super) fn navigate(&mut self, direction: NavigationDirection) -> bool {
        let moved = match direction {
            NavigationDirection::Previous => self.move_backward(),
            NavigationDirection::Next => self.move_forward(),
        };
        if moved {
            self.follow_selected_after_cursor_move();
        }
        moved
    }

    fn selected_item(&self) -> Option<&TreeSelectItem> {
        self.selected_item_index()
            .and_then(|item| self.items.get(item))
    }

    fn selected_item_index(&self) -> Option<usize> {
        self.selected
            .and_then(|selected| self.candidates.get(selected))
            .copied()
    }

    fn query_text(&self) -> String {
        self.query.texteditor.text_without_cursor().to_string()
    }

    fn has_query(&self) -> bool {
        !self.query_text().trim().is_empty()
    }

    fn rebuild(&mut self, selected_key: Option<&str>) {
        match parse_query(&self.query_text()) {
            Ok(clauses) => {
                self.query_error = None;
                self.clauses = clauses;
                self.candidates = self
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.matches(&self.clauses))
                    .map(|(index, _)| index)
                    .collect();
            }
            Err(error) => {
                self.query_error = Some(error);
                self.clauses.clear();
                self.candidates.clear();
            }
        }

        let matches = (0..self.items.len())
            .map(|item| self.candidates.contains(&item))
            .collect::<Vec<_>>();
        self.rows = create_rows(&self.items, &matches);
        self.clamp_viewport();

        self.selected = selected_key
            .and_then(|key| self.items.iter().position(|item| item.key == key))
            .and_then(|item| {
                self.candidates
                    .iter()
                    .position(|candidate| *candidate == item)
            })
            .or_else(|| (!self.candidates.is_empty()).then_some(0));
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
            .filter(|selected| selected.saturating_add(1) < self.candidates.len())
        {
            self.selected = Some(selected + 1);
            true
        } else {
            false
        }
    }

    fn scroll_backward(&mut self) {
        let viewport_start = self.viewport_start.saturating_sub(1);
        self.viewport_was_scrolled |= viewport_start != self.viewport_start;
        self.viewport_start = viewport_start;
    }

    fn scroll_forward(&mut self) {
        let viewport_start = self
            .viewport_start
            .saturating_add(1)
            .min(self.maximum_viewport_start());
        self.viewport_was_scrolled |= viewport_start != self.viewport_start;
        self.viewport_start = viewport_start;
    }

    fn follow_selected_after_cursor_move(&mut self) {
        if self.viewport_was_scrolled {
            self.center_selected();
            self.viewport_was_scrolled = false;
        } else {
            self.ensure_selected_visible();
        }
    }

    fn center_selected(&mut self) {
        let Some(item_index) = self.selected_item_index() else {
            return;
        };
        let Some(row) = self
            .rows
            .iter()
            .position(|row| row.item_index == Some(item_index))
        else {
            return;
        };
        let capacity = self.visible_row_capacity();
        self.viewport_start = row
            .saturating_sub(capacity / 2)
            .min(self.maximum_viewport_start());
    }

    fn ensure_selected_visible(&mut self) {
        let Some(item_index) = self.selected_item_index() else {
            return;
        };
        let Some(row) = self
            .rows
            .iter()
            .position(|row| row.item_index == Some(item_index))
        else {
            return;
        };
        let capacity = self.visible_row_capacity();
        if row < self.viewport_start || row >= self.viewport_start.saturating_add(capacity) {
            self.viewport_start = self.selection_viewport_start(row, capacity);
        }
        self.clamp_viewport();
        self.viewport_was_scrolled = false;
    }

    fn visible_row_capacity(&self) -> usize {
        self.visible_lines
            .saturating_sub(usize::from(self.has_query()))
            .max(1)
    }

    fn visible_row_count(&self) -> usize {
        self.rows
            .len()
            .saturating_sub(self.viewport_start)
            .min(self.visible_row_capacity())
    }

    fn maximum_viewport_start(&self) -> usize {
        self.rows.len().saturating_sub(self.visible_row_capacity())
    }

    fn clamp_viewport(&mut self) {
        self.viewport_start = self.viewport_start.min(self.maximum_viewport_start());
    }

    fn selection_viewport_start(&self, selected_row: usize, capacity: usize) -> usize {
        let root = self.rows[selected_row].root_index;
        if selected_row.saturating_sub(root) < capacity {
            root
        } else {
            selected_row.saturating_add(1).saturating_sub(capacity)
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

    fn render_row(&self, row: &TreeSelectRow, active: bool) -> StyledGraphemes {
        let cursor = if active { "❯ " } else { "  " };
        let text = format!(
            "{cursor}{}{}",
            tree_prefix(&row.ancestor_last, row.row.depth, row.is_last),
            row.row.label
        );
        let related = row.matched || row.contains_match;
        let base_style = if active {
            ContentStyle {
                foreground_color: Some(Color::DarkCyan),
                attributes: Attributes::from(Attribute::Bold),
                ..Default::default()
            }
        } else if related {
            ContentStyle::default()
        } else {
            ContentStyle {
                attributes: Attributes::from(Attribute::Dim),
                ..Default::default()
            }
        };

        let terms = if related {
            self.clauses
                .iter()
                .map(QueryClause::display_value)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        styled_with_matches(&text, base_style, &terms)
    }

    fn render_query_summary(&self) -> StyledGraphemes {
        if let Some(error) = &self.query_error {
            return StyledGraphemes::from_str(
                format!("  {error}"),
                ContentStyle {
                    foreground_color: Some(Color::DarkRed),
                    ..Default::default()
                },
            );
        }

        let suffix = if self.candidates.len() == 1 {
            "match"
        } else {
            "matches"
        };
        StyledGraphemes::from_str(
            format!(
                "  {} {suffix} / {} terminals",
                self.candidates.len(),
                self.items.len()
            ),
            ContentStyle {
                attributes: Attributes::from(Attribute::Dim),
                ..Default::default()
            },
        )
    }
}

impl TreeSelectItem {
    fn matches(&self, clauses: &[QueryClause]) -> bool {
        clauses.iter().all(|clause| match clause {
            QueryClause::Text { field, value } => match field {
                None => [
                    self.application.as_str(),
                    self.name.as_str(),
                    self.working_directory.as_str(),
                    &self.location(),
                ]
                .iter()
                .any(|candidate| contains_ignoring_ascii_case(candidate, value)),
                Some(TextField::Application) => {
                    contains_ignoring_ascii_case(&self.application, value)
                }
                Some(TextField::Name) => contains_ignoring_ascii_case(&self.name, value),
                Some(TextField::WorkingDirectory) => {
                    contains_ignoring_ascii_case(&self.working_directory, value)
                }
                Some(TextField::Id) => contains_ignoring_ascii_case(&self.id, value),
            },
            QueryClause::Number { field, value } => match field {
                NumberField::Window => self.window_index == *value,
                NumberField::Tab => self.tab_index == *value,
                NumberField::Pane => self.pane_index == *value,
            },
        })
    }

    fn location(&self) -> String {
        format!(
            "W{}/T{}/P{}",
            self.window_index, self.tab_index, self.pane_index
        )
    }

    fn pane_label(&self) -> String {
        format!(
            "Pane {} · {} · {}",
            self.pane_index, self.name, self.working_directory
        )
    }
}

#[derive(Clone)]
enum QueryClause {
    Text {
        field: Option<TextField>,
        value: String,
    },
    Number {
        field: NumberField,
        value: usize,
    },
}

impl QueryClause {
    fn display_value(&self) -> String {
        match self {
            Self::Text { value, .. } => value.clone(),
            Self::Number { value, .. } => value.to_string(),
        }
    }
}

#[derive(Clone, Copy)]
enum TextField {
    Application,
    Name,
    WorkingDirectory,
    Id,
}

#[derive(Clone, Copy)]
enum NumberField {
    Window,
    Tab,
    Pane,
}

fn parse_query(query: &str) -> Result<Vec<QueryClause>, String> {
    tokenize(query)?
        .into_iter()
        .map(|token| parse_clause(&token))
        .collect()
}

fn parse_clause(token: &str) -> Result<QueryClause, String> {
    let Some((field, value)) = token.split_once(':') else {
        return Ok(QueryClause::Text {
            field: None,
            value: token.to_owned(),
        });
    };
    if !field
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Ok(QueryClause::Text {
            field: None,
            value: token.to_owned(),
        });
    }
    if value.is_empty() {
        return Err(format!("Search field \"{field}\" needs a value"));
    }

    match field.to_ascii_lowercase().as_str() {
        "app" => Ok(text_clause(TextField::Application, value)),
        "name" => Ok(text_clause(TextField::Name, value)),
        "cwd" | "path" => Ok(text_clause(TextField::WorkingDirectory, value)),
        "id" => Ok(text_clause(TextField::Id, value)),
        "window" | "w" => number_clause(NumberField::Window, "Window", value),
        "tab" | "t" => number_clause(NumberField::Tab, "Tab", value),
        "pane" | "p" => number_clause(NumberField::Pane, "Pane", value),
        _ => Err(format!(
            "Unknown search field \"{field}\"; use app, name, cwd, w, t, p, or id"
        )),
    }
}

fn text_clause(field: TextField, value: &str) -> QueryClause {
    QueryClause::Text {
        field: Some(field),
        value: value.to_owned(),
    }
}

fn number_clause(field: NumberField, label: &str, value: &str) -> Result<QueryClause, String> {
    let value = value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{label} index must be a positive integer"))?;
    Ok(QueryClause::Number { field, value })
}

fn tokenize(query: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut escaped = false;

    for character in query.chars() {
        if escaped {
            token.push(character);
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '"' => quoted = !quoted,
                character if character.is_whitespace() && !quoted => {
                    if !token.is_empty() {
                        tokens.push(std::mem::take(&mut token));
                    }
                }
                _ => token.push(character),
            }
        }
    }
    if escaped {
        token.push('\\');
    }
    if quoted {
        return Err("Unclosed quote in search query".to_owned());
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    Ok(tokens)
}

struct Node {
    key: String,
    label: String,
    item_index: Option<usize>,
    children: Vec<Node>,
    matched: bool,
    contains_match: bool,
}

impl Node {
    fn branch(key: String, label: String) -> Self {
        Self {
            key,
            label,
            item_index: None,
            children: Vec::new(),
            matched: false,
            contains_match: false,
        }
    }

    fn leaf(key: String, label: String, item_index: usize) -> Self {
        Self {
            key,
            label,
            item_index: Some(item_index),
            children: Vec::new(),
            matched: false,
            contains_match: false,
        }
    }

    fn mark_matches(&mut self, matches: &[bool]) -> bool {
        self.matched = self
            .item_index
            .and_then(|index| matches.get(index))
            .copied()
            .unwrap_or(false);
        let child_contains_match = self
            .children
            .iter_mut()
            .map(|child| child.mark_matches(matches))
            .fold(false, |contains_match, child_matches| {
                contains_match | child_matches
            });
        self.contains_match = self.matched || child_contains_match;
        self.contains_match
    }
}

fn create_rows(items: &[TreeSelectItem], matches: &[bool]) -> Vec<TreeSelectRow> {
    let mut roots = Vec::new();
    for (item_index, item) in items.iter().enumerate() {
        let path = [
            (
                format!("application:{}", item.application.to_ascii_lowercase()),
                item.application.clone(),
            ),
            (
                format!("window:{}", item.window_index),
                format!("Window {}", item.window_index),
            ),
            (
                format!("tab:{}", item.tab_index),
                format!("Tab {}", item.tab_index),
            ),
        ];
        insert_item(
            &mut roots,
            &path,
            Node::leaf(item.key.clone(), item.pane_label(), item_index),
        );
    }
    for root in &mut roots {
        root.mark_matches(matches);
    }

    let mut rows = Vec::new();
    flatten_nodes(&roots, &mut Vec::new(), &mut Vec::new(), None, &mut rows);
    rows
}

fn insert_item(nodes: &mut Vec<Node>, path: &[(String, String)], leaf: Node) {
    let Some(((key, label), rest)) = path.split_first() else {
        nodes.push(leaf);
        return;
    };
    let position = nodes
        .iter()
        .position(|node| node.key == *key)
        .unwrap_or_else(|| {
            nodes.push(Node::branch(key.clone(), label.clone()));
            nodes.len() - 1
        });
    insert_item(&mut nodes[position].children, rest, leaf);
}

fn flatten_nodes(
    nodes: &[Node],
    path: &mut Vec<String>,
    ancestor_last: &mut Vec<bool>,
    root_index: Option<usize>,
    rows: &mut Vec<TreeSelectRow>,
) {
    for (index, node) in nodes.iter().enumerate() {
        let is_last = index + 1 == nodes.len();
        let root_index = root_index.unwrap_or(rows.len());
        path.push(node.label.clone());
        rows.push(TreeSelectRow {
            row: TreeRow {
                depth: path.len() - 1,
                label: node.label.clone(),
            },
            selectable: node.item_index.is_some() && node.matched,
            matched: node.matched,
            contains_match: node.contains_match,
            item_index: node.item_index,
            root_index,
            ancestor_last: ancestor_last.clone(),
            is_last,
        });
        ancestor_last.push(is_last);
        flatten_nodes(&node.children, path, ancestor_last, Some(root_index), rows);
        ancestor_last.pop();
        path.pop();
    }
}

fn tree_prefix(ancestor_last: &[bool], depth: usize, is_last: bool) -> String {
    if depth == 0 {
        return String::new();
    }
    let mut prefix = ancestor_last
        .iter()
        .skip(1)
        .map(|is_last| if *is_last { "   " } else { "│  " })
        .collect::<String>();
    prefix.push_str(if is_last { "└─ " } else { "├─ " });
    prefix
}

fn contains_ignoring_ascii_case(candidate: &str, query: &str) -> bool {
    candidate
        .to_ascii_lowercase()
        .contains(&query.to_ascii_lowercase())
}

fn styled_with_matches(text: &str, base_style: ContentStyle, terms: &[String]) -> StyledGraphemes {
    let characters = text.chars().collect::<Vec<_>>();
    let mut highlighted = vec![false; characters.len()];
    for term in terms.iter().filter(|term| !term.is_empty()) {
        let query = term.chars().collect::<Vec<_>>();
        if query.len() > characters.len() {
            continue;
        }
        for start in 0..=characters.len() - query.len() {
            if characters[start..start + query.len()]
                .iter()
                .zip(&query)
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
            {
                highlighted[start..start + query.len()].fill(true);
            }
        }
    }

    let highlight_style = ContentStyle {
        foreground_color: Some(Color::DarkYellow),
        attributes: Attributes::from(Attribute::Bold),
        ..Default::default()
    };
    characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| {
            StyledGrapheme::new(
                character,
                if highlighted[index] {
                    highlight_style
                } else {
                    base_style
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        key: &'static str,
        application: &str,
        window: usize,
        tab: usize,
        pane: usize,
        name: &str,
        cwd: &str,
    ) -> TreeSelectItem {
        TreeSelectItem {
            key: key.into(),
            application: application.into(),
            window_index: window,
            tab_index: tab,
            pane_index: pane,
            name: name.into(),
            working_directory: cwd.into(),
            id: key.into(),
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn scroll(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn type_query(select: &mut TreeSelect, query: &str) {
        for character in query.chars() {
            select.handle_event(&key(KeyCode::Char(character)));
        }
    }

    fn items() -> Vec<TreeSelectItem> {
        vec![
            item("ghostty", "Ghostty", 1, 1, 1, "shell", "/workspace"),
            item("cargo", "iTerm2", 1, 1, 1, "cargo", "/workspace"),
            item("claude", "iTerm2", 1, 2, 2, "claude", "/src/convey"),
        ]
    }

    #[test]
    fn renders_the_terminal_hierarchy() {
        let mut select = TreeSelect::new(20);
        select.replace_items(items());

        let rendered = select.list_graphemes();

        assert_eq!(rendered.layout.width_mode, WidthMode::Truncate);
        assert_eq!(
            rendered.graphemes.to_string(),
            concat!(
                "  Ghostty\n",
                "  └─ Window 1\n",
                "     └─ Tab 1\n",
                "❯       └─ Pane 1 · shell · /workspace\n",
                "  iTerm2\n",
                "  └─ Window 1\n",
                "     ├─ Tab 1\n",
                "     │  └─ Pane 1 · cargo · /workspace\n",
                "     └─ Tab 2\n",
                "        └─ Pane 2 · claude · /src/convey",
            )
        );
    }

    #[test]
    fn searches_multiple_qualified_fields_with_and_semantics() {
        let mut select = TreeSelect::new(20);
        select.replace_items(items());

        type_query(&mut select, "app:iterm2 t:2 name:claude");

        assert_eq!(select.selected_key(), Some("claude"));
        assert_eq!(select.candidates, vec![2]);
        assert!(
            select
                .list_graphemes()
                .graphemes
                .to_string()
                .contains("1 match / 3 terminals")
        );
    }

    #[test]
    fn searches_unqualified_terms_across_fields_with_and_semantics() {
        let mut select = TreeSelect::new(20);
        select.replace_items(items());

        type_query(&mut select, "iterm2 convey");

        assert_eq!(select.selected_key(), Some("claude"));
        assert_eq!(select.candidates, vec![2]);
    }

    #[test]
    fn searches_numeric_indexes_by_exact_value() {
        let mut select = TreeSelect::new(20);
        select.replace_items(vec![
            item("one", "Ghostty", 1, 1, 1, "one", "/workspace"),
            item("ten", "Ghostty", 10, 1, 1, "ten", "/workspace"),
        ]);

        type_query(&mut select, "w:1");

        assert_eq!(select.selected_key(), Some("one"));
        assert_eq!(select.candidates, vec![0]);
    }

    #[test]
    fn moves_only_between_matching_panes() {
        let mut select = TreeSelect::new(20);
        select.replace_items(items());
        type_query(&mut select, "app:iterm2");

        assert_eq!(select.selected_key(), Some("cargo"));
        select.handle_event(&key(KeyCode::Down));
        assert_eq!(select.selected_key(), Some("claude"));
        select.handle_event(&key(KeyCode::Down));
        assert_eq!(select.selected_key(), Some("claude"));
        select.handle_event(&key(KeyCode::Up));
        assert_eq!(select.selected_key(), Some("cargo"));
    }

    #[test]
    fn cursor_move_recenters_a_manually_scrolled_viewport() {
        let mut select = TreeSelect::new(4);
        select.replace_items(items());

        for _ in 0..4 {
            select.handle_event(&scroll(MouseEventKind::ScrollDown));
        }

        assert_eq!(select.viewport_start, 4);
        assert_eq!(select.selected_key(), Some("ghostty"));

        select.handle_event(&key(KeyCode::Down));

        assert_eq!(select.viewport_start, 5);
        assert_eq!(select.selected_key(), Some("cargo"));
        assert_eq!(
            select.list_graphemes().cursor,
            Some(ContentPosition { row: 2, column: 0 })
        );

        select.handle_event(&key(KeyCode::Down));

        assert_eq!(select.viewport_start, 6);
        assert_eq!(select.selected_key(), Some("claude"));
    }

    #[test]
    fn selects_a_pane_at_its_viewport_relative_row() {
        let mut select = TreeSelect::new(4);
        select.replace_items(items());
        for _ in 0..4 {
            select.handle_event(&scroll(MouseEventKind::ScrollDown));
        }

        assert!(select.select_at(ContentPosition { row: 3, column: 0 }));

        assert_eq!(select.selected_key(), Some("cargo"));
    }

    #[test]
    fn searches_quoted_values() {
        let mut select = TreeSelect::new(20);
        select.replace_items(vec![item(
            "claude",
            "iTerm2",
            1,
            1,
            1,
            "Claude Code",
            "/src/convey",
        )]);

        type_query(&mut select, "name:\"Claude Code\"");

        assert_eq!(select.selected_key(), Some("claude"));
    }

    #[test]
    fn keeps_non_candidates_visible_but_unselectable() {
        let mut select = TreeSelect::new(20);
        select.replace_items(items());
        type_query(&mut select, "name:claude");

        assert_eq!(select.rows.len(), 10);
        let ghostty_pane = select
            .rows
            .iter()
            .position(|row| row.item_index == Some(0))
            .unwrap();
        let claude_pane = select
            .rows
            .iter()
            .position(|row| row.item_index == Some(2))
            .unwrap();

        assert!(!select.select_at(ContentPosition {
            row: ghostty_pane,
            column: 0,
        }));
        assert!(select.select_at(ContentPosition {
            row: claude_pane,
            column: 0,
        }));
    }

    #[test]
    fn does_not_render_selection_cursors_when_nothing_matches() {
        let mut select = TreeSelect::new(20);
        select.replace_items(items());

        type_query(&mut select, "no-such-terminal");

        let rendered = select.list_graphemes();
        assert_eq!(select.selected_key(), None);
        assert_eq!(rendered.cursor, None);
        assert!(!rendered.graphemes.to_string().contains('❯'));
        assert!(
            rendered
                .graphemes
                .to_string()
                .contains("0 matches / 3 terminals")
        );
    }

    #[test]
    fn reports_invalid_search_fields() {
        let mut select = TreeSelect::new(20);
        select.replace_items(items());

        type_query(&mut select, "windw:1");

        assert_eq!(select.selected_key(), None);
        assert!(
            select
                .list_graphemes()
                .graphemes
                .to_string()
                .contains("Unknown search field \"windw\"")
        );
    }
}
