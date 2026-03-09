//! # parser
//!
//! Ssh config parser

use std::fs::File;
use std::io::{BufRead, BufReader, Error as IoError};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use bitflags::bitflags;
use glob::glob;
use thiserror::Error;

use super::{Host, HostClause, HostParams, SshConfig};
use crate::DefaultAlgorithms;
use crate::params::AlgorithmsRule;

// modules
mod field;
use field::Field;

pub type SshParserResult<T> = Result<T, SshParserError>;

/// [`SshConfigParser::update_host`] result
#[derive(Debug, PartialEq, Eq)]
enum UpdateHost {
    /// Update current host
    UpdateHost,
    /// Add new hosts
    NewHosts(Vec<Host>),
}

/// Ssh config parser error
#[derive(Debug, Error)]
pub enum SshParserError {
    #[error("expected boolean value ('yes', 'no')")]
    ExpectedBoolean,
    #[error("expected port number")]
    ExpectedPort,
    #[error("expected unsigned value")]
    ExpectedUnsigned,
    #[error("expected algorithms")]
    ExpectedAlgorithms,
    #[error("expected path")]
    ExpectedPath,
    #[error("IO error: {0}")]
    Io(#[from] IoError),
    #[error("glob error: {0}")]
    Glob(#[from] glob::GlobError),
    #[error("invalid quotes")]
    InvalidQuotes,
    #[error("missing argument")]
    MissingArgument,
    #[error("pattern error: {0}")]
    PatternError(#[from] glob::PatternError),
    #[error("unknown field: {0}")]
    UnknownField(String, Vec<String>),
    #[error("unknown field: {0}")]
    UnsupportedField(String, Vec<String>),
}

bitflags! {
    /// The parsing mode
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ParseRule: u8 {
        /// Don't allow any invalid field or value
        const STRICT = 0b00000000;
        /// Allow unknown field
        const ALLOW_UNKNOWN_FIELDS = 0b00000001;
        /// Allow unsupported fields
        const ALLOW_UNSUPPORTED_FIELDS = 0b00000010;
    }
}

// -- parser

/// Ssh config parser
pub(crate) struct SshConfigParser;

impl SshConfigParser {
    /// Parse reader lines and apply parameters to configuration
    pub(crate) fn parse(
        config: &mut SshConfig,
        reader: &mut impl BufRead,
        rules: ParseRule,
        ignore_unknown: Option<Vec<String>>,
    ) -> SshParserResult<()> {
        // Options preceding the first `Host` section
        // are parsed as command line options;
        // overriding all following host-specific options.
        //
        // See https://github.com/openssh/openssh-portable/blob/master/readconf.c#L1173-L1176
        let mut default_params = HostParams::new(&config.default_algorithms);
        default_params.ignore_unknown = ignore_unknown;
        config.hosts.push(Host::new(
            vec![HostClause::new(String::from("*"), false)],
            default_params,
        ));

        // Current host pointer
        let mut current_host = config.hosts.last_mut().unwrap();

        let mut lines = reader.lines();
        // iter lines
        loop {
            let line = match lines.next() {
                None => break,
                Some(Err(err)) => return Err(SshParserError::Io(err)),
                Some(Ok(line)) => Self::strip_comments(line.trim()),
            };
            if line.is_empty() {
                continue;
            }
            // tokenize
            let (field, args) = match Self::tokenize_line(&line) {
                Ok((field, args)) => (field, args),
                Err(SshParserError::UnknownField(field, args))
                    if rules.intersects(ParseRule::ALLOW_UNKNOWN_FIELDS)
                        || current_host.params.ignored(&field) =>
                {
                    current_host.params.ignored_fields.insert(field, args);
                    continue;
                }
                Err(SshParserError::UnknownField(field, args)) => {
                    return Err(SshParserError::UnknownField(field, args));
                }
                Err(err) => return Err(err),
            };
            // If field is block, init a new block
            if field == Field::Host {
                // Pass `ignore_unknown` from global overrides down into the tokenizer.
                let mut params = HostParams::new(&config.default_algorithms);
                params.ignore_unknown = config.hosts[0].params.ignore_unknown.clone();
                let pattern = Self::parse_host(args)?;
                trace!("Adding new host: {pattern:?}",);

                // Add a new host
                config.hosts.push(Host::new(pattern, params));
                // Update current host pointer
                current_host = config.hosts.last_mut().expect("Just added hosts");
            } else {
                // Update field
                match Self::update_host(
                    field,
                    args,
                    current_host,
                    rules,
                    &config.default_algorithms,
                ) {
                    Ok(UpdateHost::UpdateHost) => Ok(()),
                    Ok(UpdateHost::NewHosts(new_hosts)) => {
                        trace!("Adding new hosts from 'UpdateHost::NewHosts': {new_hosts:?}",);
                        config.hosts.extend(new_hosts);
                        current_host = config.hosts.last_mut().expect("Just added hosts");
                        Ok(())
                    }
                    // If we're allowing unsupported fields to be parsed, add them to the map
                    Err(SshParserError::UnsupportedField(field, args))
                        if rules.intersects(ParseRule::ALLOW_UNSUPPORTED_FIELDS) =>
                    {
                        current_host.params.unsupported_fields.insert(field, args);
                        Ok(())
                    }
                    // Eat the error here to not break the API with this change
                    // Also it'd be weird to error on correct ssh_config's just because they're
                    // not supported by this library
                    Err(SshParserError::UnsupportedField(_, _)) => Ok(()),
                    Err(e) => Err(e),
                }?;
            }
        }

        Ok(())
    }

    /// Strip comments from line (quote-aware)
    fn strip_comments(s: &str) -> String {
        let mut in_quotes = false;
        let mut result = String::new();

        for c in s.chars() {
            match c {
                '"' => {
                    in_quotes = !in_quotes;
                    result.push(c);
                }
                '#' if !in_quotes => {
                    // Found a comment outside quotes, stop here
                    break;
                }
                _ => {
                    result.push(c);
                }
            }
        }

        result
    }

    /// Count unescaped double quotes in a string.
    /// A quote is considered escaped if preceded by a backslash that is not itself escaped.
    fn count_unescaped_quotes(s: &str) -> usize {
        let mut count = 0;
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                // Skip the escaped character
                i += 2;
            } else if chars[i] == '"' {
                count += 1;
                i += 1;
            } else {
                i += 1;
            }
        }
        count
    }

    /// Check if a string ends with an unescaped double quote.
    fn ends_with_unescaped_quote(s: &str) -> bool {
        if !s.ends_with('"') {
            return false;
        }
        // Count trailing backslashes before the final quote
        let chars: Vec<char> = s.chars().collect();
        let mut backslash_count = 0;
        for i in (0..chars.len() - 1).rev() {
            if chars[i] == '\\' {
                backslash_count += 1;
            } else {
                break;
            }
        }
        // If even number of backslashes, the quote is unescaped
        backslash_count % 2 == 0
    }

    /// Process escape sequences in a string.
    /// Handles: \" -> ", \\ -> \, \' -> '
    /// Unrecognized escapes preserve the backslash.
    fn unescape_string(s: &str) -> String {
        let mut result = String::with_capacity(s.len());
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                match next {
                    '"' | '\\' | '\'' => {
                        // Recognized escape sequence: skip backslash, add the character
                        result.push(next);
                        i += 2;
                    }
                    _ => {
                        // Unrecognized escape: preserve the backslash
                        result.push(chars[i]);
                        i += 1;
                    }
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    /// Update current given host with field argument
    fn update_host(
        field: Field,
        args: Vec<String>,
        host: &mut Host,
        rules: ParseRule,
        default_algos: &DefaultAlgorithms,
    ) -> SshParserResult<UpdateHost> {
        trace!("parsing field {field:?} with args {args:?}",);
        let params = &mut host.params;
        match field {
            Field::AddKeysToAgent => {
                let value = Self::parse_boolean(args)?;
                trace!("add_keys_to_agent: {value}",);
                params.add_keys_to_agent = Some(value);
            }
            Field::BindAddress => {
                let value = Self::parse_string(args)?;
                trace!("bind_address: {value}",);
                params.bind_address = Some(value);
            }
            Field::BindInterface => {
                let value = Self::parse_string(args)?;
                trace!("bind_interface: {value}",);
                params.bind_interface = Some(value);
            }
            Field::CaSignatureAlgorithms => {
                let rule = Self::parse_algos(args)?;
                trace!("ca_signature_algorithms: {rule:?}",);
                params.ca_signature_algorithms.apply(rule);
            }
            Field::CertificateFile => {
                let value = Self::parse_path(args)?;
                trace!("certificate_file: {value:?}",);
                params.certificate_file = Some(value);
            }
            Field::Ciphers => {
                let rule = Self::parse_algos(args)?;
                trace!("ciphers: {rule:?}",);
                params.ciphers.apply(rule);
            }
            Field::Compression => {
                let value = Self::parse_boolean(args)?;
                trace!("compression: {value}",);
                params.compression = Some(value);
            }
            Field::ConnectTimeout => {
                let value = Self::parse_duration(args)?;
                trace!("connect_timeout: {value:?}",);
                params.connect_timeout = Some(value);
            }
            Field::ConnectionAttempts => {
                let value = Self::parse_unsigned(args)?;
                trace!("connection_attempts: {value}",);
                params.connection_attempts = Some(value);
            }
            Field::ForwardAgent => {
                let value = Self::parse_boolean(args)?;
                trace!("forward_agent: {value}",);
                params.forward_agent = Some(value);
            }
            Field::Host => { /* already handled before */ }
            Field::HostKeyAlgorithms => {
                let rule = Self::parse_algos(args)?;
                trace!("host_key_algorithm: {rule:?}",);
                params.host_key_algorithms.apply(rule);
            }
            Field::HostName => {
                let value = Self::parse_string(args)?;
                trace!("host_name: {value}",);
                params.host_name = Some(value);
            }
            Field::Include => {
                return Self::include_files(
                    args,
                    host,
                    rules,
                    default_algos,
                    host.params.ignore_unknown.clone(),
                )
                .map(UpdateHost::NewHosts);
            }
            Field::IdentityFile => {
                let value = Self::parse_path_list(args)?;
                trace!("identity_file: {value:?}",);
                if let Some(existing) = &mut params.identity_file {
                    existing.extend(value);
                } else {
                    params.identity_file = Some(value);
                }
            }
            Field::IgnoreUnknown => {
                let value = Self::parse_comma_separated_list(args)?;
                trace!("ignore_unknown: {value:?}",);
                params.ignore_unknown = Some(value);
            }
            Field::KexAlgorithms => {
                let rule = Self::parse_algos(args)?;
                trace!("kex_algorithms: {rule:?}",);
                params.kex_algorithms.apply(rule);
            }
            Field::Mac => {
                let rule = Self::parse_algos(args)?;
                trace!("mac: {rule:?}",);
                params.mac.apply(rule);
            }
            Field::Port => {
                let value = Self::parse_port(args)?;
                trace!("port: {value}",);
                params.port = Some(value);
            }
            Field::ProxyJump => {
                let rule = Self::parse_comma_separated_list(args)?;
                trace!("proxy_jump: {rule:?}",);
                params.proxy_jump = Some(rule);
            }
            Field::PubkeyAcceptedAlgorithms => {
                let rule = Self::parse_algos(args)?;
                trace!("pubkey_accepted_algorithms: {rule:?}",);
                params.pubkey_accepted_algorithms.apply(rule);
            }
            Field::PubkeyAuthentication => {
                let value = Self::parse_boolean(args)?;
                trace!("pubkey_authentication: {value}",);
                params.pubkey_authentication = Some(value);
            }
            Field::RemoteForward => {
                let value = Self::parse_port(args)?;
                trace!("remote_forward: {value}",);
                params.remote_forward = Some(value);
            }
            Field::ServerAliveInterval => {
                let value = Self::parse_duration(args)?;
                trace!("server_alive_interval: {value:?}",);
                params.server_alive_interval = Some(value);
            }
            Field::TcpKeepAlive => {
                let value = Self::parse_boolean(args)?;
                trace!("tcp_keep_alive: {value}",);
                params.tcp_keep_alive = Some(value);
            }
            #[cfg(target_os = "macos")]
            Field::UseKeychain => {
                let value = Self::parse_boolean(args)?;
                trace!("use_keychain: {value}",);
                params.use_keychain = Some(value);
            }
            Field::User => {
                let value = Self::parse_string(args)?;
                trace!("user: {value}",);
                params.user = Some(value);
            }
            // -- unimplemented fields
            Field::AddressFamily
            | Field::BatchMode
            | Field::CanonicalDomains
            | Field::CanonicalizeFallbackLock
            | Field::CanonicalizeHostname
            | Field::CanonicalizeMaxDots
            | Field::CanonicalizePermittedCNAMEs
            | Field::CheckHostIP
            | Field::ClearAllForwardings
            | Field::ControlMaster
            | Field::ControlPath
            | Field::ControlPersist
            | Field::DynamicForward
            | Field::EnableSSHKeysign
            | Field::EscapeChar
            | Field::ExitOnForwardFailure
            | Field::FingerprintHash
            | Field::ForkAfterAuthentication
            | Field::ForwardX11
            | Field::ForwardX11Timeout
            | Field::ForwardX11Trusted
            | Field::GatewayPorts
            | Field::GlobalKnownHostsFile
            | Field::GSSAPIAuthentication
            | Field::GSSAPIDelegateCredentials
            | Field::HashKnownHosts
            | Field::HostbasedAcceptedAlgorithms
            | Field::HostbasedAuthentication
            | Field::HostKeyAlias
            | Field::HostbasedKeyTypes
            | Field::IdentitiesOnly
            | Field::IdentityAgent
            | Field::IPQoS
            | Field::KbdInteractiveAuthentication
            | Field::KbdInteractiveDevices
            | Field::KnownHostsCommand
            | Field::LocalCommand
            | Field::LocalForward
            | Field::LogLevel
            | Field::LogVerbose
            | Field::NoHostAuthenticationForLocalhost
            | Field::NumberOfPasswordPrompts
            | Field::PasswordAuthentication
            | Field::PermitLocalCommand
            | Field::PermitRemoteOpen
            | Field::PKCS11Provider
            | Field::PreferredAuthentications
            | Field::ProxyCommand
            | Field::ProxyUseFdpass
            | Field::PubkeyAcceptedKeyTypes
            | Field::RekeyLimit
            | Field::RequestTTY
            | Field::RevokedHostKeys
            | Field::SecruityKeyProvider
            | Field::SendEnv
            | Field::ServerAliveCountMax
            | Field::SessionType
            | Field::SetEnv
            | Field::StdinNull
            | Field::StreamLocalBindMask
            | Field::StrictHostKeyChecking
            | Field::SyslogFacility
            | Field::UpdateHostKeys
            | Field::UserKnownHostsFile
            | Field::VerifyHostKeyDNS
            | Field::VisualHostKey
            | Field::XAuthLocation => {
                return Err(SshParserError::UnsupportedField(field.to_string(), args));
            }
        }
        Ok(UpdateHost::UpdateHost)
    }

    /// Resolve the include path for a given path match.
    ///
    /// If the path match is absolute, it just returns the path as-is;
    /// if it is relative, it prepends $HOME/.ssh to it
    fn resolve_include_path(path_match: &str) -> String {
        #[cfg(windows)]
        const PATH_SEPARATOR: &str = "\\";
        #[cfg(unix)]
        const PATH_SEPARATOR: &str = "/";

        // if path match doesn't start with the path separator, prepend it
        if path_match.starts_with(PATH_SEPARATOR) {
            path_match.to_string()
        } else {
            let home_dir = dirs::home_dir().unwrap_or(PathBuf::from(PATH_SEPARATOR));
            // if path_match starts with `~`, strip it and prepend $HOME
            if let Some(stripped) = path_match.strip_prefix("~") {
                format!("{dir}{PATH_SEPARATOR}{stripped}", dir = home_dir.display())
            } else {
                // prepend $HOME/.ssh
                format!(
                    "{dir}{PATH_SEPARATOR}{path_match}",
                    dir = home_dir.join(".ssh").display()
                )
            }
        }
    }

    /// include a file by parsing it and updating host rules by merging the read config to the current one for the host
    fn include_files(
        args: Vec<String>,
        host: &mut Host,
        rules: ParseRule,
        default_algos: &DefaultAlgorithms,
        ignore_unknown: Option<Vec<String>>,
    ) -> SshParserResult<Vec<Host>> {
        let path_match = Self::resolve_include_path(&Self::parse_string(args)?);

        trace!("include files: {path_match}",);
        let files = glob(&path_match)?;

        let mut new_hosts = vec![];

        for file in files {
            let file = file?;
            trace!("including file: {}", file.display());
            let mut reader = BufReader::new(File::open(file)?);
            let mut sub_config = SshConfig::default().default_algorithms(default_algos.clone());
            Self::parse(&mut sub_config, &mut reader, rules, ignore_unknown.clone())?;

            // merge sub-config into host
            for pattern in &host.pattern {
                if pattern.negated {
                    trace!("excluding sub-config for pattern: {pattern:?}",);
                    continue;
                }
                trace!("merging sub-config for pattern: {pattern:?}",);
                let params = sub_config.query(&pattern.pattern);
                host.params.overwrite_if_none(&params);
            }

            // merge additional hosts
            for sub_host in sub_config.hosts.into_iter().skip(1) {
                trace!("adding sub-host: {sub_host:?}",);
                new_hosts.push(sub_host);
            }
        }

        Ok(new_hosts)
    }

    /// Tokenize line if possible. Returns [`Field`] name and args as a [`Vec`] of [`String`].
    ///
    /// All of these lines are valid for tokenization
    ///
    /// ```txt
    /// IgnoreUnknown=Pippo,Pluto
    /// ConnectTimeout = 15
    /// Ciphers "Pepperoni Pizza,Margherita Pizza,Hawaiian Pizza"
    /// Macs="Pasta Carbonara,Pasta con tonno"
    /// ```
    ///
    /// So lines have syntax `field args...`, `field=args...`, `field "args"`, `field="args"`
    fn tokenize_line(line: &str) -> SshParserResult<(Field, Vec<String>)> {
        // check what comes first, space or =?
        let trimmed_line = line.trim();
        // first token is the field, and it may be separated either by a space or by '='
        let (field, other_tokens) = if trimmed_line.find('=').unwrap_or(usize::MAX)
            < trimmed_line.find(char::is_whitespace).unwrap_or(usize::MAX)
        {
            trimmed_line
                .split_once('=')
                .ok_or(SshParserError::MissingArgument)?
        } else {
            trimmed_line
                .split_once(char::is_whitespace)
                .ok_or(SshParserError::MissingArgument)?
        };

        trace!("tokenized line '{line}' - field '{field}' with args '{other_tokens}'",);

        // other tokens should trim = and whitespace
        let other_tokens = other_tokens.trim().trim_start_matches('=').trim();
        trace!("other tokens trimmed: '{other_tokens}'",);

        // Validate quotes - count unescaped quotes (not preceded by backslash)
        let unescaped_quote_count = Self::count_unescaped_quotes(other_tokens);
        if unescaped_quote_count % 2 != 0 {
            return Err(SshParserError::InvalidQuotes);
        }

        // if args is quoted, don't split it
        let args = if other_tokens.starts_with('"') && Self::ends_with_unescaped_quote(other_tokens)
        {
            trace!("quoted args: '{other_tokens}'",);
            let content = &other_tokens[1..other_tokens.len() - 1];
            vec![Self::unescape_string(content)]
        } else {
            trace!("splitting args (non-quoted): '{other_tokens}'",);
            // split by whitespace
            let tokens = other_tokens.split_whitespace();

            tokens
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        };

        match Field::from_str(field) {
            Ok(field) => Ok((field, args)),
            Err(_) => Err(SshParserError::UnknownField(field.to_string(), args)),
        }
    }

    // -- value parsers

    /// parse boolean value
    fn parse_boolean(args: Vec<String>) -> SshParserResult<bool> {
        match args.first().map(|x| x.as_str()) {
            Some("yes") => Ok(true),
            Some("no") => Ok(false),
            Some(_) => Err(SshParserError::ExpectedBoolean),
            None => Err(SshParserError::MissingArgument),
        }
    }

    /// Parse algorithms argument
    fn parse_algos(args: Vec<String>) -> SshParserResult<AlgorithmsRule> {
        let first = args.first().ok_or(SshParserError::MissingArgument)?;

        AlgorithmsRule::from_str(first)
    }

    /// Parse comma separated list arguments
    fn parse_comma_separated_list(args: Vec<String>) -> SshParserResult<Vec<String>> {
        match args
            .first()
            .map(|x| x.split(',').map(|x| x.to_string()).collect())
        {
            Some(args) => Ok(args),
            _ => Err(SshParserError::MissingArgument),
        }
    }

    /// Parse duration argument
    fn parse_duration(args: Vec<String>) -> SshParserResult<Duration> {
        let value = Self::parse_unsigned(args)?;
        Ok(Duration::from_secs(value as u64))
    }

    /// Parse host argument.
    /// A leading `!` indicates a negated pattern. Any `!` characters after the first position
    /// are treated as literal characters in the pattern.
    fn parse_host(args: Vec<String>) -> SshParserResult<Vec<HostClause>> {
        if args.is_empty() {
            return Err(SshParserError::MissingArgument);
        }
        // Collect hosts
        Ok(args
            .into_iter()
            .map(|x| {
                if let Some(pattern) = x.strip_prefix('!') {
                    HostClause::new(pattern.to_string(), true)
                } else {
                    HostClause::new(x, false)
                }
            })
            .collect())
    }

    /// Parse a list of paths
    fn parse_path_list(args: Vec<String>) -> SshParserResult<Vec<PathBuf>> {
        if args.is_empty() {
            return Err(SshParserError::MissingArgument);
        }
        args.iter()
            .map(|x| Self::parse_path_arg(x.as_str()))
            .collect()
    }

    /// Parse path argument
    fn parse_path(args: Vec<String>) -> SshParserResult<PathBuf> {
        if let Some(s) = args.first() {
            Self::parse_path_arg(s)
        } else {
            Err(SshParserError::MissingArgument)
        }
    }

    /// Parse path argument
    fn parse_path_arg(s: &str) -> SshParserResult<PathBuf> {
        // Remove tilde
        let s = if s.starts_with('~') {
            let home_dir = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .to_string_lossy()
                .to_string();
            s.replacen('~', &home_dir, 1)
        } else {
            s.to_string()
        };
        Ok(PathBuf::from(s))
    }

    /// Parse port number argument
    fn parse_port(args: Vec<String>) -> SshParserResult<u16> {
        match args.first().map(|x| u16::from_str(x)) {
            Some(Ok(val)) => Ok(val),
            Some(Err(_)) => Err(SshParserError::ExpectedPort),
            None => Err(SshParserError::MissingArgument),
        }
    }

    /// Parse string argument
    fn parse_string(args: Vec<String>) -> SshParserResult<String> {
        if let Some(s) = args.into_iter().next() {
            Ok(s)
        } else {
            Err(SshParserError::MissingArgument)
        }
    }

    /// Parse unsigned argument
    fn parse_unsigned(args: Vec<String>) -> SshParserResult<usize> {
        match args.first().map(|x| usize::from_str(x)) {
            Some(Ok(val)) => Ok(val),
            Some(Err(_)) => Err(SshParserError::ExpectedUnsigned),
            None => Err(SshParserError::MissingArgument),
        }
    }
}

#[cfg(test)]
mod tests {

    use std::fs::File;
    use std::io::{BufReader, Write};
    use std::path::Path;

    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;

    use super::*;
    use crate::DefaultAlgorithms;

    #[test]
    fn should_parse_configuration() -> Result<(), SshParserError> {
        crate::test_log();
        let temp = create_ssh_config();
        let file = File::open(temp.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms {
                ca_signature_algorithms: vec![],
                ciphers: vec![],
                host_key_algorithms: vec![],
                kex_algorithms: vec![],
                mac: vec![],
                pubkey_accepted_algorithms: vec!["omar-crypt".to_string()],
            })
            .parse(&mut reader, ParseRule::STRICT)?;

        // Query openssh cmdline overrides (options preceding the first `Host` section,
        // overriding all following options)
        let params = config.query("*");
        assert_eq!(
            params.ignore_unknown.as_deref().unwrap(),
            &["Pippo", "Pluto"]
        );
        assert_eq!(params.compression.unwrap(), true);
        assert_eq!(params.connection_attempts.unwrap(), 10);
        assert_eq!(params.connect_timeout.unwrap(), Duration::from_secs(60));
        assert_eq!(
            params.server_alive_interval.unwrap(),
            Duration::from_secs(40)
        );
        assert_eq!(params.tcp_keep_alive.unwrap(), true);
        assert_eq!(params.ciphers.algorithms(), &["a-manella", "blowfish"]);
        assert_eq!(
            params.pubkey_accepted_algorithms.algorithms(),
            &["desu", "omar-crypt", "fast-omar-crypt"]
        );

        // Query explicit all-hosts fallback options (`Host *`)
        assert_eq!(params.ca_signature_algorithms.algorithms(), &["random"]);
        assert_eq!(
            params.host_key_algorithms.algorithms(),
            &["luigi", "mario",]
        );
        assert_eq!(params.kex_algorithms.algorithms(), &["desu", "gigi",]);
        assert_eq!(params.mac.algorithms(), &["concorde"]);
        assert!(params.bind_address.is_none());

        // Query 172.26.104.4, yielding cmdline overrides,
        // explicit `Host 192.168.*.* 172.26.*.* !192.168.1.30` options,
        // and all-hosts fallback options.
        let params_172_26_104_4 = config.query("172.26.104.4");

        // cmdline overrides
        assert_eq!(params_172_26_104_4.add_keys_to_agent.unwrap(), true);
        assert_eq!(params_172_26_104_4.compression.unwrap(), true);
        assert_eq!(params_172_26_104_4.connection_attempts.unwrap(), 10);
        assert_eq!(
            params_172_26_104_4.connect_timeout.unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(params_172_26_104_4.tcp_keep_alive.unwrap(), true);

        // all-hosts fallback options, merged with host-specific options
        assert_eq!(
            params_172_26_104_4.ca_signature_algorithms.algorithms(),
            &["random"]
        );
        assert_eq!(
            params_172_26_104_4.ciphers.algorithms(),
            &["a-manella", "blowfish",]
        );
        assert_eq!(params_172_26_104_4.mac.algorithms(), &["spyro", "deoxys"]); // use subconfig; defined before * macs
        assert_eq!(
            params_172_26_104_4.proxy_jump.unwrap(),
            &["jump.example.com"]
        ); // use subconfig; defined before * macs
        assert_eq!(
            params_172_26_104_4
                .pubkey_accepted_algorithms
                .algorithms()
                .is_empty(), // should have removed omar-crypt
            true
        );
        assert_eq!(
            params_172_26_104_4.bind_address.as_deref().unwrap(),
            "10.8.0.10"
        );
        assert_eq!(
            params_172_26_104_4.bind_interface.as_deref().unwrap(),
            "tun0"
        );
        assert_eq!(params_172_26_104_4.port.unwrap(), 2222);
        assert_eq!(
            params_172_26_104_4.identity_file.as_deref().unwrap(),
            vec![
                Path::new("/home/root/.ssh/pippo.key"),
                Path::new("/home/root/.ssh/pluto.key")
            ]
        );
        assert_eq!(params_172_26_104_4.user.as_deref().unwrap(), "omar");

        // Query tostapane
        let params_tostapane = config.query("tostapane");
        assert_eq!(params_tostapane.compression.unwrap(), true); // it takes the first value defined, which is `yes`
        assert_eq!(params_tostapane.connection_attempts.unwrap(), 10);
        assert_eq!(
            params_tostapane.connect_timeout.unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(params_tostapane.tcp_keep_alive.unwrap(), true);
        assert_eq!(params_tostapane.remote_forward.unwrap(), 88);
        assert_eq!(params_tostapane.user.as_deref().unwrap(), "ciro-esposito");

        // all-hosts fallback options
        assert_eq!(
            params_tostapane.ca_signature_algorithms.algorithms(),
            &["random"]
        );
        assert_eq!(
            params_tostapane.ciphers.algorithms(),
            &["a-manella", "blowfish",]
        );
        assert_eq!(
            params_tostapane.mac.algorithms(),
            vec!["spyro".to_string(), "deoxys".to_string(),]
        );
        assert_eq!(
            params_tostapane.proxy_jump.unwrap(),
            vec![
                "jump1.example.com".to_string(),
                "jump2.example.com".to_string(),
            ]
        );
        assert_eq!(
            params_tostapane.pubkey_accepted_algorithms.algorithms(),
            &["desu", "omar-crypt", "fast-omar-crypt"]
        );

        // query 192.168.1.30
        let params_192_168_1_30 = config.query("192.168.1.30");

        // host-specific options
        assert_eq!(params_192_168_1_30.user.as_deref().unwrap(), "nutellaro");
        assert_eq!(params_192_168_1_30.remote_forward.unwrap(), 123);

        // cmdline overrides
        assert_eq!(params_192_168_1_30.compression.unwrap(), true);
        assert_eq!(params_192_168_1_30.connection_attempts.unwrap(), 10);
        assert_eq!(
            params_192_168_1_30.connect_timeout.unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(params_192_168_1_30.tcp_keep_alive.unwrap(), true);

        // all-hosts fallback options
        assert_eq!(
            params_192_168_1_30.ca_signature_algorithms.algorithms(),
            &["random"]
        );
        assert_eq!(
            params_192_168_1_30.ciphers.algorithms(),
            &["a-manella", "blowfish"]
        );
        assert_eq!(params_192_168_1_30.mac.algorithms(), &["concorde"]);
        assert_eq!(
            params_192_168_1_30.pubkey_accepted_algorithms.algorithms(),
            &["desu", "omar-crypt", "fast-omar-crypt"]
        );

        Ok(())
    }

    #[test]
    fn should_allow_unknown_field() -> Result<(), SshParserError> {
        crate::test_log();
        let temp = create_ssh_config_with_unknown_fields();
        let file = File::open(temp.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);
        let _config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::ALLOW_UNKNOWN_FIELDS)?;

        Ok(())
    }

    #[test]
    fn should_not_allow_unknown_field() {
        crate::test_log();
        let temp = create_ssh_config_with_unknown_fields();
        let file = File::open(temp.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);
        assert!(matches!(
            SshConfig::default()
                .default_algorithms(DefaultAlgorithms::empty())
                .parse(&mut reader, ParseRule::STRICT)
                .unwrap_err(),
            SshParserError::UnknownField(..)
        ));
    }

    #[test]
    fn should_store_unknown_fields() {
        crate::test_log();
        let temp = create_ssh_config_with_unknown_fields();
        let file = File::open(temp.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::ALLOW_UNKNOWN_FIELDS)
            .unwrap();

        let host = config.query("cross-platform");
        assert_eq!(
            host.ignored_fields.get("Piropero").unwrap(),
            &vec![String::from("yes")]
        );
    }

    #[test]
    fn should_parse_inversed_ssh_config() {
        crate::test_log();
        let temp = create_inverted_ssh_config();
        let file = File::open(temp.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::STRICT)
            .unwrap();

        let home_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .to_string_lossy()
            .to_string();

        let remote_host = config.query("remote-host");

        // From `*-host`
        assert_eq!(
            remote_host.identity_file.unwrap()[0].as_path(),
            Path::new(format!("{home_dir}/.ssh/id_rsa_good").as_str()) // because it's the first in the file
        );

        // From `remote-*`
        assert_eq!(remote_host.host_name.unwrap(), "hostname.com");
        assert_eq!(remote_host.user.unwrap(), "user");

        // From `*`
        assert_eq!(
            remote_host.connect_timeout.unwrap(),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn should_parse_configuration_with_hosts() {
        crate::test_log();
        let temp = create_ssh_config_with_comments();

        let file = File::open(temp.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::STRICT)
            .unwrap();

        let hostname = config.query("cross-platform").host_name.unwrap();
        assert_eq!(&hostname, "hostname.com");

        assert!(config.query("this").host_name.is_none());
    }

    #[test]
    fn should_update_host_bind_address() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::BindAddress,
            vec![String::from("127.0.0.1")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.bind_address.as_deref().unwrap(), "127.0.0.1");
        Ok(())
    }

    #[test]
    fn should_update_host_bind_interface() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::BindInterface,
            vec![String::from("aaa")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.bind_interface.as_deref().unwrap(), "aaa");
        Ok(())
    }

    #[test]
    fn should_update_host_ca_signature_algos() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::CaSignatureAlgorithms,
            vec![String::from("a,b,c")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(
            host.params.ca_signature_algorithms.algorithms(),
            &["a", "b", "c"]
        );
        Ok(())
    }

    #[test]
    fn should_update_host_certificate_file() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::CertificateFile,
            vec![String::from("/tmp/a.crt")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(
            host.params.certificate_file.as_deref().unwrap(),
            Path::new("/tmp/a.crt")
        );
        Ok(())
    }

    #[test]
    fn should_update_host_ciphers() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::Ciphers,
            vec![String::from("a,b,c")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.ciphers.algorithms(), &["a", "b", "c"]);
        Ok(())
    }

    #[test]
    fn should_update_host_compression() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::Compression,
            vec![String::from("yes")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.compression.unwrap(), true);
        Ok(())
    }

    #[test]
    fn should_update_host_connection_attempts() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::ConnectionAttempts,
            vec![String::from("4")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.connection_attempts.unwrap(), 4);
        Ok(())
    }

    #[test]
    fn should_update_host_connection_timeout() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::ConnectTimeout,
            vec![String::from("10")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(
            host.params.connect_timeout.unwrap(),
            Duration::from_secs(10)
        );
        Ok(())
    }

    #[test]
    fn should_update_host_key_algorithms() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::HostKeyAlgorithms,
            vec![String::from("a,b,c")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(
            host.params.host_key_algorithms.algorithms(),
            &["a", "b", "c"]
        );
        Ok(())
    }

    #[test]
    fn should_update_host_host_name() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::HostName,
            vec![String::from("192.168.1.1")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.host_name.as_deref().unwrap(), "192.168.1.1");
        Ok(())
    }

    #[test]
    fn should_update_host_ignore_unknown() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::IgnoreUnknown,
            vec![String::from("a,b,c")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(
            host.params.ignore_unknown.as_deref().unwrap(),
            &["a", "b", "c"]
        );
        Ok(())
    }

    #[test]
    fn should_update_kex_algorithms() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::KexAlgorithms,
            vec![String::from("a,b,c")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.kex_algorithms.algorithms(), &["a", "b", "c"]);
        Ok(())
    }

    #[test]
    fn should_update_host_mac() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::Mac,
            vec![String::from("a,b,c")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.mac.algorithms(), &["a", "b", "c"]);
        Ok(())
    }

    #[test]
    fn should_update_host_port() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::Port,
            vec![String::from("2222")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.port.unwrap(), 2222);
        Ok(())
    }

    #[test]
    fn should_update_host_pubkey_accepted_algos() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::PubkeyAcceptedAlgorithms,
            vec![String::from("a,b,c")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(
            host.params.pubkey_accepted_algorithms.algorithms(),
            &["a", "b", "c"]
        );
        Ok(())
    }

    #[test]
    fn should_update_host_pubkey_authentication() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::PubkeyAuthentication,
            vec![String::from("yes")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.pubkey_authentication.unwrap(), true);
        Ok(())
    }

    #[test]
    fn should_update_host_remote_forward() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::RemoteForward,
            vec![String::from("3005")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.remote_forward.unwrap(), 3005);
        Ok(())
    }

    #[test]
    fn should_update_host_server_alive_interval() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::ServerAliveInterval,
            vec![String::from("40")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(
            host.params.server_alive_interval.unwrap(),
            Duration::from_secs(40)
        );
        Ok(())
    }

    #[test]
    fn should_update_host_tcp_keep_alive() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::TcpKeepAlive,
            vec![String::from("no")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.tcp_keep_alive.unwrap(), false);
        Ok(())
    }

    #[test]
    fn should_update_host_user() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::User,
            vec![String::from("pippo")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        )?;
        assert_eq!(host.params.user.as_deref().unwrap(), "pippo");
        Ok(())
    }

    #[test]
    fn should_not_update_host_if_unknown() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        let result = SshConfigParser::update_host(
            Field::PasswordAuthentication,
            vec![String::from("yes")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        );

        match result {
            Ok(_) | Err(SshParserError::UnsupportedField(_, _)) => Ok(()),
            Err(e) => Err(e),
        }?;

        assert_eq!(host.params, HostParams::new(&DefaultAlgorithms::empty()));
        Ok(())
    }

    #[test]
    fn should_update_host_if_unsupported() -> Result<(), SshParserError> {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        let result = SshConfigParser::update_host(
            Field::PasswordAuthentication,
            vec![String::from("yes")],
            &mut host,
            ParseRule::ALLOW_UNKNOWN_FIELDS,
            &DefaultAlgorithms::empty(),
        );

        match result {
            Err(SshParserError::UnsupportedField(field, _)) => {
                assert_eq!(field, "passwordauthentication");
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }?;

        assert_eq!(host.params, HostParams::new(&DefaultAlgorithms::empty()));
        Ok(())
    }

    #[test]
    fn should_tokenize_line() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::tokenize_line("HostName 192.168.*.* 172.26.*.*")?,
            (
                Field::HostName,
                vec![String::from("192.168.*.*"), String::from("172.26.*.*")]
            )
        );
        // Tokenize line with spaces
        assert_eq!(
            SshConfigParser::tokenize_line(
                "      HostName        192.168.*.*        172.26.*.*        "
            )?,
            (
                Field::HostName,
                vec![String::from("192.168.*.*"), String::from("172.26.*.*")]
            )
        );
        Ok(())
    }

    #[test]
    fn should_not_tokenize_line() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::tokenize_line("Omar     yes").unwrap_err(),
            SshParserError::UnknownField(..)
        ));
    }

    #[test]
    fn should_fail_parsing_field() {
        crate::test_log();

        assert!(matches!(
            SshConfigParser::tokenize_line("                  ").unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_fail_on_mismatched_quotes() {
        crate::test_log();

        // Unclosed opening quote
        assert!(matches!(
            SshConfigParser::tokenize_line(r#"Hostname "example.com"#).unwrap_err(),
            SshParserError::InvalidQuotes
        ));
        // Unexpected closing quote (no opening)
        assert!(matches!(
            SshConfigParser::tokenize_line(r#"Hostname example.com""#).unwrap_err(),
            SshParserError::InvalidQuotes
        ));
        // Quote in middle, unclosed
        assert!(matches!(
            SshConfigParser::tokenize_line(r#"Hostname foo "bar"#).unwrap_err(),
            SshParserError::InvalidQuotes
        ));
    }

    #[test]
    fn should_parse_boolean() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_boolean(vec![String::from("yes")])?,
            true
        );
        assert_eq!(
            SshConfigParser::parse_boolean(vec![String::from("no")])?,
            false
        );
        Ok(())
    }

    #[test]
    fn should_fail_parsing_boolean() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_boolean(vec!["boh".to_string()]).unwrap_err(),
            SshParserError::ExpectedBoolean
        ));
        assert!(matches!(
            SshConfigParser::parse_boolean(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_algos() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_algos(vec![String::from("a,b,c,d")])?,
            AlgorithmsRule::Set(vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
            ])
        );

        assert_eq!(
            SshConfigParser::parse_algos(vec![String::from("a")])?,
            AlgorithmsRule::Set(vec!["a".to_string()])
        );

        assert_eq!(
            SshConfigParser::parse_algos(vec![String::from("+a,b")])?,
            AlgorithmsRule::Append(vec!["a".to_string(), "b".to_string()])
        );

        Ok(())
    }

    #[test]
    fn should_parse_comma_separated_list() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_comma_separated_list(vec![String::from("a,b,c,d")])?,
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
            ]
        );
        assert_eq!(
            SshConfigParser::parse_comma_separated_list(vec![String::from("a")])?,
            vec!["a".to_string()]
        );
        Ok(())
    }

    #[test]
    fn should_fail_parsing_comma_separated_list() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_comma_separated_list(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_duration() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_duration(vec![String::from("60")])?,
            Duration::from_secs(60)
        );
        Ok(())
    }

    #[test]
    fn should_fail_parsing_duration() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_duration(vec![String::from("AAA")]).unwrap_err(),
            SshParserError::ExpectedUnsigned
        ));
        assert!(matches!(
            SshConfigParser::parse_duration(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_host() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_host(vec![
                String::from("192.168.*.*"),
                String::from("!192.168.1.1"),
                String::from("172.26.104.*"),
                String::from("!172.26.104.10"),
            ])?,
            vec![
                HostClause::new(String::from("192.168.*.*"), false),
                HostClause::new(String::from("192.168.1.1"), true),
                HostClause::new(String::from("172.26.104.*"), false),
                HostClause::new(String::from("172.26.104.10"), true),
            ]
        );
        Ok(())
    }

    #[test]
    fn should_fail_parsing_host() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_host(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_path() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_path(vec![String::from("/tmp/a.txt")])?,
            PathBuf::from("/tmp/a.txt")
        );
        Ok(())
    }

    #[test]
    fn should_parse_path_and_resolve_tilde() -> Result<(), SshParserError> {
        crate::test_log();
        let mut expected = dirs::home_dir().unwrap();
        expected.push(".ssh/id_dsa");
        assert_eq!(
            SshConfigParser::parse_path(vec![String::from("~/.ssh/id_dsa")])?,
            expected
        );
        Ok(())
    }

    #[test]
    fn should_parse_path_list() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_path_list(vec![
                String::from("/tmp/a.txt"),
                String::from("/tmp/b.txt")
            ])?,
            vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")]
        );
        Ok(())
    }

    #[test]
    fn should_fail_parse_path_list() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_path_list(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_fail_parsing_path() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_path(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_port() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(SshConfigParser::parse_port(vec![String::from("22")])?, 22);
        Ok(())
    }

    #[test]
    fn should_fail_parsing_port() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_port(vec![String::from("1234567")]).unwrap_err(),
            SshParserError::ExpectedPort
        ));
        assert!(matches!(
            SshConfigParser::parse_port(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_string() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_string(vec![String::from("foobar")])?,
            String::from("foobar")
        );
        Ok(())
    }

    #[test]
    fn should_fail_parsing_string() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_string(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_unsigned() -> Result<(), SshParserError> {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_unsigned(vec![String::from("43")])?,
            43
        );
        Ok(())
    }

    #[test]
    fn should_fail_parsing_unsigned() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_unsigned(vec![String::from("abc")]).unwrap_err(),
            SshParserError::ExpectedUnsigned
        ));
        assert!(matches!(
            SshConfigParser::parse_unsigned(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_strip_comments() {
        crate::test_log();

        assert_eq!(
            SshConfigParser::strip_comments("host my_host # this is my fav host").as_str(),
            "host my_host "
        );
        assert_eq!(
            SshConfigParser::strip_comments("# this is a comment").as_str(),
            ""
        );
    }

    #[test]
    fn should_preserve_hash_inside_quoted_strings() {
        crate::test_log();

        // Hash inside quotes should NOT be treated as a comment
        assert_eq!(
            SshConfigParser::strip_comments(r#"Ciphers "aes256-ctr # not a comment""#).as_str(),
            r#"Ciphers "aes256-ctr # not a comment""#
        );
        // Hash after closing quote should be treated as a comment
        assert_eq!(
            SshConfigParser::strip_comments(r#"Ciphers "aes256-ctr" # this is a comment"#).as_str(),
            r#"Ciphers "aes256-ctr" "#
        );
        // Multiple quoted sections
        assert_eq!(
            SshConfigParser::strip_comments(r#"ProxyCommand "ssh # hop" -W "dest # host""#)
                .as_str(),
            r#"ProxyCommand "ssh # hop" -W "dest # host""#
        );
        // Comment after multiple quoted sections
        assert_eq!(
            SshConfigParser::strip_comments(r#"Key "val1" "val2" # comment"#).as_str(),
            r#"Key "val1" "val2" "#
        );
    }

    #[test]
    fn test_should_parse_config_with_quotes_and_eq() {
        crate::test_log();

        let config = create_ssh_config_with_quotes_and_eq();
        let file = File::open(config.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);

        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::STRICT)
            .expect("Failed to parse config");

        let params = config.query("foo");

        // connect timeout is 15
        assert_eq!(
            params.connect_timeout.expect("unspec connect timeout"),
            Duration::from_secs(15)
        );
        assert_eq!(
            params
                .ignore_unknown
                .as_deref()
                .expect("unspec ignore unknown"),
            &["Pippo", "Pluto"]
        );
        assert_eq!(
            params
                .ciphers
                .algorithms()
                .iter()
                .map(|x| x.as_str())
                .collect::<Vec<&str>>(),
            &["Pepperoni Pizza", "Margherita Pizza", "Hawaiian Pizza"]
        );
        assert_eq!(
            params
                .mac
                .algorithms()
                .iter()
                .map(|x| x.as_str())
                .collect::<Vec<&str>>(),
            &["Pasta Carbonara", "Pasta con tonno"]
        );
    }

    #[test]
    fn test_should_resolve_absolute_include_path() {
        crate::test_log();

        let expected = PathBuf::from("/tmp/config.local");

        let s = "/tmp/config.local";
        let resolved = PathBuf::from(SshConfigParser::resolve_include_path(s));
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_should_resolve_relative_include_path() {
        crate::test_log();

        let expected = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(".ssh")
            .join("config.local");

        let s = "config.local";
        let resolved = PathBuf::from(SshConfigParser::resolve_include_path(s));
        assert_eq!(resolved, expected);
    }

    #[test]
    fn test_should_resolve_include_path_with_tilde() {
        let p = "~/.ssh/config.local";
        let resolved = SshConfigParser::resolve_include_path(p);
        let mut expected = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
        expected.push(".ssh");
        expected.push("config.local");
        assert_eq!(PathBuf::from(resolved), expected);
    }

    #[test]
    fn should_fail_parsing_algos_missing_arg() {
        crate::test_log();
        assert!(matches!(
            SshConfigParser::parse_algos(vec![]).unwrap_err(),
            SshParserError::MissingArgument
        ));
    }

    #[test]
    fn should_parse_duration_zero() {
        crate::test_log();
        assert_eq!(
            SshConfigParser::parse_duration(vec![String::from("0")]).unwrap(),
            Duration::from_secs(0)
        );
    }

    #[test]
    fn should_parse_port_boundary() {
        crate::test_log();
        // Minimum valid port
        assert_eq!(
            SshConfigParser::parse_port(vec![String::from("1")]).unwrap(),
            1
        );
        // Maximum valid port
        assert_eq!(
            SshConfigParser::parse_port(vec![String::from("65535")]).unwrap(),
            65535
        );
    }

    #[test]
    fn should_update_host_add_keys_to_agent() {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::AddKeysToAgent,
            vec![String::from("yes")],
            &mut host,
            ParseRule::STRICT,
            &DefaultAlgorithms::empty(),
        )
        .unwrap();
        assert_eq!(host.params.add_keys_to_agent.unwrap(), true);

        let mut host2 = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::AddKeysToAgent,
            vec![String::from("no")],
            &mut host2,
            ParseRule::STRICT,
            &DefaultAlgorithms::empty(),
        )
        .unwrap();
        assert_eq!(host2.params.add_keys_to_agent.unwrap(), false);
    }

    #[test]
    fn should_update_host_forward_agent() {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::ForwardAgent,
            vec![String::from("yes")],
            &mut host,
            ParseRule::STRICT,
            &DefaultAlgorithms::empty(),
        )
        .unwrap();
        assert_eq!(host.params.forward_agent.unwrap(), true);
    }

    #[test]
    fn should_update_host_proxy_jump() {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::ProxyJump,
            vec![String::from("jump1,jump2,jump3")],
            &mut host,
            ParseRule::STRICT,
            &DefaultAlgorithms::empty(),
        )
        .unwrap();
        assert_eq!(
            host.params.proxy_jump.unwrap(),
            vec![
                "jump1".to_string(),
                "jump2".to_string(),
                "jump3".to_string()
            ]
        );
    }

    #[test]
    fn should_update_host_identity_file() {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::IdentityFile,
            vec![String::from("/path/to/key1"), String::from("/path/to/key2")],
            &mut host,
            ParseRule::STRICT,
            &DefaultAlgorithms::empty(),
        )
        .unwrap();
        assert_eq!(
            host.params.identity_file.unwrap(),
            vec![
                PathBuf::from("/path/to/key1"),
                PathBuf::from("/path/to/key2")
            ]
        );
    }

    #[test]
    fn test_should_allow_and_append_multiple_identity_files_directives() {
        crate::test_log();
        let config = r##"
Host test
    IdentityFile /path/to/key1 /path/to/key2
    IdentityFile /path/to/key3
"##;
        let mut reader = BufReader::new(config.as_bytes());
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::STRICT)
            .expect("Failed to parse config");

        let params = config.query("test");
        assert_eq!(
            params.identity_file.as_ref().unwrap(),
            &vec![
                PathBuf::from("/path/to/key1"),
                PathBuf::from("/path/to/key2"),
                PathBuf::from("/path/to/key3"),
            ]
        );
    }

    #[test]
    fn test_should_accumulate_identity_files_across_host_blocks() {
        crate::test_log();
        let config = r##"
Host test
    IdentityFile /path/to/specific_key

Host *
    IdentityFile /path/to/default_key
"##;
        let mut reader = BufReader::new(config.as_bytes());
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::STRICT)
            .expect("Failed to parse config");

        let params = config.query("test");
        // Both identity files should be present: specific first, then default
        assert_eq!(
            params.identity_file.as_ref().unwrap(),
            &vec![
                PathBuf::from("/path/to/specific_key"),
                PathBuf::from("/path/to/default_key"),
            ]
        );
    }

    #[test]
    fn should_store_unsupported_fields_when_allowed() {
        crate::test_log();

        let config = r##"
Host test
    PasswordAuthentication yes
"##;
        let mut reader = BufReader::new(config.as_bytes());
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS)
            .unwrap();

        let params = config.query("test");
        assert!(
            params
                .unsupported_fields
                .contains_key("passwordauthentication")
        );
    }

    #[test]
    fn should_tokenize_line_with_equals_separator() {
        crate::test_log();
        let (field, args) = SshConfigParser::tokenize_line("HostName=example.com").unwrap();
        assert_eq!(field, Field::HostName);
        assert_eq!(args, vec!["example.com".to_string()]);
    }

    #[test]
    fn should_tokenize_line_with_quoted_args() {
        crate::test_log();
        let (field, args) =
            SshConfigParser::tokenize_line("Ciphers \"aes256-ctr,aes128-ctr\"").unwrap();
        assert_eq!(field, Field::Ciphers);
        assert_eq!(args, vec!["aes256-ctr,aes128-ctr".to_string()]);
    }

    #[test]
    fn should_tokenize_line_with_equals_and_quoted_args() {
        crate::test_log();
        let (field, args) =
            SshConfigParser::tokenize_line("Ciphers=\"aes256-ctr,aes128-ctr\"").unwrap();
        assert_eq!(field, Field::Ciphers);
        assert_eq!(args, vec!["aes256-ctr,aes128-ctr".to_string()]);
    }

    #[test]
    fn should_unescape_quoted_args() {
        crate::test_log();

        // Test escaped double quote: \" -> "
        let (field, args) =
            SshConfigParser::tokenize_line(r#"HostName "gateway\"server""#).unwrap();
        assert_eq!(field, Field::HostName);
        assert_eq!(args, vec![r#"gateway"server"#.to_string()]);

        // Test escaped backslash: \\ -> \
        let (field, args) = SshConfigParser::tokenize_line(r#"HostName "path\\to\\host""#).unwrap();
        assert_eq!(field, Field::HostName);
        assert_eq!(args, vec![r#"path\to\host"#.to_string()]);

        // Test escaped single quote: \' -> '
        let (field, args) = SshConfigParser::tokenize_line(r#"HostName "it\'s a test""#).unwrap();
        assert_eq!(field, Field::HostName);
        assert_eq!(args, vec!["it's a test".to_string()]);

        // Test multiple escape sequences combined
        let (field, args) =
            SshConfigParser::tokenize_line(r#"HostName "say \"hello\" and \\go""#).unwrap();
        assert_eq!(field, Field::HostName);
        assert_eq!(args, vec![r#"say "hello" and \go"#.to_string()]);

        // Test unrecognized escape sequence (backslash preserved)
        let (field, args) = SshConfigParser::tokenize_line(r#"HostName "test\nvalue""#).unwrap();
        assert_eq!(field, Field::HostName);
        assert_eq!(args, vec![r#"test\nvalue"#.to_string()]);
    }

    #[test]
    fn should_count_unescaped_quotes() {
        crate::test_log();

        // No quotes
        assert_eq!(SshConfigParser::count_unescaped_quotes("hello"), 0);

        // Simple unescaped quotes
        assert_eq!(SshConfigParser::count_unescaped_quotes(r#""hello""#), 2);

        // Escaped quotes should not be counted
        assert_eq!(SshConfigParser::count_unescaped_quotes(r#"\"hello\""#), 0);

        // Mixed escaped and unescaped
        assert_eq!(
            SshConfigParser::count_unescaped_quotes(r#""hello\"world""#),
            2
        );

        // Escaped backslash before quote (quote is unescaped)
        assert_eq!(SshConfigParser::count_unescaped_quotes(r#"\\""#), 1);

        // Empty string
        assert_eq!(SshConfigParser::count_unescaped_quotes(""), 0);

        // Only escaped quote
        assert_eq!(SshConfigParser::count_unescaped_quotes(r#"\""#), 0);
    }

    #[test]
    fn should_detect_ends_with_unescaped_quote() {
        crate::test_log();

        // Ends with unescaped quote
        assert!(SshConfigParser::ends_with_unescaped_quote(r#""hello""#));

        // Ends with escaped quote (odd backslashes)
        assert!(!SshConfigParser::ends_with_unescaped_quote(r#""hello\""#));

        // Ends with escaped backslash then unescaped quote
        assert!(SshConfigParser::ends_with_unescaped_quote(r#""hello\\""#));

        // Ends with three backslashes then quote (escaped)
        assert!(!SshConfigParser::ends_with_unescaped_quote(r#""hello\\\""#));

        // Doesn't end with quote at all
        assert!(!SshConfigParser::ends_with_unescaped_quote("hello"));

        // Single quote
        assert!(SshConfigParser::ends_with_unescaped_quote(r#"""#));

        // Single escaped quote
        assert!(!SshConfigParser::ends_with_unescaped_quote(r#"\""#));
    }

    #[test]
    fn should_unescape_string() {
        crate::test_log();

        // Escaped double quote
        assert_eq!(
            SshConfigParser::unescape_string(r#"hello\"world"#),
            r#"hello"world"#
        );

        // Escaped backslash
        assert_eq!(
            SshConfigParser::unescape_string(r#"path\\to\\file"#),
            r#"path\to\file"#
        );

        // Escaped single quote
        assert_eq!(SshConfigParser::unescape_string(r#"it\'s"#), "it's");

        // Multiple escape sequences
        assert_eq!(
            SshConfigParser::unescape_string(r#"say \"hi\" and \\go"#),
            r#"say "hi" and \go"#
        );

        // Unrecognized escape (backslash preserved)
        assert_eq!(
            SshConfigParser::unescape_string(r#"test\nvalue"#),
            r#"test\nvalue"#
        );

        // No escapes
        assert_eq!(SshConfigParser::unescape_string("plain text"), "plain text");

        // Empty string
        assert_eq!(SshConfigParser::unescape_string(""), "");

        // Trailing backslash (no char to escape)
        assert_eq!(SshConfigParser::unescape_string(r#"test\"#), r#"test\"#);

        // Double escaped backslash
        assert_eq!(SshConfigParser::unescape_string(r#"\\\\"#), r#"\\"#);
    }

    #[test]
    fn should_parse_host_with_single_pattern() {
        crate::test_log();
        let result = SshConfigParser::parse_host(vec![String::from("example.com")]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pattern, "example.com");
        assert!(!result[0].negated);
    }

    #[test]
    fn should_parse_host_with_exclamation_in_pattern() {
        crate::test_log();

        // Pattern with ! in the middle should be treated as literal (non-negated)
        let result = SshConfigParser::parse_host(vec![String::from("host!name")]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pattern, "host!name");
        assert!(!result[0].negated);

        // Negated pattern with ! in the pattern itself
        let result = SshConfigParser::parse_host(vec![String::from("!host!name")]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pattern, "host!name");
        assert!(result[0].negated);

        // Multiple ! after the negation prefix should be preserved
        let result = SshConfigParser::parse_host(vec![String::from("!a!b!c")]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pattern, "a!b!c");
        assert!(result[0].negated);

        // Only leading ! is negation, rest is literal
        let result = SshConfigParser::parse_host(vec![String::from("a!b")]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pattern, "a!b");
        assert!(!result[0].negated);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn should_update_host_use_keychain() {
        crate::test_log();
        let mut host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::empty()));
        SshConfigParser::update_host(
            Field::UseKeychain,
            vec![String::from("yes")],
            &mut host,
            ParseRule::STRICT,
            &DefaultAlgorithms::empty(),
        )
        .unwrap();
        assert_eq!(host.params.use_keychain.unwrap(), true);
    }

    fn create_ssh_config_with_quotes_and_eq() -> NamedTempFile {
        let mut tmpfile: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let config = r##"
# ssh config
# written by veeso


# I put a comment here just to annoy

IgnoreUnknown=Pippo,Pluto
ConnectTimeout = 15
Ciphers "Pepperoni Pizza,Margherita Pizza,Hawaiian Pizza"
Macs="Pasta Carbonara,Pasta con tonno"
"##;
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile
    }

    fn create_ssh_config() -> NamedTempFile {
        let mut tmpfile: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let config = r##"
# ssh config
# written by veeso


        # I put a comment here just to annoy

IgnoreUnknown Pippo,Pluto

Compression yes
ConnectionAttempts          10
ConnectTimeout 60
ServerAliveInterval 40
TcpKeepAlive    yes
Ciphers     +a-manella,blowfish

# Let's start defining some hosts

Host 192.168.*.*    172.26.*.*      !192.168.1.30
    User    omar
    # ForwardX11 is actually not supported; I just want to see that it wont' fail parsing
    ForwardX11    yes
    BindAddress     10.8.0.10
    BindInterface   tun0
    AddKeysToAgent yes
    Ciphers     +coi-piedi,cazdecan,triestin-stretto
    IdentityFile    /home/root/.ssh/pippo.key /home/root/.ssh/pluto.key
    Macs     spyro,deoxys
    Port 2222
    PubkeyAcceptedAlgorithms    -omar-crypt
    ProxyJump jump.example.com

Host tostapane
    User    ciro-esposito
    HostName    192.168.24.32
    RemoteForward   88
    Compression no
    Pippo yes
    Pluto 56
    ProxyJump jump1.example.com,jump2.example.com
    Macs +spyro,deoxys

Host    192.168.1.30
    User    nutellaro
    RemoteForward   123

Host *
    CaSignatureAlgorithms   random
    HostKeyAlgorithms   luigi,mario
    KexAlgorithms   desu,gigi
    Macs     concorde
    PubkeyAcceptedAlgorithms    desu,omar-crypt,fast-omar-crypt
"##;
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile
    }

    fn create_inverted_ssh_config() -> NamedTempFile {
        let mut tmpfile: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let config = r##"
Host *-host
    IdentityFile ~/.ssh/id_rsa_good

Host remote-*
    HostName hostname.com
    User user
    IdentityFile ~/.ssh/id_rsa_bad

Host *
    ConnectTimeout 15
    IdentityFile ~/.ssh/id_rsa_ugly
    "##;
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile
    }

    fn create_ssh_config_with_comments() -> NamedTempFile {
        let mut tmpfile: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let config = r##"
Host cross-platform # this is my fav host
    HostName hostname.com
    User user
    IdentityFile ~/.ssh/id_rsa_good

Host *
    AddKeysToAgent yes
    IdentityFile ~/.ssh/id_rsa_bad
    "##;
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile
    }

    fn create_ssh_config_with_unknown_fields() -> NamedTempFile {
        let mut tmpfile: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let config = r##"
Host cross-platform # this is my fav host
    HostName hostname.com
    User user
    IdentityFile ~/.ssh/id_rsa_good
    Piropero yes

Host *
    AddKeysToAgent yes
    IdentityFile ~/.ssh/id_rsa_bad
    "##;
        tmpfile.write_all(config.as_bytes()).unwrap();
        tmpfile
    }

    #[test]
    fn test_should_parse_config_with_include() {
        crate::test_log();

        let config = create_include_config();
        let file = File::open(config.config.path()).expect("Failed to open tempfile");
        let mut reader = BufReader::new(file);

        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::STRICT)
            .expect("Failed to parse config");

        let default_params = config.query("unknown-host");
        // verify default params
        assert_eq!(
            default_params.connect_timeout.unwrap(),
            Duration::from_secs(60) // first read
        );
        assert_eq!(
            default_params.server_alive_interval.unwrap(),
            Duration::from_secs(40) // first read
        );
        assert_eq!(default_params.tcp_keep_alive.unwrap(), true);
        assert_eq!(default_params.ciphers.algorithms().is_empty(), true);
        assert_eq!(
            default_params.ignore_unknown.as_deref().unwrap(),
            &["Pippo", "Pluto"]
        );
        assert_eq!(default_params.compression.unwrap(), true);
        assert_eq!(default_params.connection_attempts.unwrap(), 10);

        // verify include 1 overwrites the default value
        let glob_params = config.query("192.168.1.1");
        assert_eq!(
            glob_params.connect_timeout.unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(
            glob_params.server_alive_interval.unwrap(),
            Duration::from_secs(40) // first read
        );
        assert_eq!(glob_params.tcp_keep_alive.unwrap(), true);
        assert_eq!(glob_params.ciphers.algorithms().is_empty(), true);

        // verify tostapane
        let tostapane_params = config.query("tostapane");
        assert_eq!(
            tostapane_params.connect_timeout.unwrap(),
            Duration::from_secs(60) // first read
        );
        assert_eq!(
            tostapane_params.server_alive_interval.unwrap(),
            Duration::from_secs(40) // first read
        );
        assert_eq!(tostapane_params.tcp_keep_alive.unwrap(), true);
        // verify ciphers
        assert_eq!(
            tostapane_params.ciphers.algorithms(),
            &[
                "a-manella",
                "blowfish",
                "coi-piedi",
                "cazdecan",
                "triestin-stretto"
            ]
        );

        // verify included host (microwave)
        let microwave_params = config.query("microwave");
        assert_eq!(
            microwave_params.connect_timeout.unwrap(),
            Duration::from_secs(60) // (not) updated in inc4
        );
        assert_eq!(
            microwave_params.server_alive_interval.unwrap(),
            Duration::from_secs(40) // (not) updated in inc4
        );
        assert_eq!(
            microwave_params.port.unwrap(),
            345 // updated in inc4
        );
        assert_eq!(microwave_params.tcp_keep_alive.unwrap(), true);
        assert_eq!(microwave_params.ciphers.algorithms().is_empty(), true);
        assert_eq!(microwave_params.user.as_deref().unwrap(), "mario-rossi");
        assert_eq!(
            microwave_params.host_name.as_deref().unwrap(),
            "192.168.24.33"
        );
        assert_eq!(microwave_params.remote_forward.unwrap(), 88);
        assert_eq!(microwave_params.compression.unwrap(), true);

        // verify included host (fridge)
        let fridge_params = config.query("fridge");
        assert_eq!(
            fridge_params.connect_timeout.unwrap(),
            Duration::from_secs(60)
        ); // default
        assert_eq!(
            fridge_params.server_alive_interval.unwrap(),
            Duration::from_secs(40)
        ); // default
        assert_eq!(fridge_params.tcp_keep_alive.unwrap(), true);
        assert_eq!(fridge_params.ciphers.algorithms().is_empty(), true);
        assert_eq!(fridge_params.user.as_deref().unwrap(), "luigi-verdi");
        assert_eq!(fridge_params.host_name.as_deref().unwrap(), "192.168.24.34");
    }

    #[allow(dead_code)]
    struct ConfigWithInclude {
        config: NamedTempFile,
        inc1: NamedTempFile,
        inc2: NamedTempFile,
        inc3: NamedTempFile,
        inc4: NamedTempFile,
    }

    fn create_include_config() -> ConfigWithInclude {
        let mut config_file: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let mut inc1_file: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let mut inc2_file: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let mut inc3_file: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");
        let mut inc4_file: tempfile::NamedTempFile =
            tempfile::NamedTempFile::new().expect("Failed to create tempfile");

        let config = format!(
            r##"
# ssh config
# written by veeso


        # I put a comment here just to annoy

IgnoreUnknown Pippo,Pluto

Compression yes
ConnectionAttempts          10
ConnectTimeout 60
ServerAliveInterval 40
Include {inc1}

# Let's start defining some hosts

Host tostapane
    User    ciro-esposito
    HostName    192.168.24.32
    RemoteForward   88
    Compression no
    # Ignore unknown fields should be inherited from the global section
    Pippo yes
    Pluto 56
    Include {inc2}

Include {inc3}
Include {inc4}
"##,
            inc1 = inc1_file.path().display(),
            inc2 = inc2_file.path().display(),
            inc3 = inc3_file.path().display(),
            inc4 = inc4_file.path().display(),
        );
        config_file.write_all(config.as_bytes()).unwrap();

        // write include 1
        let inc1 = r##"
        ConnectTimeout 60
        ServerAliveInterval 60
        TcpKeepAlive    yes
        "##;
        inc1_file.write_all(inc1.as_bytes()).unwrap();

        // write include 2
        let inc2 = r##"
        ConnectTimeout 180
        ServerAliveInterval 180
        Ciphers     +a-manella,blowfish,coi-piedi,cazdecan,triestin-stretto
        "##;
        inc2_file.write_all(inc2.as_bytes()).unwrap();

        // write include 3 with host directive
        let inc3 = r##"
Host microwave
    User    mario-rossi
    HostName    192.168.24.33
    RemoteForward   88
    Compression no
    # Ignore unknown fields should be inherited from the global section
    Pippo yes
    Pluto 56
"##;
        inc3_file.write_all(inc3.as_bytes()).unwrap();

        // write include 4 which updates a param from microwave and then create a new host
        let inc4 = r##"
    # Update microwave
    ServerAliveInterval 30
    Port 345

# Force microwave update (it won't work)
Host microwave
    ConnectTimeout 30

Host fridge
    User    luigi-verdi
    HostName    192.168.24.34
    RemoteForward   88
    Compression no
"##;
        inc4_file.write_all(inc4.as_bytes()).unwrap();

        ConfigWithInclude {
            config: config_file,
            inc1: inc1_file,
            inc2: inc2_file,
            inc3: inc3_file,
            inc4: inc4_file,
        }
    }
}
