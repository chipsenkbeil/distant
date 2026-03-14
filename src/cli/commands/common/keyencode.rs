//! Encodes crossterm key events into terminal byte sequences.
//!
//! Converts [`crossterm::event::KeyEvent`] values into the byte strings a
//! remote terminal expects (Xterm encoding).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode a crossterm [`KeyEvent`] into the byte string a remote terminal
/// expects, or `None` for unrepresentable keys (modifier-only, media, etc.).
///
/// When `application_cursor` is `true` (DECCKM mode is active), arrow keys
/// and Home/End emit SS3 sequences (`ESC O x`) instead of CSI (`ESC [ x`).
/// Applications like top, htop, and less enable DECCKM and expect SS3.
pub fn encode_key(event: &KeyEvent, application_cursor: bool) -> Option<String> {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let alt = event.modifiers.contains(KeyModifiers::ALT);

    if let KeyCode::Char(c) = event.code {
        return Some(encode_char(c, ctrl, alt));
    }

    let seq = match event.code {
        KeyCode::Enter => "\r",
        KeyCode::Backspace => "\x7f",
        KeyCode::Tab => "\t",
        KeyCode::BackTab => "\x1b[Z",
        KeyCode::Esc => "\x1b",
        KeyCode::Up => {
            if application_cursor {
                "\x1bOA"
            } else {
                "\x1b[A"
            }
        }
        KeyCode::Down => {
            if application_cursor {
                "\x1bOB"
            } else {
                "\x1b[B"
            }
        }
        KeyCode::Right => {
            if application_cursor {
                "\x1bOC"
            } else {
                "\x1b[C"
            }
        }
        KeyCode::Left => {
            if application_cursor {
                "\x1bOD"
            } else {
                "\x1b[D"
            }
        }
        KeyCode::Home => {
            if application_cursor {
                "\x1bOH"
            } else {
                "\x1b[H"
            }
        }
        KeyCode::End => {
            if application_cursor {
                "\x1bOF"
            } else {
                "\x1b[F"
            }
        }
        KeyCode::Insert => "\x1b[2~",
        KeyCode::Delete => "\x1b[3~",
        KeyCode::PageUp => "\x1b[5~",
        KeyCode::PageDown => "\x1b[6~",
        KeyCode::F(n) => return encode_function_key(n),
        _ => return None,
    };
    Some(seq.to_string())
}

/// Encode a printable character with Ctrl / Alt modifiers.
fn encode_char(c: char, ctrl: bool, alt: bool) -> String {
    if ctrl {
        let byte = c.to_ascii_lowercase() as u8;
        if byte.is_ascii_lowercase() {
            let ctrl_char = (byte - b'a' + 1) as char;
            return if alt {
                format!("\x1b{ctrl_char}")
            } else {
                String::from(ctrl_char)
            };
        }
        return match c {
            '@' => String::from('\0'),
            '[' => String::from('\x1b'),
            '\\' => String::from('\x1c'),
            ']' => String::from('\x1d'),
            '^' => String::from('\x1e'),
            '_' => String::from('\x1f'),
            _ => String::from(c),
        };
    }
    if alt {
        format!("\x1b{c}")
    } else {
        String::from(c)
    }
}

/// Encode F1–F12 to their standard xterm sequences.
fn encode_function_key(n: u8) -> Option<String> {
    let s = match n {
        1 => "\x1bOP",
        2 => "\x1bOQ",
        3 => "\x1bOR",
        4 => "\x1bOS",
        5 => "\x1b[15~",
        6 => "\x1b[17~",
        7 => "\x1b[18~",
        8 => "\x1b[19~",
        9 => "\x1b[20~",
        10 => "\x1b[21~",
        11 => "\x1b[23~",
        12 => "\x1b[24~",
        _ => return None,
    };
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEventKind, KeyEventState};

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn key_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn key_alt(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::ALT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn printable_char_should_encode_directly() {
        assert_eq!(encode_key(&key(KeyCode::Char('a')), false).unwrap(), "a");
        assert_eq!(encode_key(&key(KeyCode::Char('Z')), false).unwrap(), "Z");
        assert_eq!(encode_key(&key(KeyCode::Char('5')), false).unwrap(), "5");
        assert_eq!(encode_key(&key(KeyCode::Char('@')), false).unwrap(), "@");
    }

    #[test]
    fn ctrl_c_should_encode_to_etx() {
        assert_eq!(
            encode_key(&key_ctrl(KeyCode::Char('c')), false).unwrap(),
            "\x03"
        );
    }

    #[test]
    fn ctrl_a_should_encode_to_soh() {
        assert_eq!(
            encode_key(&key_ctrl(KeyCode::Char('a')), false).unwrap(),
            "\x01"
        );
    }

    #[test]
    fn ctrl_z_should_encode_to_sub() {
        assert_eq!(
            encode_key(&key_ctrl(KeyCode::Char('z')), false).unwrap(),
            "\x1a"
        );
    }

    #[test]
    fn alt_a_should_encode_with_escape_prefix() {
        assert_eq!(
            encode_key(&key_alt(KeyCode::Char('a')), false).unwrap(),
            "\x1ba"
        );
    }

    #[test]
    fn enter_should_encode_to_cr() {
        assert_eq!(encode_key(&key(KeyCode::Enter), false).unwrap(), "\r");
    }

    #[test]
    fn backspace_should_encode_to_del() {
        assert_eq!(encode_key(&key(KeyCode::Backspace), false).unwrap(), "\x7f");
    }

    #[test]
    fn tab_should_encode_to_ht() {
        assert_eq!(encode_key(&key(KeyCode::Tab), false).unwrap(), "\t");
    }

    #[test]
    fn escape_should_encode_to_esc() {
        assert_eq!(encode_key(&key(KeyCode::Esc), false).unwrap(), "\x1b");
    }

    #[test]
    fn arrow_keys_should_encode_to_csi_sequences() {
        assert_eq!(encode_key(&key(KeyCode::Up), false).unwrap(), "\x1b[A");
        assert_eq!(encode_key(&key(KeyCode::Down), false).unwrap(), "\x1b[B");
        assert_eq!(encode_key(&key(KeyCode::Right), false).unwrap(), "\x1b[C");
        assert_eq!(encode_key(&key(KeyCode::Left), false).unwrap(), "\x1b[D");
    }

    #[test]
    fn arrow_keys_should_encode_to_ss3_in_application_cursor_mode() {
        assert_eq!(encode_key(&key(KeyCode::Up), true).unwrap(), "\x1bOA");
        assert_eq!(encode_key(&key(KeyCode::Down), true).unwrap(), "\x1bOB");
        assert_eq!(encode_key(&key(KeyCode::Right), true).unwrap(), "\x1bOC");
        assert_eq!(encode_key(&key(KeyCode::Left), true).unwrap(), "\x1bOD");
    }

    #[test]
    fn function_keys_should_encode_correctly() {
        assert_eq!(encode_key(&key(KeyCode::F(1)), false).unwrap(), "\x1bOP");
        assert_eq!(encode_key(&key(KeyCode::F(4)), false).unwrap(), "\x1bOS");
        assert_eq!(encode_key(&key(KeyCode::F(5)), false).unwrap(), "\x1b[15~");
        assert_eq!(encode_key(&key(KeyCode::F(12)), false).unwrap(), "\x1b[24~");
    }

    #[test]
    fn f13_should_return_none() {
        assert!(encode_key(&key(KeyCode::F(13)), false).is_none());
    }

    #[test]
    fn home_end_should_encode_correctly() {
        assert_eq!(encode_key(&key(KeyCode::Home), false).unwrap(), "\x1b[H");
        assert_eq!(encode_key(&key(KeyCode::End), false).unwrap(), "\x1b[F");
    }

    #[test]
    fn home_end_should_encode_to_ss3_in_application_cursor_mode() {
        assert_eq!(encode_key(&key(KeyCode::Home), true).unwrap(), "\x1bOH");
        assert_eq!(encode_key(&key(KeyCode::End), true).unwrap(), "\x1bOF");
    }

    #[test]
    fn insert_delete_page_should_encode_correctly() {
        assert_eq!(encode_key(&key(KeyCode::Insert), false).unwrap(), "\x1b[2~");
        assert_eq!(encode_key(&key(KeyCode::Delete), false).unwrap(), "\x1b[3~");
        assert_eq!(encode_key(&key(KeyCode::PageUp), false).unwrap(), "\x1b[5~");
        assert_eq!(
            encode_key(&key(KeyCode::PageDown), false).unwrap(),
            "\x1b[6~"
        );
    }

    #[test]
    fn backtab_should_encode_to_csi_z() {
        assert_eq!(encode_key(&key(KeyCode::BackTab), false).unwrap(), "\x1b[Z");
    }

    #[test]
    fn ctrl_at_should_encode_to_nul() {
        assert_eq!(
            encode_key(&key_ctrl(KeyCode::Char('@')), false).unwrap(),
            "\x00"
        );
    }

    #[test]
    fn ctrl_bracket_should_encode_to_esc() {
        assert_eq!(
            encode_key(&key_ctrl(KeyCode::Char('[')), false).unwrap(),
            "\x1b"
        );
    }
}
