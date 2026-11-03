// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Application shortcuts, binding for binding as in the desktop app. macOS uses Command;
//! elsewhere Ctrl+Shift, so Ctrl+W, Ctrl+D, Ctrl+F and Ctrl+K keep reaching the shell.

use eframe::egui::{Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    ClosePane,
    SplitRight,
    SplitDown,
    NextPane,
    PreviousPane,
    PaneLeft,
    PaneRight,
    PaneUp,
    PaneDown,
    MaximizePane,
    Find,
    Clear,
    Palette,
    Broadcast,
    SendToAgent,
    ToggleAgent,
    ToggleSidebar,
    ToggleFullscreen,
    FontBigger,
    FontSmaller,
    FontReset,
    Settings,
}

#[derive(Clone, Copy)]
struct Binding {
    key: Key,
    shift: bool,
    alt: bool,
    /// Ctrl alone (no Shift) on Windows and Linux.
    ctrl_only: bool,
    /// Control+Command on macOS.
    ctrl: bool,
    /// The key alone, as F11.
    bare: bool,
}

const fn key(key: Key) -> Binding {
    Binding { key, shift: false, alt: false, ctrl_only: false, ctrl: false, bare: false }
}
const fn shift(key: Key) -> Binding {
    Binding { shift: true, ..self::key(key) }
}
const fn alt(key: Key) -> Binding {
    Binding { alt: true, ..self::key(key) }
}
const fn ctrl_only(binding: Binding) -> Binding {
    Binding { ctrl_only: true, ..binding }
}

struct Shortcut {
    action: Action,
    label: &'static str,
    mac: Binding,
    other: Binding,
}

const SHORTCUTS: &[Shortcut] = &[
    Shortcut { action: Action::NewTab, label: "新建标签", mac: key(Key::T), other: shift(Key::T) },
    Shortcut { action: Action::ClosePane, label: "关闭当前窗格", mac: key(Key::W), other: shift(Key::W) },
    Shortcut { action: Action::SplitRight, label: "向右分屏", mac: key(Key::D), other: shift(Key::E) },
    Shortcut { action: Action::SplitDown, label: "向下分屏", mac: shift(Key::D), other: shift(Key::O) },
    Shortcut { action: Action::NextPane, label: "下一个窗格", mac: key(Key::CloseBracket), other: shift(Key::CloseBracket) },
    Shortcut { action: Action::PreviousPane, label: "上一个窗格", mac: key(Key::OpenBracket), other: shift(Key::OpenBracket) },
    Shortcut { action: Action::PaneLeft, label: "切换到左侧窗格", mac: alt(Key::ArrowLeft), other: ctrl_only(alt(Key::ArrowLeft)) },
    Shortcut { action: Action::PaneRight, label: "切换到右侧窗格", mac: alt(Key::ArrowRight), other: ctrl_only(alt(Key::ArrowRight)) },
    Shortcut { action: Action::PaneUp, label: "切换到上方窗格", mac: alt(Key::ArrowUp), other: ctrl_only(alt(Key::ArrowUp)) },
    Shortcut { action: Action::PaneDown, label: "切换到下方窗格", mac: alt(Key::ArrowDown), other: ctrl_only(alt(Key::ArrowDown)) },
    Shortcut { action: Action::MaximizePane, label: "最大化 / 还原窗格", mac: shift(Key::Enter), other: shift(Key::Enter) },
    Shortcut { action: Action::Find, label: "搜索终端输出", mac: key(Key::F), other: shift(Key::F) },
    Shortcut { action: Action::Clear, label: "清空终端", mac: key(Key::K), other: shift(Key::K) },
    Shortcut { action: Action::Palette, label: "命令面板", mac: shift(Key::P), other: shift(Key::P) },
    Shortcut { action: Action::Broadcast, label: "广播输入到当前标签所有窗格", mac: alt(Key::I), other: shift(Key::I) },
    Shortcut { action: Action::SendToAgent, label: "附加选中内容到 omniTerm Agent", mac: shift(Key::A), other: shift(Key::A) },
    Shortcut { action: Action::ToggleAgent, label: "显示 / 隐藏 omniTerm Agent 面板", mac: key(Key::J), other: shift(Key::J) },
    Shortcut { action: Action::ToggleSidebar, label: "显示 / 隐藏工作区侧栏", mac: key(Key::B), other: shift(Key::B) },
    Shortcut {
        action: Action::ToggleFullscreen,
        label: "全屏终端（隐藏其他面板）",
        mac: Binding { ctrl: true, ..key(Key::F) },
        other: Binding { bare: true, ..key(Key::F11) },
    },
    Shortcut { action: Action::FontBigger, label: "放大字体", mac: key(Key::Equals), other: ctrl_only(key(Key::Equals)) },
    Shortcut { action: Action::FontSmaller, label: "缩小字体", mac: key(Key::Minus), other: ctrl_only(key(Key::Minus)) },
    Shortcut { action: Action::FontReset, label: "重置字体大小", mac: key(Key::Num0), other: ctrl_only(key(Key::Num0)) },
    Shortcut { action: Action::Settings, label: "打开设置", mac: key(Key::Comma), other: ctrl_only(key(Key::Comma)) },
];

fn matches(binding: &Binding, key: Key, modifiers: Modifiers, mac: bool) -> bool {
    // `+` arrives as Plus on some layouts and Equals on others.
    let same = key == binding.key || (binding.key == Key::Equals && key == Key::Plus);
    if !same {
        return false;
    }
    if binding.bare {
        return !modifiers.any();
    }
    if mac {
        return modifiers.mac_cmd && modifiers.ctrl == binding.ctrl && modifiers.shift == binding.shift && modifiers.alt == binding.alt;
    }
    if !modifiers.ctrl {
        return false;
    }
    if binding.ctrl_only {
        return modifiers.alt == binding.alt && (modifiers.shift == binding.shift || binding.key == Key::Equals);
    }
    modifiers.shift && modifiers.alt == binding.alt
}

pub fn action(key: Key, modifiers: Modifiers, mac: bool) -> Option<Action> {
    SHORTCUTS.iter().find(|shortcut| matches(if mac { &shortcut.mac } else { &shortcut.other }, key, modifiers, mac)).map(|shortcut| shortcut.action)
}

/// Ctrl+Tab cycles tabs everywhere; it means nothing to shells.
pub fn tab_cycle(key: Key, modifiers: Modifiers) -> i32 {
    if key != Key::Tab || !modifiers.ctrl || modifiers.alt || modifiers.mac_cmd && !modifiers.ctrl {
        return 0;
    }
    if modifiers.shift { -1 } else { 1 }
}

/// ⌘1…9 on macOS, Alt+1…9 elsewhere, as in most Linux terminals.
pub fn tab_number(key: Key, modifiers: Modifiers, mac: bool) -> Option<usize> {
    let number = match key {
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    };
    let held = if mac { modifiers.mac_cmd && !modifiers.ctrl && !modifiers.alt } else { modifiers.alt && !modifiers.ctrl && !modifiers.mac_cmd };
    (held && !modifiers.shift).then_some(number)
}

pub fn label(action: Action, mac: bool) -> String {
    let Some(shortcut) = SHORTCUTS.iter().find(|shortcut| shortcut.action == action) else { return String::new() };
    let binding = if mac { shortcut.mac } else { shortcut.other };
    let name = match binding.key {
        Key::OpenBracket => "[".to_string(),
        Key::CloseBracket => "]".to_string(),
        Key::ArrowLeft => "←".to_string(),
        Key::ArrowRight => "→".to_string(),
        Key::ArrowUp => "↑".to_string(),
        Key::ArrowDown => "↓".to_string(),
        Key::Equals => "+".to_string(),
        Key::Minus => "−".to_string(),
        Key::Comma => ",".to_string(),
        Key::Enter => "↩".to_string(),
        Key::Num0 => "0".to_string(),
        other => other.name().to_string(),
    };
    if binding.bare {
        return name;
    }
    if mac {
        return format!("{}{}{}⌘{name}", if binding.ctrl { "⌃" } else { "" }, if binding.alt { "⌥" } else { "" }, if binding.shift { "⇧" } else { "" });
    }
    let mut parts = vec!["Ctrl"];
    if binding.alt {
        parts.push("Alt");
    }
    if binding.shift || !binding.ctrl_only {
        parts.push("Shift");
    }
    format!("{}+{name}", parts.join("+"))
}

/// Every shortcut with its label, for the settings dialog.
pub fn all(mac: bool) -> Vec<(&'static str, String)> {
    SHORTCUTS.iter().map(|shortcut| (shortcut.label, label(shortcut.action, mac))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modifiers(ctrl: bool, shift: bool, alt: bool, cmd: bool) -> Modifiers {
        Modifiers { ctrl, shift, alt, mac_cmd: cmd, command: cmd || ctrl }
    }

    #[test]
    fn shell_keys_stay_with_the_shell() {
        assert_eq!(action(Key::W, modifiers(true, false, false, false), false), None);
        assert_eq!(action(Key::D, modifiers(true, false, false, false), false), None);
        assert_eq!(action(Key::B, modifiers(true, false, false, false), false), None, "tmux's prefix");
        assert_eq!(action(Key::W, modifiers(true, true, false, false), false), Some(Action::ClosePane));
    }

    #[test]
    fn platforms_use_their_own_bindings() {
        assert_eq!(action(Key::D, modifiers(false, false, false, true), true), Some(Action::SplitRight));
        assert_eq!(action(Key::D, modifiers(false, true, false, true), true), Some(Action::SplitDown));
        assert_eq!(action(Key::E, modifiers(true, true, false, false), false), Some(Action::SplitRight));
        assert_eq!(action(Key::F11, Modifiers::NONE, false), Some(Action::ToggleFullscreen));
        assert_eq!(action(Key::F, modifiers(true, false, false, true), true), Some(Action::ToggleFullscreen));
        assert_eq!(action(Key::F, modifiers(false, false, false, true), true), Some(Action::Find));
        assert_eq!(action(Key::ArrowLeft, modifiers(true, false, true, false), false), Some(Action::PaneLeft));
        assert_eq!(action(Key::Plus, modifiers(true, true, false, false), false), Some(Action::FontBigger));
    }

    #[test]
    fn labels_match_the_desktop_app() {
        assert_eq!(label(Action::SplitDown, true), "⇧⌘D");
        assert_eq!(label(Action::NewTab, false), "Ctrl+Shift+T");
        assert_eq!(label(Action::FontBigger, false), "Ctrl++");
        assert_eq!(label(Action::PaneLeft, false), "Ctrl+Alt+←");
        assert_eq!(label(Action::ToggleFullscreen, false), "F11");
        assert_eq!(label(Action::ToggleFullscreen, true), "⌃⌘F");
    }

    #[test]
    fn tabs_switch_by_number_and_cycle() {
        assert_eq!(tab_number(Key::Num3, modifiers(false, false, true, false), false), Some(3));
        assert_eq!(tab_number(Key::Num3, modifiers(false, false, false, true), true), Some(3));
        assert_eq!(tab_number(Key::Num3, modifiers(true, false, false, false), false), None);
        assert_eq!(tab_cycle(Key::Tab, modifiers(true, true, false, false)), -1);
    }
}
