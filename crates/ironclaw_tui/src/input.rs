//! Key handling and command parsing for the TUI.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Parsed user command from keyboard input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// Submit the current input text to the agent.
    Submit,
    /// Quit the TUI.
    Quit,
    /// Toggle sidebar visibility.
    ToggleSidebar,
    /// Toggle between Conversation and Logs tabs.
    ToggleLogs,
    /// Scroll conversation up.
    ScrollUp,
    /// Scroll conversation down.
    ScrollDown,
    /// Cancel / interrupt current operation.
    Interrupt,
    /// Navigate approval dialog up.
    ApprovalUp,
    /// Navigate approval dialog down.
    ApprovalDown,
    /// Confirm approval selection.
    ApprovalConfirm,
    /// Cancel approval (deny).
    ApprovalCancel,
    /// Quick approve.
    QuickApprove,
    /// Quick always-approve.
    QuickAlways,
    /// Quick deny.
    QuickDeny,
    /// Navigate command palette up.
    PaletteUp,
    /// Navigate command palette down.
    PaletteDown,
    /// Select the highlighted command palette item.
    PaletteSelect,
    /// Close the command palette.
    PaletteClose,
    /// No recognized action — pass to input box.
    Forward,
}

/// Map a key event to an action, considering whether an approval dialog or
/// the command palette is active.
pub fn map_key(key: KeyEvent, approval_active: bool, palette_active: bool) -> InputAction {
    if approval_active {
        return map_approval_key(key);
    }

    if palette_active {
        return map_palette_key(key);
    }

    match (key.code, key.modifiers) {
        (KeyCode::Enter, KeyModifiers::NONE) => InputAction::Submit,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => InputAction::Quit,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => InputAction::ToggleSidebar,
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => InputAction::ToggleLogs,
        (KeyCode::Esc, _) => InputAction::Interrupt,
        (KeyCode::PageUp, _) => InputAction::ScrollUp,
        (KeyCode::PageDown, _) => InputAction::ScrollDown,
        // Ctrl+Up / Ctrl+Down for scroll
        (KeyCode::Up, KeyModifiers::CONTROL) => InputAction::ScrollUp,
        (KeyCode::Down, KeyModifiers::CONTROL) => InputAction::ScrollDown,
        _ => InputAction::Forward,
    }
}

/// Map key events when the command palette is active.
fn map_palette_key(key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Up => InputAction::PaletteUp,
        KeyCode::Down => InputAction::PaletteDown,
        KeyCode::Enter | KeyCode::Tab => InputAction::PaletteSelect,
        KeyCode::Esc => InputAction::PaletteClose,
        // Ctrl-C should still quit
        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => InputAction::Quit,
        // Everything else goes to the textarea (palette will re-filter after)
        _ => InputAction::Forward,
    }
}

/// Map key events when the approval dialog is active.
fn map_approval_key(key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => InputAction::ApprovalUp,
        KeyCode::Down | KeyCode::Char('j') => InputAction::ApprovalDown,
        KeyCode::Enter => InputAction::ApprovalConfirm,
        KeyCode::Esc => InputAction::ApprovalCancel,
        KeyCode::Char('y') | KeyCode::Char('Y') => InputAction::QuickApprove,
        KeyCode::Char('a') | KeyCode::Char('A') => InputAction::QuickAlways,
        KeyCode::Char('n') | KeyCode::Char('N') => InputAction::QuickDeny,
        _ => InputAction::Forward,
    }
}

/// Parse a slash command from user input text.
pub fn parse_slash_command(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.starts_with('/') {
        Some(trimmed)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_submits() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(map_key(key, false, false), InputAction::Submit);
    }

    #[test]
    fn ctrl_c_quits() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(key, false, false), InputAction::Quit);
    }

    #[test]
    fn ctrl_b_toggles_sidebar() {
        let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(map_key(key, false, false), InputAction::ToggleSidebar);
    }

    #[test]
    fn ctrl_l_toggles_logs() {
        let key = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert_eq!(map_key(key, false, false), InputAction::ToggleLogs);
    }

    #[test]
    fn esc_interrupts() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_key(key, false, false), InputAction::Interrupt);
    }

    #[test]
    fn approval_mode_y_approves() {
        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(map_key(key, true, false), InputAction::QuickApprove);
    }

    #[test]
    fn approval_mode_n_denies() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(map_key(key, true, false), InputAction::QuickDeny);
    }

    #[test]
    fn palette_up_down() {
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(map_key(up, false, true), InputAction::PaletteUp);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(map_key(down, false, true), InputAction::PaletteDown);
    }

    #[test]
    fn palette_enter_selects() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(map_key(key, false, true), InputAction::PaletteSelect);
    }

    #[test]
    fn palette_tab_selects() {
        let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(map_key(key, false, true), InputAction::PaletteSelect);
    }

    #[test]
    fn palette_esc_closes() {
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(map_key(key, false, true), InputAction::PaletteClose);
    }

    #[test]
    fn palette_typing_forwards() {
        let key = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(map_key(key, false, true), InputAction::Forward);
    }

    #[test]
    fn slash_command_detected() {
        assert_eq!(parse_slash_command("/help"), Some("/help"));
        assert_eq!(parse_slash_command("  /quit  "), Some("/quit"));
        assert_eq!(parse_slash_command("hello"), None);
    }
}
