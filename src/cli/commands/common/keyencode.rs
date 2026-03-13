//! Encodes crossterm key events into terminal byte sequences.
//!
//! Converts [`crossterm::event::KeyEvent`] values into the byte strings a
//! remote terminal expects (Xterm encoding).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode a crossterm [`KeyEvent`] into the byte string a remote terminal
/// expects, or `None` for unrepresentable keys (modifier-only, media, etc.).
pub fn encode_key(event: &KeyEvent) -> Option<String> {
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
        KeyCode::Up => "\x1b[A",
        KeyCode::Down => "\x1b[B",
        KeyCode::Right => "\x1b[C",
        KeyCode::Left => "\x1b[D",
        KeyCode::Home => "\x1b[H",
        KeyCode::End => "\x1b[F",
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
        assert_eq!(encode_key(&key(KeyCode::Char('a'))).unwrap(), "a");
        assert_eq!(encode_key(&key(KeyCode::Char('Z'))).unwrap(), "Z");
        assert_eq!(encode_key(&key(KeyCode::Char('5'))).unwrap(), "5");
        assert_eq!(encode_key(&key(KeyCode::Char('@'))).unwrap(), "@");
    }

    #[test]
    fn ctrl_c_should_encode_to_etx() {
        assert_eq!(encode_key(&key_ctrl(KeyCode::Char('c'))).unwrap(), "\x03");
    }

    #[test]
    fn ctrl_a_should_encode_to_soh() {
        assert_eq!(encode_key(&key_ctrl(KeyCode::Char('a'))).unwrap(), "\x01");
    }

    #[test]
    fn ctrl_z_should_encode_to_sub() {
        assert_eq!(encode_key(&key_ctrl(KeyCode::Char('z'))).unwrap(), "\x1a");
    }

    #[test]
    fn alt_a_should_encode_with_escape_prefix() {
        assert_eq!(encode_key(&key_alt(KeyCode::Char('a'))).unwrap(), "\x1ba");
    }

    #[test]
    fn enter_should_encode_to_cr() {
        assert_eq!(encode_key(&key(KeyCode::Enter)).unwrap(), "\r");
    }

    #[test]
    fn backspace_should_encode_to_del() {
        assert_eq!(encode_key(&key(KeyCode::Backspace)).unwrap(), "\x7f");
    }

    #[test]
    fn tab_should_encode_to_ht() {
        assert_eq!(encode_key(&key(KeyCode::Tab)).unwrap(), "\t");
    }

    #[test]
    fn escape_should_encode_to_esc() {
        assert_eq!(encode_key(&key(KeyCode::Esc)).unwrap(), "\x1b");
    }

    #[test]
    fn arrow_keys_should_encode_to_csi_sequences() {
        assert_eq!(encode_key(&key(KeyCode::Up)).unwrap(), "\x1b[A");
        assert_eq!(encode_key(&key(KeyCode::Down)).unwrap(), "\x1b[B");
        assert_eq!(encode_key(&key(KeyCode::Right)).unwrap(), "\x1b[C");
        assert_eq!(encode_key(&key(KeyCode::Left)).unwrap(), "\x1b[D");
    }

    #[test]
    fn function_keys_should_encode_correctly() {
        assert_eq!(encode_key(&key(KeyCode::F(1))).unwrap(), "\x1bOP");
        assert_eq!(encode_key(&key(KeyCode::F(4))).unwrap(), "\x1bOS");
        assert_eq!(encode_key(&key(KeyCode::F(5))).unwrap(), "\x1b[15~");
        assert_eq!(encode_key(&key(KeyCode::F(12))).unwrap(), "\x1b[24~");
    }

    #[test]
    fn f13_should_return_none() {
        assert!(encode_key(&key(KeyCode::F(13))).is_none());
    }

    #[test]
    fn home_end_should_encode_correctly() {
        assert_eq!(encode_key(&key(KeyCode::Home)).unwrap(), "\x1b[H");
        assert_eq!(encode_key(&key(KeyCode::End)).unwrap(), "\x1b[F");
    }

    #[test]
    fn insert_delete_page_should_encode_correctly() {
        assert_eq!(encode_key(&key(KeyCode::Insert)).unwrap(), "\x1b[2~");
        assert_eq!(encode_key(&key(KeyCode::Delete)).unwrap(), "\x1b[3~");
        assert_eq!(encode_key(&key(KeyCode::PageUp)).unwrap(), "\x1b[5~");
        assert_eq!(encode_key(&key(KeyCode::PageDown)).unwrap(), "\x1b[6~");
    }

    #[test]
    fn backtab_should_encode_to_csi_z() {
        assert_eq!(encode_key(&key(KeyCode::BackTab)).unwrap(), "\x1b[Z");
    }

    #[test]
    fn ctrl_at_should_encode_to_nul() {
        assert_eq!(encode_key(&key_ctrl(KeyCode::Char('@'))).unwrap(), "\x00");
    }

    #[test]
    fn ctrl_bracket_should_encode_to_esc() {
        assert_eq!(encode_key(&key_ctrl(KeyCode::Char('['))).unwrap(), "\x1b");
    }
}
