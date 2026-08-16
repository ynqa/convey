use promkit::{
    core::{
        ContentPosition, CreatedGraphemes, HeightPolicy, Widget,
        crossterm::{
            event::{Event, KeyCode, KeyEventKind, KeyEventState, KeyModifiers},
            style::{Color, ContentStyle},
        },
    },
    widgets::text_editor,
};

use crate::tui::NavigationDirection;

pub(super) struct Textarea {
    editor: text_editor::State,
}

impl Textarea {
    pub(super) fn new() -> Self {
        Self {
            editor: text_editor::State {
                config: text_editor::Config {
                    prefix: "│ ".into(),
                    continuation_prefix: "│ ".into(),
                    prefix_style: ContentStyle {
                        foreground_color: Some(Color::DarkGreen),
                        ..Default::default()
                    },
                    active_char_style: ContentStyle {
                        background_color: Some(Color::DarkCyan),
                        ..Default::default()
                    },
                    lines: None,
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    pub(super) fn value(&self) -> String {
        self.editor.texteditor.text_without_cursor().to_string()
    }

    pub(super) fn graphemes(&self) -> CreatedGraphemes {
        let mut created = self.editor.create_graphemes();
        created.layout.height_policy = HeightPolicy::FairContent;
        created
    }

    pub(super) fn move_cursor_to(&mut self, position: ContentPosition) {
        let Some(text_editor::TextEditorHit::Cursor { index }) = self.editor.hit_at(position)
        else {
            return;
        };
        self.editor.texteditor.move_to(index);
    }

    pub(super) fn navigate(&mut self, direction: NavigationDirection) -> bool {
        match direction {
            NavigationDirection::Previous => self.editor.texteditor.move_up(),
            NavigationDirection::Next => self.editor.texteditor.move_down(),
        }
    }

    pub(super) fn handle_event(&mut self, event: &Event) {
        let Event::Key(key) = event else {
            return;
        };
        if key.kind != KeyEventKind::Press || key.state != KeyEventState::NONE {
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Enter, KeyModifiers::NONE) => self.editor.texteditor.insert_newline(),
            (KeyCode::Left, KeyModifiers::NONE) => {
                self.editor.texteditor.backward();
            }
            (KeyCode::Right, KeyModifiers::NONE) => {
                self.editor.texteditor.forward();
            }
            (KeyCode::Up, KeyModifiers::NONE) => {
                self.editor.texteditor.move_up();
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                self.editor.texteditor.move_down();
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                self.editor.texteditor.move_to_line_head();
            }
            (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
                self.editor.texteditor.move_to_line_tail();
            }
            (KeyCode::Char('b'), KeyModifiers::ALT) => {
                let word_break_chars = self.editor.config.word_break_chars.clone();
                self.editor
                    .texteditor
                    .move_to_previous_nearest(&word_break_chars);
            }
            (KeyCode::Char('f'), KeyModifiers::ALT) => {
                let word_break_chars = self.editor.config.word_break_chars.clone();
                self.editor
                    .texteditor
                    .move_to_next_nearest(&word_break_chars);
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => self.editor.texteditor.erase(),
            (KeyCode::Delete, KeyModifiers::NONE) => self.editor.texteditor.erase_forward(),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.editor.texteditor.erase_all();
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                let word_break_chars = self.editor.config.word_break_chars.clone();
                self.editor
                    .texteditor
                    .erase_to_previous_nearest(&word_break_chars);
            }
            (KeyCode::Char(character), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                self.editor.texteditor.insert(character);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use promkit::core::crossterm::event::KeyEvent;

    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn edits_multiple_lines() {
        let mut textarea = Textarea::new();
        textarea.handle_event(&key(KeyCode::Char('a')));
        textarea.handle_event(&key(KeyCode::Enter));
        textarea.handle_event(&key(KeyCode::Char('b')));

        assert_eq!(textarea.value(), "a\nb");
    }

    #[test]
    fn moves_the_cursor_to_a_clicked_position() {
        let mut textarea = Textarea::new();
        textarea.handle_event(&key(KeyCode::Char('a')));
        textarea.handle_event(&key(KeyCode::Char('b')));

        textarea.move_cursor_to(ContentPosition { row: 0, column: 3 });
        textarea.handle_event(&key(KeyCode::Char('x')));

        assert_eq!(textarea.value(), "axb");
    }
}
