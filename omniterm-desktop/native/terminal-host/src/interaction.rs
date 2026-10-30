//! Decides whether a terminal can take a command. A live process is not an idle shell:
//! anything that cannot be confirmed stays `Unknown`, and unknown never auto-submits.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionState {
    Ready,
    Busy,
    Fullscreen,
    Password,
    Unknown,
    Exited,
    /// SFTP, serial and tunnel sessions have their own command semantics.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StateSource {
    Lifecycle,
    ScreenMode,
    ForegroundProcess,
    TerminalMode,
    ShellIntegration,
    PromptText,
    PendingInput,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    pub state: InteractionState,
    pub source: StateSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Foreground {
    Shell,
    OtherProcess,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellPhase {
    /// No trusted shell integration has been seen.
    #[default]
    Unobserved,
    AtPrompt,
    Running,
}

#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub running: bool,
    pub executable: bool,
    pub alternate_screen: bool,
    pub foreground: Foreground,
    /// ECHO off with line editing on, the terminal mode of a password prompt.
    pub password_mode: bool,
    pub phase: ShellPhase,
    pub pending_input: bool,
    pub cursor_line: &'a str,
}

pub fn looks_like_password_prompt(line: &str) -> bool {
    let line = line.trim_end().to_lowercase();
    ["password:", "passphrase", "password for", "密码", "口令", "verification code"]
        .iter()
        .any(|needle| line.contains(needle))
}

pub fn classify(facts: Facts<'_>) -> Interaction {
    use InteractionState as State;
    use StateSource as Source;
    let result = |state, source| Interaction { state, source };
    if !facts.running {
        return result(State::Exited, Source::Lifecycle);
    }
    if !facts.executable {
        return result(State::Unsupported, Source::Lifecycle);
    }
    if facts.alternate_screen {
        return result(State::Fullscreen, Source::ScreenMode);
    }
    if facts.password_mode {
        return result(State::Password, Source::TerminalMode);
    }
    if looks_like_password_prompt(facts.cursor_line) {
        return result(State::Password, Source::PromptText);
    }
    if facts.foreground == Foreground::OtherProcess {
        return result(State::Busy, Source::ForegroundProcess);
    }
    if facts.phase == ShellPhase::Running {
        return result(State::Busy, Source::ShellIntegration);
    }
    if facts.pending_input {
        return result(State::Unknown, Source::PendingInput);
    }
    if facts.phase == ShellPhase::AtPrompt {
        return result(State::Ready, Source::ShellIntegration);
    }
    if facts.foreground == Foreground::Shell {
        return result(State::Ready, Source::ForegroundProcess);
    }
    result(State::Unknown, Source::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts<'static> {
        Facts {
            running: true,
            executable: true,
            alternate_screen: false,
            foreground: Foreground::Unknown,
            password_mode: false,
            phase: ShellPhase::Unobserved,
            pending_input: false,
            cursor_line: "$ ",
        }
    }

    fn state(facts: Facts<'_>) -> InteractionState {
        classify(facts).state
    }

    #[test]
    fn an_ssh_session_without_integration_is_never_assumed_idle() {
        assert_eq!(state(facts()), InteractionState::Unknown);
    }

    #[test]
    fn only_a_confirmed_prompt_is_ready() {
        let at_prompt = Facts { phase: ShellPhase::AtPrompt, ..facts() };
        assert_eq!(state(at_prompt), InteractionState::Ready);
        let shell = Facts { foreground: Foreground::Shell, ..facts() };
        assert_eq!(state(shell), InteractionState::Ready);
    }

    #[test]
    fn editors_repls_and_password_prompts_are_not_shells() {
        let vim = Facts { alternate_screen: true, foreground: Foreground::Shell, ..facts() };
        assert_eq!(state(vim), InteractionState::Fullscreen);
        let repl = Facts { foreground: Foreground::OtherProcess, phase: ShellPhase::AtPrompt, ..facts() };
        assert_eq!(state(repl), InteractionState::Busy);
        let sudo = Facts { password_mode: true, ..facts() };
        assert_eq!(state(sudo), InteractionState::Password);
        let ssh = Facts { cursor_line: "alice@host's password: ", phase: ShellPhase::AtPrompt, ..facts() };
        assert_eq!(state(ssh), InteractionState::Password);
    }

    #[test]
    fn a_running_command_and_unsent_input_block_submission() {
        let running = Facts { phase: ShellPhase::Running, foreground: Foreground::Shell, ..facts() };
        assert_eq!(state(running), InteractionState::Busy);
        let typed = Facts { pending_input: true, foreground: Foreground::Shell, ..facts() };
        assert_eq!(state(typed), InteractionState::Unknown);
    }

    #[test]
    fn exited_and_non_shell_sessions_are_labeled() {
        assert_eq!(state(Facts { running: false, ..facts() }), InteractionState::Exited);
        assert_eq!(state(Facts { executable: false, ..facts() }), InteractionState::Unsupported);
    }
}
