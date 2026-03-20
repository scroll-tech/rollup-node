//! Shared terminal utilities for the debug REPLs.

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::{io::Write, time::Duration};

/// RAII guard: enable raw mode on create, disable on drop.
pub(super) struct RawModeGuard;

impl RawModeGuard {
    pub(super) fn new() -> eyre::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

pub(super) enum InputAction {
    Command(String),
    Quit,
    None,
}

/// Drain pending key events and return the next user action.
pub(super) fn poll_keyboard(input_buffer: &mut String, prompt: &str) -> eyre::Result<InputAction> {
    let mut stdout = std::io::stdout();

    while event::poll(Duration::from_millis(0))? {
        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Enter => {
                    print!("\r\n");
                    let _ = stdout.flush();
                    let line = input_buffer.trim().to_string();
                    input_buffer.clear();
                    if !line.is_empty() {
                        return Ok(InputAction::Command(line));
                    }
                    print!("{}", prompt);
                    let _ = stdout.flush();
                }
                KeyCode::Backspace => {
                    if !input_buffer.is_empty() {
                        input_buffer.pop();
                        print!("\x08 \x08");
                        let _ = stdout.flush();
                    }
                }
                KeyCode::Char(c) => {
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' {
                        print!("\r\nUse 'exit' to quit\r\n{}{}", prompt, input_buffer);
                        let _ = stdout.flush();
                    } else if key_event.modifiers.contains(KeyModifiers::CONTROL) && c == 'd' {
                        print!("\r\n");
                        let _ = stdout.flush();
                        return Ok(InputAction::Quit);
                    } else {
                        input_buffer.push(c);
                        print!("{}", c);
                        let _ = stdout.flush();
                    }
                }
                KeyCode::Esc => {
                    input_buffer.clear();
                    print!("\r\x1b[K{}", prompt);
                    let _ = stdout.flush();
                }
                _ => {}
            }
        }
    }

    Ok(InputAction::None)
}
