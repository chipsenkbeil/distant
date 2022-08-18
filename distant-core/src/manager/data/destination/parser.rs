use super::{Destination, Host, HostParseError};

type PResult<'a, T> = Result<(&'a str, T), PError>;
type PError = &'static str;

/// Parses `s` into a [`Destination`]
pub fn parse(s: &str) -> Result<Destination, &'static str> {
    let (s, scheme) = maybe(parse_scheme)(s)?;
    let (s, username_password) = maybe(parse_username_password)(s)?;
    let (s, host) = parse_and_then(parse_until(|c| c == ':'), parse_host)(s)?;
    let (s, port) = maybe(prefixed(parse_char(':'), parse_port))(s)?;

    if !s.is_empty() {
        return Err("Str has more characters after destination");
    }

    Ok(Destination {
        scheme: scheme.map(ToString::to_string),
        username: username_password
            .as_ref()
            .and_then(|up| up.0)
            .map(ToString::to_string),
        password: username_password
            .as_ref()
            .and_then(|up| up.1)
            .map(ToString::to_string),
        host,
        port,
    })
}

fn parse_scheme(s: &str) -> PResult<&str> {
    let (scheme, remaining) = s.split_once("://").ok_or("Scheme missing ://")?;

    if scheme
        .chars()
        .all(|c| c.is_alphanumeric() || c == '+' || c == '.' || c == '-')
    {
        Ok((remaining, scheme))
    } else {
        Err("Invalid scheme")
    }
}

fn parse_username_password(s: &str) -> PResult<(Option<&str>, Option<&str>)> {
    let (auth, remaining) = s.split_once('@').ok_or("Auth missing @")?;
    let (auth, username) = maybe(parse_until(|c| !c.is_alphanumeric()))(auth)?;
    let (auth, password) = maybe(prefixed(
        parse_char(':'),
        parse_until(|c| !c.is_alphanumeric()),
    ))(auth)?;

    if !auth.is_empty() {
        return Err("Dangling characters after username/password");
    }

    Ok((remaining, (username, password)))
}

fn parse_host(s: &str) -> PResult<Host> {
    let host = s.parse::<Host>().map_err(HostParseError::into_static_str)?;
    Ok(("", host))
}

fn parse_port(s: &str) -> PResult<u16> {
    let port = s
        .parse::<u16>()
        .map_err(|_| "Not an unsigned 16-bit integer")?;

    Ok(("", port))
}

/// Execute two parsers in a row, failing if either fails, and returns second parser's result
fn prefixed<'a, T1, T2>(
    prefix_parser: impl Fn(&'a str) -> PResult<'a, T1>,
    parser: impl Fn(&'a str) -> PResult<'a, T2>,
) -> impl Fn(&'a str) -> PResult<'a, T2> {
    move |s: &str| {
        let (s, _) = prefix_parser(s)?;
        let (s, value) = parser(s)?;
        Ok((s, value))
    }
}

/// Execute a parser, returning Some(value) if succeeds and None if fails
fn maybe<'a, T>(
    parser: impl Fn(&'a str) -> PResult<'a, T>,
) -> impl Fn(&'a str) -> PResult<'a, Option<T>> {
    move |s: &str| match parser(s) {
        Ok((remaining, value)) => Ok((remaining, Some(value))),
        Err(_) => Ok((s, None)),
    }
}

/// Parses using `first`, and then feeds result into `second`, failing if `second` does not fully
/// parse the result of `first`
fn parse_and_then<'a, T>(
    first: impl Fn(&'a str) -> PResult<'a, &'a str>,
    second: impl Fn(&'a str) -> PResult<'a, T>,
) -> impl Fn(&'a str) -> PResult<'a, T> {
    move |s: &str| {
        let (s, first_s) = first(s)?;
        let (first_s, value) = second(first_s)?;

        if !first_s.is_empty() {
            return Err("Second parser did not fully consume results of first parser");
        }

        Ok((s, value))
    }
}

/// Parse str until predicate returns true, failing if nothing parsed
fn parse_until(predicate: impl Fn(char) -> bool) -> impl Fn(&str) -> PResult<&str> {
    move |s: &str| {
        if s.is_empty() {
            return Err("Empty str");
        }

        let (s, value) = match s.char_indices().find(|(_, c)| predicate(*c)) {
            // Position represents the first character (at boundary) that is not alphanumeric
            Some((i, _)) => (&s[i..], &s[..i]),

            // No position means that the remainder of the str was alphanumeric
            None => ("", s),
        };

        if value.is_empty() {
            return Err("Predicate immediately returned true");
        }

        Ok((s, value))
    }
}

/// Parse a single character
fn parse_char(c: char) -> impl Fn(&str) -> PResult<char> {
    move |s: &str| {
        if s.is_empty() {
            return Err("Empty str");
        }

        if s.starts_with(c) {
            Ok((&s[1..], c))
        } else {
            Err("Wrong char")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_should_fail_if_string_is_only_whitespace() {
        let _ = parse("").unwrap_err();
        let _ = parse(" ").unwrap_err();
        let _ = parse("\t").unwrap_err();
        let _ = parse("\n").unwrap_err();
        let _ = parse("\r").unwrap_err();
        let _ = parse("\r\n").unwrap_err();
    }

    #[test]
    fn parse_should_succeed_when_parsing_valid_destination() {
        // Minimal example
        let destination = parse("example.com").unwrap();
        assert_eq!(destination.scheme, None);
        assert_eq!(destination.username, None);
        assert_eq!(destination.password, None);
        assert_eq!(destination.host, "example.com");
        assert_eq!(destination.port, None);

        // Full example
        let destination = parse("scheme://username:password@example.com:22").unwrap();
        assert_eq!(destination.scheme.as_deref(), Some("scheme"));
        assert_eq!(destination.username.as_deref(), Some("username"));
        assert_eq!(destination.password.as_deref(), Some("password"));
        assert_eq!(destination.host, "example.com");
        assert_eq!(destination.port, Some(22));
    }

    #[test]
    fn parse_should_fail_if_given_path() {
        let _ = parse("/").unwrap_err();
        let _ = parse("/localhost").unwrap_err();
        let _ = parse("my/path").unwrap_err();
        let _ = parse("/my/path").unwrap_err();
        let _ = parse("//localhost").unwrap_err();
    }

    mod parsers {
        use super::*;

        fn parse_fail(_: &str) -> PResult<&str> {
            Err("bad parser")
        }

        fn parse_all(s: &str) -> PResult<&str> {
            Ok(("", s))
        }

        fn parse_cnt(cnt: usize) -> impl Fn(&str) -> PResult<&str> {
            move |s: &str| match s.char_indices().nth(cnt) {
                Some((i, _)) => Ok((&s[i..], &s[..i])),
                None => Err("Not enough characters"),
            }
        }

        mod parse_scheme {
            use super::*;

            #[test]
            fn should_fail_if_not_ending_properly() {
                let _ = parse_scheme("scheme").unwrap_err();
            }

            #[test]
            fn should_fail_if_scheme_has_invalid_character() {
                let _ = parse_scheme("sche_me://").unwrap_err();
            }

            #[test]
            fn should_return_scheme_if_valid() {
                let (s, scheme) = parse_scheme("scheme+.-://").unwrap();
                assert_eq!(s, "");
                assert_eq!(scheme, "scheme+.-");
            }

            #[test]
            fn should_consume_up_to_the_ending_sequence() {
                let (s, scheme) = parse_scheme("scheme+.-://example.com").unwrap();
                assert_eq!(s, "example.com");
                assert_eq!(scheme, "scheme+.-");
            }
        }

        mod parse_username_password {
            use super::*;

            #[test]
            fn should_fail_if_not_ending_properly() {
                let _ = parse_username_password("username:password").unwrap_err();
            }

            #[test]
            fn should_fail_if_username_not_alphanumeric() {
                let _ = parse_username_password("us\x1bername:password@").unwrap_err();
            }

            #[test]
            fn should_fail_if_password_not_alphanumeric() {
                let _ = parse_username_password("username:pas\x1bsword@").unwrap_err();
            }

            #[test]
            fn should_return_username_if_available() {
                let (s, username_password) = parse_username_password("username@").unwrap();
                assert_eq!(s, "");
                assert_eq!(username_password.0, Some("username"));
                assert_eq!(username_password.1, None);
            }

            #[test]
            fn should_return_password_if_available() {
                let (s, username_password) = parse_username_password(":password@").unwrap();
                assert_eq!(s, "");
                assert_eq!(username_password.0, None);
                assert_eq!(username_password.1, Some("password"));
            }

            #[test]
            fn should_return_username_and_password_if_available() {
                let (s, username_password) = parse_username_password("username:password@").unwrap();
                assert_eq!(s, "");
                assert_eq!(username_password.0, Some("username"));
                assert_eq!(username_password.1, Some("password"));
            }

            #[test]
            fn should_consume_up_to_the_ending_sequence() {
                let (s, username_password) =
                    parse_username_password("username:password@example.com").unwrap();
                assert_eq!(s, "example.com");
                assert_eq!(username_password.0, Some("username"));
                assert_eq!(username_password.1, Some("password"));
            }
        }

        mod parse_host {
            use super::*;
            use std::net::{Ipv4Addr, Ipv6Addr};

            #[test]
            fn should_fail_if_domain_name_is_invalid() {
                let _ = parse_host("").unwrap_err();
                let _ = parse_host(".").unwrap_err();
            }

            #[test]
            fn should_succeed_if_ipv4_address() {
                let (s, host) = parse_host("127.0.0.1").unwrap();
                assert_eq!(s, "");
                assert_eq!(host, Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1)));
            }

            #[test]
            fn should_succeed_if_ipv6_address() {
                let (s, host) = parse_host("::1").unwrap();
                assert_eq!(s, "");
                assert_eq!(host, Host::Ipv6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)));
            }

            #[test]
            fn should_succeed_if_domain_name_is_valid() {
                let (s, host) = parse_host("example.com").unwrap();
                assert_eq!(s, "");
                assert_eq!(host, Host::Name("example.com".to_string()));
            }
        }

        mod parse_port {
            use super::*;

            #[test]
            fn should_fail_if_input_cannot_be_parsed_as_a_u16() {
                let _ = parse_port("").unwrap_err();
                let _ = parse_port("a").unwrap_err();
                let _ = parse_port("-1").unwrap_err();
                let _ = parse_port("0.1").unwrap_err();
                let _ = parse_port(&(u16::MAX as u32 + 1u32).to_string()).unwrap_err();
            }

            #[test]
            fn should_succeed_if_input_can_be_parsed_as_a_u16() {
                let (s, value) = parse_port("12345").unwrap();
                assert_eq!(s, "");
                assert_eq!(value, 12345);
            }
        }

        mod prefixed {
            use super::*;

            #[test]
            fn should_fail_if_prefix_parser_fails() {
                let _ = prefixed(parse_fail, parse_all)("abc").unwrap_err();
            }

            #[test]
            fn should_fail_if_main_parser_fails() {
                let _ = prefixed(parse_cnt(1), parse_fail)("abc").unwrap_err();
            }

            #[test]
            fn should_return_value_of_main_parser_when_succeeds() {
                let (s, value) = prefixed(parse_cnt(1), parse_cnt(1))("abc").unwrap();
                assert_eq!(s, "c");
                assert_eq!(value, "b");
            }
        }

        mod maybe {
            use super::*;

            #[test]
            fn should_return_some_value_if_wrapped_parser_succeeds() {
                let (s, value) = maybe(parse_cnt(2))("abc").unwrap();
                assert_eq!(s, "c");
                assert_eq!(value, Some("ab"));
            }

            #[test]
            fn should_return_none_if_wrapped_parser_fails() {
                let (s, value) = maybe(parse_fail)("abc").unwrap();
                assert_eq!(s, "abc");
                assert_eq!(value, None);
            }
        }

        mod parse_and_then {
            use super::*;

            #[test]
            fn should_fail_if_first_parser_fails() {
                let _ = parse_and_then(parse_fail, parse_all)("abc").unwrap_err();
            }

            #[test]
            fn should_fail_if_second_parser_fails() {
                let _ = parse_and_then(parse_all, parse_fail)("abc").unwrap_err();
            }

            #[test]
            fn should_fail_if_second_parser_does_not_fully_consume_first_parser_output() {
                let _ = parse_and_then(parse_all, parse_cnt(2))("abc").unwrap_err();
            }

            #[test]
            fn should_consume_with_first_parser_and_then_return_results_of_feeding_into_second_parser(
            ) {
                let (s, text) = parse_and_then(parse_cnt(2), parse_all)("abc").unwrap();
                assert_eq!(s, "c");
                assert_eq!(text, "ab");
            }
        }

        mod parse_until {
            use super::*;

            #[test]
            fn should_consume_until_predicate_matches() {
                let (s, text) = parse_until(|c| c == 'b')("abc").unwrap();
                assert_eq!(s, "bc");
                assert_eq!(text, "a");
            }

            #[test]
            fn should_consume_completely_if_predicate_never_matches() {
                let (s, text) = parse_until(|c| c == 'z')("abc").unwrap();
                assert_eq!(s, "");
                assert_eq!(text, "abc");
            }

            #[test]
            fn should_fail_if_nothing_consumed() {
                let _ = parse_until(|c| c == 'a')("abc").unwrap_err();
            }

            #[test]
            fn should_fail_if_input_is_empty() {
                let _ = parse_until(|c| c == 'a')("").unwrap_err();
            }
        }

        mod parse_char {
            use super::*;

            #[test]
            fn should_succeed_if_next_char_matches() {
                let (s, c) = parse_char('a')("abc").unwrap();
                assert_eq!(s, "bc");
                assert_eq!(c, 'a');
            }

            #[test]
            fn should_fail_if_next_char_does_not_match() {
                let _ = parse_char('b')("abc").unwrap_err();
            }

            #[test]
            fn should_fail_if_input_is_empty() {
                let _ = parse_char('a')("").unwrap_err();
            }
        }
    }

    mod examples {
        use super::*;

        #[test]
        fn parse_should_succeed_if_given_just_host() {
            let destination = parse("example.com").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username, None);
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_and_host() {
            let destination = parse("scheme://example.com").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username, None);
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_username_and_host() {
            let destination = parse("username@example.com").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_password_and_host() {
            let destination = parse(":password@example.com").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username, None);
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_host_and_port() {
            let destination = parse("example.com:22").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username, None);
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_username_and_host() {
            let destination = parse("scheme://username@example.com").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_password_and_host() {
            let destination = parse("scheme://:password@example.com").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username, None);
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_host_and_port() {
            let destination = parse("scheme://example.com:22").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username, None);
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_username_password_and_host() {
            let destination = parse("scheme://username:password@example.com").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_username_host_and_port() {
            let destination = parse("scheme://username@example.com:22").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_password_host_and_port() {
            let destination = parse("scheme://:password@example.com:22").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username, None);
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_if_given_scheme_username_password_host_and_port() {
            let destination = parse("scheme://username:password@example.com:22").unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("scheme"));
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_if_given_username_password_and_host() {
            let destination = parse("username:password@example.com").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, None);
        }

        #[test]
        fn parse_should_succeed_if_given_username_host_and_port() {
            let destination = parse("username@example.com:22").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password, None);
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_if_given_password_host_and_port() {
            let destination = parse(":password@example.com:22").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username, None);
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_if_given_username_password_host_and_port() {
            let destination = parse("username:password@example.com:22").unwrap();
            assert_eq!(destination.scheme, None);
            assert_eq!(destination.username.as_deref(), Some("username"));
            assert_eq!(destination.password.as_deref(), Some("password"));
            assert_eq!(destination.host, "example.com");
            assert_eq!(destination.port, Some(22));
        }

        #[test]
        fn parse_should_succeed_with_distant_server_output() {
            // This is an example of what a server might output that includes a 32-byte key
            let destination = parse(concat!(
                "distant://",
                ":d561d38251700a5ac0b162c19e0c961832a64990ee19e33f7a5728f0615b2013@",
                "localhost",
                ":59699",
            ))
            .unwrap();
            assert_eq!(destination.scheme.as_deref(), Some("distant"));
            assert_eq!(destination.username.as_deref(), None);
            assert_eq!(
                destination.password.as_deref(),
                Some("d561d38251700a5ac0b162c19e0c961832a64990ee19e33f7a5728f0615b2013")
            );
            assert_eq!(destination.host, "localhost");
            assert_eq!(destination.port, Some(59699));
        }
    }
}
