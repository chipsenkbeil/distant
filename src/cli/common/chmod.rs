/// Applies a symbolic chmod mode string (e.g. `u+x`, `go-rw`, `a=r`) to a base mode.
///
/// Returns the modified mode as a `u32` with standard Unix permission bits (0o777 mask).
///
/// # Supported syntax
///
/// - **Who**: `u` (owner), `g` (group), `o` (other), `a` (all). Multiple allowed, e.g. `ug`.
///   If omitted, defaults to `a`.
/// - **Operator**: `+` (add), `-` (remove), `=` (set exactly).
/// - **Permissions**: `r` (read), `w` (write), `x` (execute). Multiple allowed.
///
/// Multiple clauses can be comma-separated: `u+x,go-w`.
///
/// # Errors
///
/// Returns an error if the string contains unexpected characters or is malformed.
pub fn apply_symbolic_mode(base: u32, mode_str: &str) -> Result<u32, String> {
    let mut result = base & 0o777;

    for clause in mode_str.split(',') {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }

        let bytes = clause.as_bytes();
        let mut i = 0;

        // Parse "who" characters
        let mut who_mask: u32 = 0;
        while i < bytes.len() && matches!(bytes[i], b'u' | b'g' | b'o' | b'a') {
            match bytes[i] {
                b'u' => who_mask |= 0o700,
                b'g' => who_mask |= 0o070,
                b'o' => who_mask |= 0o007,
                b'a' => who_mask |= 0o777,
                _ => unreachable!(),
            }
            i += 1;
        }

        // Default to "all" if no who specified
        if who_mask == 0 {
            who_mask = 0o777;
        }

        // Parse operator
        if i >= bytes.len() || !matches!(bytes[i], b'+' | b'-' | b'=') {
            return Err(format!(
                "Expected '+', '-', or '=' operator in mode clause '{clause}'"
            ));
        }
        let op = bytes[i];
        i += 1;

        // Parse permission characters
        let mut perm_bits: u32 = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'r' => perm_bits |= 0o444,
                b'w' => perm_bits |= 0o222,
                b'x' => perm_bits |= 0o111,
                _ => {
                    return Err(format!(
                        "Unexpected permission character '{}' in mode clause '{clause}'",
                        bytes[i] as char
                    ));
                }
            }
            i += 1;
        }

        // Apply only the bits that intersect with the who mask
        let effective = perm_bits & who_mask;

        match op {
            b'+' => result |= effective,
            b'-' => result &= !effective,
            b'=' => {
                // Clear all bits for the specified "who", then set the new ones
                result = (result & !who_mask) | effective;
            }
            _ => unreachable!(),
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_user_execute() {
        assert_eq!(apply_symbolic_mode(0o644, "u+x").unwrap(), 0o744);
    }

    #[test]
    fn remove_group_other_write() {
        assert_eq!(apply_symbolic_mode(0o666, "go-w").unwrap(), 0o644);
    }

    #[test]
    fn set_all_read_only() {
        assert_eq!(apply_symbolic_mode(0o777, "a=r").unwrap(), 0o444);
    }

    #[test]
    fn multiple_clauses() {
        assert_eq!(apply_symbolic_mode(0o000, "u+rwx,g+rx,o+r").unwrap(), 0o754);
    }

    #[test]
    fn default_who_is_all() {
        assert_eq!(apply_symbolic_mode(0o000, "+x").unwrap(), 0o111);
    }

    #[test]
    fn set_exact_clears_other_bits() {
        assert_eq!(apply_symbolic_mode(0o777, "u=r").unwrap(), 0o477);
    }

    #[test]
    fn invalid_operator_returns_error() {
        assert!(apply_symbolic_mode(0o644, "u*x").is_err());
    }

    #[test]
    fn invalid_permission_char_returns_error() {
        assert!(apply_symbolic_mode(0o644, "u+z").is_err());
    }
}
