use egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    CommandPalette,
    RunQuery,
    RunStatement,
    FormatSql,
    SaveTab,
    OpenFile,
    Find,
    Replace,
    FindNext,
    FindPrev,
    Nl2Sql,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ShortcutsHelp,
}

impl Action {
    pub fn all() -> &'static [Action] {
        &[
            Action::CommandPalette,
            Action::RunQuery,
            Action::RunStatement,
            Action::FormatSql,
            Action::SaveTab,
            Action::OpenFile,
            Action::Find,
            Action::Replace,
            Action::FindNext,
            Action::FindPrev,
            Action::Nl2Sql,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ZoomReset,
            Action::ShortcutsHelp,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::CommandPalette => "Command Palette",
            Action::RunQuery => "Run Query / Selection",
            Action::RunStatement => "Run Statement at Cursor",
            Action::FormatSql => "Format SQL",
            Action::SaveTab => "Save Tab to File",
            Action::OpenFile => "Open SQL File",
            Action::Find => "Find in Editor",
            Action::Replace => "Find & Replace",
            Action::FindNext => "Find Next Match",
            Action::FindPrev => "Find Previous Match",
            Action::Nl2Sql => "Inline NL2SQL Prompt",
            Action::ZoomIn => "Increase Editor Font Size",
            Action::ZoomOut => "Decrease Editor Font Size",
            Action::ZoomReset => "Reset Editor Font Size",
            Action::ShortcutsHelp => "Keyboard Shortcuts Help",
        }
    }

    pub fn category(self) -> &'static str {
        match self {
            Action::CommandPalette | Action::ShortcutsHelp => "General",
            Action::RunQuery | Action::RunStatement | Action::FormatSql => "Execution",
            Action::SaveTab | Action::OpenFile => "Files",
            Action::Find
            | Action::Replace
            | Action::FindNext
            | Action::FindPrev
            | Action::Nl2Sql => "Editor",
            Action::ZoomIn | Action::ZoomOut | Action::ZoomReset => "View",
        }
    }

    pub fn default_shortcut(self) -> Shortcut {
        match self {
            Action::CommandPalette => Shortcut::new(Modifiers::CTRL, Key::K),
            Action::RunQuery => Shortcut::new(Modifiers::CTRL, Key::Enter),
            Action::RunStatement => Shortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::Enter),
            Action::FormatSql => Shortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::F),
            Action::SaveTab => Shortcut::new(Modifiers::CTRL, Key::S),
            Action::OpenFile => Shortcut::new(Modifiers::CTRL, Key::O),
            Action::Find => Shortcut::new(Modifiers::CTRL, Key::F),
            Action::Replace => Shortcut::new(Modifiers::CTRL, Key::H),
            Action::FindNext => Shortcut::new(Modifiers::NONE, Key::F3),
            Action::FindPrev => Shortcut::new(Modifiers::SHIFT, Key::F3),
            Action::Nl2Sql => Shortcut::new(Modifiers::CTRL, Key::I),
            Action::ZoomIn => Shortcut::new(Modifiers::CTRL, Key::Equals),
            Action::ZoomOut => Shortcut::new(Modifiers::CTRL, Key::Minus),
            Action::ZoomReset => Shortcut::new(Modifiers::CTRL, Key::Num0),
            Action::ShortcutsHelp => Shortcut::new(Modifiers::NONE, Key::F1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Shortcut {
    pub const fn new(modifiers: Modifiers, key: Key) -> Self {
        Self { modifiers, key }
    }

    pub fn format(self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.ctrl || self.modifiers.command {
            parts.push("Ctrl");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        parts.push(key_to_str(self.key));
        parts.join("+")
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut mods = Modifiers::NONE;
        let mut key = None;
        for raw in s.split('+') {
            let part = raw.trim();
            if part.is_empty() {
                continue;
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" | "option" => mods.alt = true,
                other => key = str_to_key(other),
            }
        }
        key.map(|k| Self::new(mods, k))
    }

    pub fn matches_modifiers(self, actual: Modifiers) -> bool {
        let want_ctrl = self.modifiers.ctrl || self.modifiers.command;
        let has_ctrl = actual.ctrl || actual.command;
        want_ctrl == has_ctrl
            && self.modifiers.shift == actual.shift
            && self.modifiers.alt == actual.alt
    }
}

fn key_to_str(key: Key) -> &'static str {
    match key {
        Key::Enter => "Enter",
        Key::Escape => "Esc",
        Key::Tab => "Tab",
        Key::Space => "Space",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::Minus => "-",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        _ => "?",
    }
}

fn str_to_key(s: &str) -> Option<Key> {
    match s {
        "enter" | "return" => Some(Key::Enter),
        "esc" | "escape" => Some(Key::Escape),
        "tab" => Some(Key::Tab),
        "space" => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "up" | "arrowup" => Some(Key::ArrowUp),
        "down" | "arrowdown" => Some(Key::ArrowDown),
        "left" | "arrowleft" => Some(Key::ArrowLeft),
        "right" | "arrowright" => Some(Key::ArrowRight),
        "=" | "equals" => Some(Key::Equals),
        "+" | "plus" => Some(Key::Plus),
        "-" | "minus" => Some(Key::Minus),
        "0" => Some(Key::Num0),
        "1" => Some(Key::Num1),
        "2" => Some(Key::Num2),
        "3" => Some(Key::Num3),
        "4" => Some(Key::Num4),
        "5" => Some(Key::Num5),
        "6" => Some(Key::Num6),
        "7" => Some(Key::Num7),
        "8" => Some(Key::Num8),
        "9" => Some(Key::Num9),
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),
        "a" => Some(Key::A),
        "b" => Some(Key::B),
        "c" => Some(Key::C),
        "d" => Some(Key::D),
        "e" => Some(Key::E),
        "f" => Some(Key::F),
        "g" => Some(Key::G),
        "h" => Some(Key::H),
        "i" => Some(Key::I),
        "j" => Some(Key::J),
        "k" => Some(Key::K),
        "l" => Some(Key::L),
        "m" => Some(Key::M),
        "n" => Some(Key::N),
        "o" => Some(Key::O),
        "p" => Some(Key::P),
        "q" => Some(Key::Q),
        "r" => Some(Key::R),
        "s" => Some(Key::S),
        "t" => Some(Key::T),
        "u" => Some(Key::U),
        "v" => Some(Key::V),
        "w" => Some(Key::W),
        "x" => Some(Key::X),
        "y" => Some(Key::Y),
        "z" => Some(Key::Z),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<Action, Shortcut>,
}

impl Default for Keymap {
    fn default() -> Self {
        let mut bindings = HashMap::new();
        for &action in Action::all() {
            bindings.insert(action, action.default_shortcut());
        }
        Self { bindings }
    }
}

impl Keymap {
    pub fn from_overrides(overrides: &HashMap<Action, String>) -> Self {
        let mut map = Self::default();
        for (action, raw) in overrides {
            if let Some(sc) = Shortcut::parse(raw) {
                map.bindings.insert(*action, sc);
            }
        }
        map
    }

    pub fn get(&self, action: Action) -> Shortcut {
        self.bindings
            .get(&action)
            .copied()
            .unwrap_or_else(|| action.default_shortcut())
    }

    pub fn label(&self, action: Action) -> String {
        self.get(action).format()
    }

    pub fn set(&mut self, action: Action, shortcut: Shortcut) {
        self.bindings.insert(action, shortcut);
    }

    pub fn reset_to_defaults(&mut self) {
        *self = Self::default();
    }

    pub fn find_conflict(&self, candidate: Shortcut, excluding: Action) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(act, sc)| **act != excluding && **sc == candidate)
            .map(|(act, _)| *act)
    }

    pub fn to_overrides(&self) -> HashMap<Action, String> {
        let mut out = HashMap::new();
        for &action in Action::all() {
            let current = self.get(action);
            if current != action.default_shortcut() {
                out.insert(action, current.format());
            }
        }
        out
    }

    pub fn consume(&self, i: &mut egui::InputState, action: Action) -> bool {
        let sc = self.get(action);
        let matched = i.events.iter().any(|ev| {
            matches!(
                ev,
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if *key == sc.key && sc.matches_modifiers(*modifiers)
            )
        });
        if matched {
            i.events.retain(|ev| {
                !matches!(
                    ev,
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if *key == sc.key && sc.matches_modifiers(*modifiers)
                )
            });
            true
        } else {
            false
        }
    }
}

pub fn capture_shortcut_from_input(i: &egui::InputState) -> Option<Shortcut> {
    for ev in &i.events {
        if let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = ev
        {
            let mut mods = Modifiers::NONE;
            mods.ctrl = modifiers.ctrl || modifiers.command;
            mods.shift = modifiers.shift;
            mods.alt = modifiers.alt;
            return Some(Shortcut::new(mods, *key));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shortcut_parse_and_format_roundtrip() {
        for &action in Action::all() {
            let sc = action.default_shortcut();
            let formatted = sc.format();
            let parsed = Shortcut::parse(&formatted).expect("should parse default shortcut");
            assert_eq!(parsed, sc);
        }
    }

    #[test]
    fn test_keymap_overrides_and_conflict_detection() {
        let mut overrides = HashMap::new();
        overrides.insert(Action::FormatSql, "Ctrl+Alt+L".to_string());
        let mut km = Keymap::from_overrides(&overrides);
        assert_eq!(km.label(Action::FormatSql), "Ctrl+Alt+L");
        assert_eq!(
            km.find_conflict(Shortcut::new(Modifiers::CTRL, Key::K), Action::FormatSql),
            Some(Action::CommandPalette)
        );
        assert_eq!(
            km.find_conflict(
                Shortcut::new(Modifiers::CTRL, Key::K),
                Action::CommandPalette
            ),
            None
        );
        let saved = km.to_overrides();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved.get(&Action::FormatSql).map(String::as_str),
            Some("Ctrl+Alt+L")
        );
        km.reset_to_defaults();
        assert!(km.to_overrides().is_empty());
    }
}
