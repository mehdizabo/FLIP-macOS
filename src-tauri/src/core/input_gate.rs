use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ShortcutError {
    #[error("Choose a valid single drag trigger key (Down, Up, Left, Right, Space, Tab, Fn, F1–F12, a letter/digit, or modifier). Escape and Return are reserved.")]
    InvalidDragKey,
    #[error("Idle shortcut must include a valid key and at least one modifier (or F1–F12): {0}")]
    InvalidIdleShortcut(String),
}

/// Pairs key-down, auto-repeat, and key-up so a key pressed during normal typing
/// isn't hijacked mid-hold, while a drag-activated trigger consumes its own repeats & key-up.
#[derive(Debug, Default, Clone)]
pub struct ShortcutPressGate {
    down: bool,
    captured: bool,
    reserve_repeats: bool,
}

impl ShortcutPressGate {
    pub fn is_down(&self) -> bool {
        self.down
    }

    pub fn reset(&mut self) {
        self.down = false;
        self.captured = false;
        self.reserve_repeats = false;
    }

    pub fn reserve_repeats_for_gesture(&mut self) {
        if self.down {
            self.reserve_repeats = true;
        }
    }

    /// Returns `(consume, activate)`
    pub fn process(&mut self, key_down: bool, eligible: bool) -> (bool, bool) {
        if key_down {
            if self.down {
                return (self.captured || self.reserve_repeats, false);
            }
            self.down = true;
            self.captured = eligible;
            return (self.captured, self.captured);
        }
        let consume = self.captured;
        self.down = false;
        self.captured = false;
        self.reserve_repeats = false;
        (consume, false)
    }
}

/// Supports both gesture orderings:
/// 1) drag-first -> key-down opens immediately
/// 2) key-first -> mouse-down + >=8px drag opens once
#[derive(Debug, Default, Clone)]
pub struct ShortcutOrderGate {
    held: bool,
    mouse_down: bool,
    opened: bool,
    origin_x: i32,
    origin_y: i32,
}

impl ShortcutOrderGate {
    pub fn held(&self) -> bool {
        self.held
    }

    pub fn key_down(&mut self, primary_held: bool) -> bool {
        if self.held {
            return false;
        }
        self.held = true;
        if !primary_held {
            self.mouse_down = false;
            self.opened = false;
            return false;
        }
        self.opened = true;
        true
    }

    pub fn key_up(&mut self) {
        self.held = false;
        self.mouse_down = false;
    }

    pub fn mouse_down(&mut self, x: i32, y: i32) {
        self.mouse_down = true;
        self.origin_x = x;
        self.origin_y = y;
    }

    pub fn mouse_up(&mut self) {
        self.mouse_down = false;
        self.opened = false;
    }

    pub fn move_cursor(&mut self, x: i32, y: i32) -> bool {
        if !self.held
            || !self.mouse_down
            || self.opened
            || ((x as i64 - self.origin_x as i64).abs() < 8
                && (y as i64 - self.origin_y as i64).abs() < 8)
        {
            return false;
        }
        self.opened = true;
        true
    }

    pub fn reset(&mut self) {
        self.held = false;
        self.mouse_down = false;
        self.opened = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DragTriggerShortcut {
    pub id: String,
    pub display_name: String,
    pub key_code: Option<u16>,
    pub modifier_only: Option<String>,
}

impl DragTriggerShortcut {
    pub const DEFAULT: &'static str = "Down";

    pub fn parse(raw: Option<&str>) -> Result<Self, ShortcutError> {
        let Some(raw_str) = raw else {
            return Err(ShortcutError::InvalidDragKey);
        };
        let norm = raw_str.trim().to_ascii_lowercase().replace(' ', "");
        if norm.is_empty() || norm.contains('+') {
            return Err(ShortcutError::InvalidDragKey);
        }

        match norm.as_str() {
            "down" | "downarrow" | "arrowdown" => Ok(Self {
                id: "Down".into(),
                display_name: "Down Arrow".into(),
                key_code: Some(125),
                modifier_only: None,
            }),
            "up" | "uparrow" | "arrowup" => Ok(Self {
                id: "Up".into(),
                display_name: "Up Arrow".into(),
                key_code: Some(126),
                modifier_only: None,
            }),
            "left" | "leftarrow" | "arrowleft" => Ok(Self {
                id: "Left".into(),
                display_name: "Left Arrow".into(),
                key_code: Some(123),
                modifier_only: None,
            }),
            "right" | "rightarrow" | "arrowright" => Ok(Self {
                id: "Right".into(),
                display_name: "Right Arrow".into(),
                key_code: Some(124),
                modifier_only: None,
            }),
            "space" | "spacebar" => Ok(Self {
                id: "Space".into(),
                display_name: "Space".into(),
                key_code: Some(49),
                modifier_only: None,
            }),
            "tab" => Ok(Self {
                id: "Tab".into(),
                display_name: "Tab".into(),
                key_code: Some(48),
                modifier_only: None,
            }),
            "fn" | "function" | "globe" => Ok(Self {
                id: "Fn".into(),
                display_name: "Fn / Globe".into(),
                key_code: Some(63),
                modifier_only: Some("fn".into()),
            }),
            "option" | "alt" => Ok(Self {
                id: "Option".into(),
                display_name: "Option".into(),
                key_code: None,
                modifier_only: Some("option".into()),
            }),
            "control" | "ctrl" => Ok(Self {
                id: "Control".into(),
                display_name: "Control".into(),
                key_code: None,
                modifier_only: Some("control".into()),
            }),
            "shift" => Ok(Self {
                id: "Shift".into(),
                display_name: "Shift".into(),
                key_code: None,
                modifier_only: Some("shift".into()),
            }),
            "command" | "cmd" => Ok(Self {
                id: "Command".into(),
                display_name: "Command".into(),
                key_code: None,
                modifier_only: Some("command".into()),
            }),
            other => {
                if let Some(fnum_str) = other.strip_prefix('f') {
                    if let Ok(n) = fnum_str.parse::<u8>() {
                        if let Some(kc) = function_key_code(n) {
                            return Ok(Self {
                                id: format!("F{n}"),
                                display_name: format!("F{n}"),
                                key_code: Some(kc),
                                modifier_only: None,
                            });
                        }
                    }
                }
                if other.len() == 1 {
                    let ch = other.chars().next().unwrap();
                    if let Some(kc) = ascii_key_code(ch) {
                        let upper = ch.to_ascii_uppercase().to_string();
                        return Ok(Self {
                            id: upper.clone(),
                            display_name: upper,
                            key_code: Some(kc),
                            modifier_only: None,
                        });
                    }
                }
                Err(ShortcutError::InvalidDragKey)
            }
        }
    }

    pub fn matches_key(&self, key_code: u16, is_modifier_event: bool) -> bool {
        if is_modifier_event {
            return self.key_code == Some(key_code);
        }
        self.modifier_only.is_none() && self.key_code == Some(key_code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleShortcut {
    pub command: bool,
    pub option: bool,
    pub control: bool,
    pub shift: bool,
    pub key: String,
    pub key_code: u16,
}

impl IdleShortcut {
    pub const DEFAULT: &'static str = "Option+Shift+Space";

    pub fn parse(raw: &str) -> Result<Self, ShortcutError> {
        let normalized = raw
            .replace('⌘', "Command+")
            .replace('⌥', "Option+")
            .replace('⌃', "Control+")
            .replace('⇧', "Shift+");
        let parts: Vec<String> = normalized
            .split('+')
            .map(|p| p.trim().to_ascii_lowercase())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            return Err(ShortcutError::InvalidIdleShortcut(raw.to_string()));
        }

        let mut command = false;
        let mut option = false;
        let mut control = false;
        let mut shift = false;

        for mod_tok in &parts[..parts.len() - 1] {
            match mod_tok.as_str() {
                "cmd" | "command" | "super" | "meta" => command = true,
                "opt" | "option" | "alt" => option = true,
                "ctrl" | "control" => control = true,
                "shift" => shift = true,
                _ => return Err(ShortcutError::InvalidIdleShortcut(raw.to_string())),
            }
        }

        let key_tok = &parts[parts.len() - 1];
        let (canonical_key, key_code, is_fn) = match key_tok.as_str() {
            "space" | "spacebar" => ("Space".to_string(), 49u16, false),
            "tab" => ("Tab".to_string(), 48u16, false),
            "down" | "downarrow" => ("Down".to_string(), 125u16, false),
            "up" | "uparrow" => ("Up".to_string(), 126u16, false),
            "left" | "leftarrow" => ("Left".to_string(), 123u16, false),
            "right" | "rightarrow" => ("Right".to_string(), 124u16, false),
            other => {
                if let Some(fnum_str) = other.strip_prefix('f') {
                    if let Ok(n) = fnum_str.parse::<u8>() {
                        if let Some(kc) = function_key_code(n) {
                            (format!("F{n}"), kc, true)
                        } else {
                            return Err(ShortcutError::InvalidIdleShortcut(raw.to_string()));
                        }
                    } else {
                        return Err(ShortcutError::InvalidIdleShortcut(raw.to_string()));
                    }
                } else if other.len() == 1 {
                    let ch = other.chars().next().unwrap();
                    if let Some(kc) = ascii_key_code(ch) {
                        (ch.to_ascii_uppercase().to_string(), kc, false)
                    } else {
                        return Err(ShortcutError::InvalidIdleShortcut(raw.to_string()));
                    }
                } else {
                    return Err(ShortcutError::InvalidIdleShortcut(raw.to_string()));
                }
            }
        };

        if !command && !option && !control && !shift && !is_fn {
            return Err(ShortcutError::InvalidIdleShortcut(
                "non-function idle shortcut requires at least one modifier".to_string(),
            ));
        }

        Ok(Self {
            command,
            option,
            control,
            shift,
            key: canonical_key,
            key_code,
        })
    }

    pub fn canonical_id(&self) -> String {
        let mut parts = Vec::new();
        if self.command {
            parts.push("Command");
        }
        if self.option {
            parts.push("Option");
        }
        if self.control {
            parts.push("Control");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelPlacement {
    pub x: i32,
    pub y: i32,
    pub size: i32,
}

/// Port of `Native.Clamp` — keeps the radial wheel centered on the cursor
/// while clamped completely inside the active monitor's visible frame.
pub fn clamp_wheel_placement(
    cursor_x: i32,
    cursor_y: i32,
    desired_size: i32,
    area: ScreenRect,
) -> WheelPlacement {
    let size = desired_size.min(area.width).min(area.height).max(0);
    let max_x = area.x + area.width - size;
    let max_y = area.y + area.height - size;
    let x = (cursor_x - size / 2).clamp(area.x, max_x.max(area.x));
    let y = (cursor_y - size / 2).clamp(area.y, max_y.max(area.y));
    WheelPlacement { x, y, size }
}

fn function_key_code(n: u8) -> Option<u16> {
    match n {
        1 => Some(122),
        2 => Some(120),
        3 => Some(99),
        4 => Some(118),
        5 => Some(96),
        6 => Some(97),
        7 => Some(98),
        8 => Some(100),
        9 => Some(101),
        10 => Some(109),
        11 => Some(103),
        12 => Some(111),
        _ => None,
    }
}

fn ascii_key_code(ch: char) -> Option<u16> {
    match ch.to_ascii_lowercase() {
        'a' => Some(0),
        's' => Some(1),
        'd' => Some(2),
        'f' => Some(3),
        'h' => Some(4),
        'g' => Some(5),
        'z' => Some(6),
        'x' => Some(7),
        'c' => Some(8),
        'v' => Some(9),
        'b' => Some(11),
        'q' => Some(12),
        'w' => Some(13),
        'e' => Some(14),
        'r' => Some(15),
        'y' => Some(16),
        't' => Some(17),
        '1' => Some(18),
        '2' => Some(19),
        '3' => Some(20),
        '4' => Some(21),
        '6' => Some(22),
        '5' => Some(23),
        '9' => Some(25),
        '7' => Some(26),
        '8' => Some(28),
        '0' => Some(29),
        'o' => Some(31),
        'u' => Some(32),
        'i' => Some(34),
        'p' => Some(35),
        'l' => Some(37),
        'j' => Some(38),
        'k' => Some(40),
        'n' => Some(45),
        'm' => Some(46),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_drag_trigger_is_down_arrow() {
        let s = DragTriggerShortcut::parse(Some(DragTriggerShortcut::DEFAULT)).unwrap();
        assert_eq!(s.key_code, Some(125));
        assert_eq!(s.id, "Down");
    }

    #[test]
    fn drag_trigger_roundtrip_and_invalid_rejection() {
        for id in [
            "Down", "Up", "Left", "Right", "Space", "Tab", "Fn", "F1", "F8", "F12", "Option",
            "A", "7",
        ] {
            let parsed = DragTriggerShortcut::parse(Some(id)).unwrap();
            let round = DragTriggerShortcut::parse(Some(&parsed.id)).unwrap();
            assert_eq!(parsed, round);
        }

        for bad in ["Escape", "Esc", "Return", "Enter", "", "Option+Space", "UnknownKey"] {
            assert!(
                DragTriggerShortcut::parse(Some(bad)).is_err(),
                "expected {bad} to be rejected"
            );
        }
        assert!(DragTriggerShortcut::parse(None).is_err());
    }

    #[test]
    fn idle_shortcut_parsing_symbols_and_modifiers() {
        let a = IdleShortcut::parse(IdleShortcut::DEFAULT).unwrap();
        let b = IdleShortcut::parse("⌥⇧Space").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.canonical_id(), "Option+Shift+Space");
        assert!(IdleShortcut::parse("Space").is_err());
        assert!(IdleShortcut::parse("F8").is_ok());
    }

    #[test]
    fn press_gate_ported_from_input_tests() {
        let mut consumed = ShortcutPressGate::default();
        assert_eq!(consumed.process(true, true), (true, true));
        assert_eq!(consumed.process(true, true), (true, false));
        // captured keyup after mouse release
        assert_eq!(consumed.process(false, false), (true, false));
        // normal key after captured pair
        assert_eq!(consumed.process(true, false), (false, false));
        assert_eq!(consumed.process(false, false), (false, false));

        // typing repeat not stolen by later mouse hold
        let mut typing = ShortcutPressGate::default();
        assert_eq!(typing.process(true, false), (false, false));
        assert_eq!(typing.process(true, true), (false, false));
        assert_eq!(typing.process(false, true), (false, false));

        // shortcut change resets old press
        assert_eq!(consumed.process(true, true), (true, true));
        consumed.reset();
        assert_eq!(consumed.process(true, true), (true, true));
        assert_eq!(consumed.process(false, false), (true, false));

        // key-first drag reserves repeats only
        let mut first_normal = ShortcutPressGate::default();
        first_normal.process(true, false);
        first_normal.reserve_repeats_for_gesture();
        assert_eq!(first_normal.process(true, false), (true, false));
        assert_eq!(first_normal.process(false, false), (false, false));
    }

    #[test]
    fn order_gate_ported_from_input_tests() {
        let mut key_first = ShortcutOrderGate::default();
        assert!(!key_first.key_down(false));
        assert!(!key_first.move_cursor(200, 200));

        key_first.mouse_down(20, 20);
        // click without drag stays closed (< 8 px)
        assert!(!key_first.move_cursor(22, 22));
        // key first then drag opens (>= 8 px)
        assert!(key_first.move_cursor(30, 20));
        // opens once
        assert!(!key_first.move_cursor(60, 20));

        let mut drag_first = ShortcutOrderGate::default();
        assert!(drag_first.key_down(true));
        assert!(!drag_first.key_down(true));

        let mut released_first = ShortcutOrderGate::default();
        released_first.key_down(false);
        released_first.key_up();
        released_first.mouse_down(0, 0);
        assert!(!released_first.move_cursor(20, 0));

        let mut missed_mouse_up = ShortcutOrderGate::default();
        missed_mouse_up.key_down(false);
        missed_mouse_up.mouse_down(0, 0);
        missed_mouse_up.key_up();
        missed_mouse_up.key_down(false);
        assert!(!missed_mouse_up.move_cursor(100, 100));
        missed_mouse_up.mouse_down(100, 100);
        assert!(missed_mouse_up.move_cursor(110, 100));

        let mut click_first = ShortcutOrderGate::default();
        click_first.key_down(false);
        click_first.mouse_down(0, 0);
        click_first.mouse_up();
        click_first.mouse_down(10, 10);
        assert!(click_first.move_cursor(25, 10));

        key_first.reset();
        assert!(!key_first.held());
        assert!(!key_first.move_cursor(300, 300));
    }

    #[test]
    fn wheel_placement_clamped_to_visible_monitor() {
        let area = ScreenRect {
            x: 0,
            y: 25,
            width: 1440,
            height: 875,
        };
        let top_left = clamp_wheel_placement(10, 10, 320, area);
        assert_eq!(top_left, WheelPlacement { x: 0, y: 25, size: 320 });
        let bottom_right = clamp_wheel_placement(1435, 895, 320, area);
        assert_eq!(
            bottom_right,
            WheelPlacement {
                x: 1440 - 320,
                y: 25 + 875 - 320,
                size: 320
            }
        );
    }
}
