//! # field
//!
//! Ssh config fields

use std::fmt;
use std::str::FromStr;

/// Configuration field.
/// This enum defines ALL THE SUPPORTED fields in ssh config,
/// as described at <http://man.openbsd.org/OpenBSD-current/man5/ssh_config.5>.
/// Only a few of them are implemented, as described in `HostParams` struct.
#[derive(Debug, Eq, PartialEq)]
pub enum Field {
    Host,
    AddKeysToAgent,
    BindAddress,
    BindInterface,
    CaSignatureAlgorithms,
    CertificateFile,
    Ciphers,
    Compression,
    ConnectionAttempts,
    ConnectTimeout,
    ForwardAgent,
    HostKeyAlgorithms,
    HostName,
    IdentityFile,
    IgnoreUnknown,
    KexAlgorithms,
    Mac,
    Port,
    ProxyJump,
    PubkeyAcceptedAlgorithms,
    PubkeyAuthentication,
    RemoteForward,
    ServerAliveInterval,
    TcpKeepAlive,
    #[cfg(target_os = "macos")]
    UseKeychain,
    User,
    // -- not implemented
    AddressFamily,
    BatchMode,
    CanonicalDomains,
    CanonicalizeFallbackLock,
    CanonicalizeHostname,
    CanonicalizeMaxDots,
    CanonicalizePermittedCNAMEs,
    CheckHostIP,
    ClearAllForwardings,
    ControlMaster,
    ControlPath,
    ControlPersist,
    DynamicForward,
    EnableSSHKeysign,
    EscapeChar,
    ExitOnForwardFailure,
    FingerprintHash,
    ForkAfterAuthentication,
    ForwardX11,
    ForwardX11Timeout,
    ForwardX11Trusted,
    GatewayPorts,
    GlobalKnownHostsFile,
    GSSAPIAuthentication,
    GSSAPIDelegateCredentials,
    HashKnownHosts,
    HostbasedAcceptedAlgorithms,
    HostbasedAuthentication,
    HostbasedKeyTypes,
    HostKeyAlias,
    IdentitiesOnly,
    IdentityAgent,
    Include,
    IPQoS,
    KbdInteractiveAuthentication,
    KbdInteractiveDevices,
    KnownHostsCommand,
    LocalCommand,
    LocalForward,
    LogLevel,
    LogVerbose,
    NoHostAuthenticationForLocalhost,
    NumberOfPasswordPrompts,
    PasswordAuthentication,
    PermitLocalCommand,
    PermitRemoteOpen,
    PKCS11Provider,
    PreferredAuthentications,
    ProxyCommand,
    ProxyUseFdpass,
    PubkeyAcceptedKeyTypes,
    RekeyLimit,
    RequestTTY,
    RevokedHostKeys,
    SecruityKeyProvider,
    SendEnv,
    ServerAliveCountMax,
    SessionType,
    SetEnv,
    StdinNull,
    StreamLocalBindMask,
    StrictHostKeyChecking,
    SyslogFacility,
    UpdateHostKeys,
    UserKnownHostsFile,
    VerifyHostKeyDNS,
    VisualHostKey,
    XAuthLocation,
}

impl FromStr for Field {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "host" => Ok(Self::Host),
            "addkeystoagent" => Ok(Self::AddKeysToAgent),
            "bindaddress" => Ok(Self::BindAddress),
            "bindinterface" => Ok(Self::BindInterface),
            "casignaturealgorithms" => Ok(Self::CaSignatureAlgorithms),
            "certificatefile" => Ok(Self::CertificateFile),
            "ciphers" => Ok(Self::Ciphers),
            "compression" => Ok(Self::Compression),
            "connectionattempts" => Ok(Self::ConnectionAttempts),
            "connecttimeout" => Ok(Self::ConnectTimeout),
            "forwardagent" => Ok(Self::ForwardAgent),
            "hostkeyalgorithms" => Ok(Self::HostKeyAlgorithms),
            "hostname" => Ok(Self::HostName),
            "identityfile" => Ok(Self::IdentityFile),
            "ignoreunknown" => Ok(Self::IgnoreUnknown),
            "kexalgorithms" => Ok(Self::KexAlgorithms),
            "macs" => Ok(Self::Mac),
            "port" => Ok(Self::Port),
            "proxyjump" => Ok(Self::ProxyJump),
            "pubkeyacceptedalgorithms" => Ok(Self::PubkeyAcceptedAlgorithms),
            "pubkeyauthentication" => Ok(Self::PubkeyAuthentication),
            "remoteforward" => Ok(Self::RemoteForward),
            "serveraliveinterval" => Ok(Self::ServerAliveInterval),
            "tcpkeepalive" => Ok(Self::TcpKeepAlive),
            #[cfg(target_os = "macos")]
            "usekeychain" => Ok(Self::UseKeychain),
            "user" => Ok(Self::User),
            // -- not implemented fields
            "addressfamily" => Ok(Self::AddressFamily),
            "batchmode" => Ok(Self::BatchMode),
            "canonicaldomains" => Ok(Self::CanonicalDomains),
            "canonicalizefallbacklock" => Ok(Self::CanonicalizeFallbackLock),
            "canonicalizehostname" => Ok(Self::CanonicalizeHostname),
            "canonicalizemaxdots" => Ok(Self::CanonicalizeMaxDots),
            "canonicalizepermittedcnames" => Ok(Self::CanonicalizePermittedCNAMEs),
            "checkhostip" => Ok(Self::CheckHostIP),
            "clearallforwardings" => Ok(Self::ClearAllForwardings),
            "controlmaster" => Ok(Self::ControlMaster),
            "controlpath" => Ok(Self::ControlPath),
            "controlpersist" => Ok(Self::ControlPersist),
            "dynamicforward" => Ok(Self::DynamicForward),
            "enablesshkeysign" => Ok(Self::EnableSSHKeysign),
            "escapechar" => Ok(Self::EscapeChar),
            "exitonforwardfailure" => Ok(Self::ExitOnForwardFailure),
            "fingerprinthash" => Ok(Self::FingerprintHash),
            "forkafterauthentication" => Ok(Self::ForkAfterAuthentication),
            "forwardx11" => Ok(Self::ForwardX11),
            "forwardx11timeout" => Ok(Self::ForwardX11Timeout),
            "forwardx11trusted" => Ok(Self::ForwardX11Trusted),
            "gatewayports" => Ok(Self::GatewayPorts),
            "globalknownhostsfile" => Ok(Self::GlobalKnownHostsFile),
            "gssapiauthentication" => Ok(Self::GSSAPIAuthentication),
            "gssapidelegatecredentials" => Ok(Self::GSSAPIDelegateCredentials),
            "hashknownhosts" => Ok(Self::HashKnownHosts),
            "hostbasedacceptedalgorithms" => Ok(Self::HostbasedAcceptedAlgorithms),
            "hostbasedauthentication" => Ok(Self::HostbasedAuthentication),
            "hostkeyalias" => Ok(Self::HostKeyAlias),
            "hostbasedkeytypes" => Ok(Self::HostbasedKeyTypes),
            "identitiesonly" => Ok(Self::IdentitiesOnly),
            "identityagent" => Ok(Self::IdentityAgent),
            "include" => Ok(Self::Include),
            "ipqos" => Ok(Self::IPQoS),
            "kbdinteractiveauthentication" => Ok(Self::KbdInteractiveAuthentication),
            "kbdinteractivedevices" => Ok(Self::KbdInteractiveDevices),
            "knownhostscommand" => Ok(Self::KnownHostsCommand),
            "localcommand" => Ok(Self::LocalCommand),
            "localforward" => Ok(Self::LocalForward),
            "loglevel" => Ok(Self::LogLevel),
            "logverbose" => Ok(Self::LogVerbose),
            "nohostauthenticationforlocalhost" => Ok(Self::NoHostAuthenticationForLocalhost),
            "numberofpasswordprompts" => Ok(Self::NumberOfPasswordPrompts),
            "passwordauthentication" => Ok(Self::PasswordAuthentication),
            "permitlocalcommand" => Ok(Self::PermitLocalCommand),
            "permitremoteopen" => Ok(Self::PermitRemoteOpen),
            "pkcs11provider" => Ok(Self::PKCS11Provider),
            "preferredauthentications" => Ok(Self::PreferredAuthentications),
            "proxycommand" => Ok(Self::ProxyCommand),
            "proxyusefdpass" => Ok(Self::ProxyUseFdpass),
            "pubkeyacceptedkeytypes" => Ok(Self::PubkeyAcceptedKeyTypes),
            "rekeylimit" => Ok(Self::RekeyLimit),
            "requesttty" => Ok(Self::RequestTTY),
            "revokedhostkeys" => Ok(Self::RevokedHostKeys),
            "secruitykeyprovider" => Ok(Self::SecruityKeyProvider),
            "sendenv" => Ok(Self::SendEnv),
            "serveralivecountmax" => Ok(Self::ServerAliveCountMax),
            "sessiontype" => Ok(Self::SessionType),
            "setenv" => Ok(Self::SetEnv),
            "stdinnull" => Ok(Self::StdinNull),
            "streamlocalbindmask" => Ok(Self::StreamLocalBindMask),
            "stricthostkeychecking" => Ok(Self::StrictHostKeyChecking),
            "syslogfacility" => Ok(Self::SyslogFacility),
            "updatehostkeys" => Ok(Self::UpdateHostKeys),
            "userknownhostsfile" => Ok(Self::UserKnownHostsFile),
            "verifyhostkeydns" => Ok(Self::VerifyHostKeyDNS),
            "visualhostkey" => Ok(Self::VisualHostKey),
            "xauthlocation" => Ok(Self::XAuthLocation),
            // -- unknwon field
            _ => Err(s.to_string()),
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Host => "host",
            Self::AddKeysToAgent => "addkeystoagent",
            Self::BindAddress => "bindaddress",
            Self::BindInterface => "bindinterface",
            Self::CaSignatureAlgorithms => "casignaturealgorithms",
            Self::CertificateFile => "certificatefile",
            Self::Ciphers => "ciphers",
            Self::Compression => "compression",
            Self::ConnectionAttempts => "connectionattempts",
            Self::ConnectTimeout => "connecttimeout",
            Self::ForwardAgent => "forwardagent",
            Self::HostKeyAlgorithms => "hostkeyalgorithms",
            Self::HostName => "hostname",
            Self::IdentityFile => "identityfile",
            Self::IgnoreUnknown => "ignoreunknown",
            Self::KexAlgorithms => "kexalgorithms",
            Self::Mac => "macs",
            Self::Port => "port",
            Self::ProxyJump => "proxyjump",
            Self::PubkeyAcceptedAlgorithms => "pubkeyacceptedalgorithms",
            Self::PubkeyAuthentication => "pubkeyauthentication",
            Self::RemoteForward => "remoteforward",
            Self::ServerAliveInterval => "serveraliveinterval",
            Self::TcpKeepAlive => "tcpkeepalive",
            #[cfg(target_os = "macos")]
            Self::UseKeychain => "usekeychain",
            Self::User => "user",
            // Continuation of the rest of the enum variants
            Self::AddressFamily => "addressfamily",
            Self::BatchMode => "batchmode",
            Self::CanonicalDomains => "canonicaldomains",
            Self::CanonicalizeFallbackLock => "canonicalizefallbacklock",
            Self::CanonicalizeHostname => "canonicalizehostname",
            Self::CanonicalizeMaxDots => "canonicalizemaxdots",
            Self::CanonicalizePermittedCNAMEs => "canonicalizepermittedcnames",
            Self::CheckHostIP => "checkhostip",
            Self::ClearAllForwardings => "clearallforwardings",
            Self::ControlMaster => "controlmaster",
            Self::ControlPath => "controlpath",
            Self::ControlPersist => "controlpersist",
            Self::DynamicForward => "dynamicforward",
            Self::EnableSSHKeysign => "enablesshkeysign",
            Self::EscapeChar => "escapechar",
            Self::ExitOnForwardFailure => "exitonforwardfailure",
            Self::FingerprintHash => "fingerprinthash",
            Self::ForkAfterAuthentication => "forkafterauthentication",
            Self::ForwardX11 => "forwardx11",
            Self::ForwardX11Timeout => "forwardx11timeout",
            Self::ForwardX11Trusted => "forwardx11trusted",
            Self::GatewayPorts => "gatewayports",
            Self::GlobalKnownHostsFile => "globalknownhostsfile",
            Self::GSSAPIAuthentication => "gssapiauthentication",
            Self::GSSAPIDelegateCredentials => "gssapidelegatecredentials",
            Self::HashKnownHosts => "hashknownhosts",
            Self::HostbasedAcceptedAlgorithms => "hostbasedacceptedalgorithms",
            Self::HostbasedAuthentication => "hostbasedauthentication",
            Self::HostKeyAlias => "hostkeyalias",
            Self::HostbasedKeyTypes => "hostbasedkeytypes",
            Self::IdentitiesOnly => "identitiesonly",
            Self::IdentityAgent => "identityagent",
            Self::Include => "include",
            Self::IPQoS => "ipqos",
            Self::KbdInteractiveAuthentication => "kbdinteractiveauthentication",
            Self::KbdInteractiveDevices => "kbdinteractivedevices",
            Self::KnownHostsCommand => "knownhostscommand",
            Self::LocalCommand => "localcommand",
            Self::LocalForward => "localforward",
            Self::LogLevel => "loglevel",
            Self::LogVerbose => "logverbose",
            Self::NoHostAuthenticationForLocalhost => "nohostauthenticationforlocalhost",
            Self::NumberOfPasswordPrompts => "numberofpasswordprompts",
            Self::PasswordAuthentication => "passwordauthentication",
            Self::PermitLocalCommand => "permitlocalcommand",
            Self::PermitRemoteOpen => "permitremoteopen",
            Self::PKCS11Provider => "pkcs11provider",
            Self::PreferredAuthentications => "preferredauthentications",
            Self::ProxyCommand => "proxycommand",
            Self::ProxyUseFdpass => "proxyusefdpass",
            Self::PubkeyAcceptedKeyTypes => "pubkeyacceptedkeytypes",
            Self::RekeyLimit => "rekeylimit",
            Self::RequestTTY => "requesttty",
            Self::RevokedHostKeys => "revokedhostkeys",
            Self::SecruityKeyProvider => "secruitykeyprovider",
            Self::SendEnv => "sendenv",
            Self::ServerAliveCountMax => "serveralivecountmax",
            Self::SessionType => "sessiontype",
            Self::SetEnv => "setenv",
            Self::StdinNull => "stdinnull",
            Self::StreamLocalBindMask => "streamlocalbindmask",
            Self::StrictHostKeyChecking => "stricthostkeychecking",
            Self::SyslogFacility => "syslogfacility",
            Self::UpdateHostKeys => "updatehostkeys",
            Self::UserKnownHostsFile => "userknownhostsfile",
            Self::VerifyHostKeyDNS => "verifyhostkeydns",
            Self::VisualHostKey => "visualhostkey",
            Self::XAuthLocation => "xauthlocation",
        };
        write!(f, "{}", s)
    }
}

#[cfg(test)]
mod tests {

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn should_parse_field_from_string() {
        assert_eq!(Field::from_str("Host").ok().unwrap(), Field::Host);
        assert_eq!(
            Field::from_str("AddKeysToAgent").ok().unwrap(),
            Field::AddKeysToAgent
        );
        assert_eq!(
            Field::from_str("BindAddress").ok().unwrap(),
            Field::BindAddress
        );
        assert_eq!(
            Field::from_str("BindInterface").ok().unwrap(),
            Field::BindInterface
        );
        assert_eq!(
            Field::from_str("CaSignatureAlgorithms").ok().unwrap(),
            Field::CaSignatureAlgorithms
        );
        assert_eq!(
            Field::from_str("CertificateFile").ok().unwrap(),
            Field::CertificateFile
        );
        assert_eq!(Field::from_str("Ciphers").ok().unwrap(), Field::Ciphers);
        assert_eq!(
            Field::from_str("Compression").ok().unwrap(),
            Field::Compression
        );
        assert_eq!(
            Field::from_str("ConnectionAttempts").ok().unwrap(),
            Field::ConnectionAttempts
        );
        assert_eq!(
            Field::from_str("ConnectTimeout").ok().unwrap(),
            Field::ConnectTimeout
        );
        assert_eq!(
            Field::from_str("ForwardAgent").ok().unwrap(),
            Field::ForwardAgent
        );
        assert_eq!(Field::from_str("HostName").ok().unwrap(), Field::HostName);
        assert_eq!(
            Field::from_str("IdentityFile").ok().unwrap(),
            Field::IdentityFile
        );
        assert_eq!(
            Field::from_str("IgnoreUnknown").ok().unwrap(),
            Field::IgnoreUnknown
        );
        assert_eq!(Field::from_str("Macs").ok().unwrap(), Field::Mac);
        assert_eq!(Field::from_str("ProxyJump").ok().unwrap(), Field::ProxyJump);
        assert_eq!(
            Field::from_str("PubkeyAcceptedAlgorithms").ok().unwrap(),
            Field::PubkeyAcceptedAlgorithms
        );
        assert_eq!(
            Field::from_str("PubkeyAuthentication").ok().unwrap(),
            Field::PubkeyAuthentication
        );
        assert_eq!(
            Field::from_str("RemoteForward").ok().unwrap(),
            Field::RemoteForward
        );
        assert_eq!(
            Field::from_str("TcpKeepAlive").ok().unwrap(),
            Field::TcpKeepAlive
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            Field::from_str("UseKeychain").ok().unwrap(),
            Field::UseKeychain
        );
        assert_eq!(Field::from_str("User").ok().unwrap(), Field::User);
        assert_eq!(
            Field::from_str("AddKeysToAgent").ok().unwrap(),
            Field::AddKeysToAgent
        );
        assert_eq!(
            Field::from_str("AddressFamily").ok().unwrap(),
            Field::AddressFamily
        );
        assert_eq!(Field::from_str("BatchMode").ok().unwrap(), Field::BatchMode);
        assert_eq!(
            Field::from_str("CanonicalDomains").ok().unwrap(),
            Field::CanonicalDomains
        );
        assert_eq!(
            Field::from_str("CanonicalizeFallbackLock").ok().unwrap(),
            Field::CanonicalizeFallbackLock
        );
        assert_eq!(
            Field::from_str("CanonicalizeHostname").ok().unwrap(),
            Field::CanonicalizeHostname
        );
        assert_eq!(
            Field::from_str("CanonicalizeMaxDots").ok().unwrap(),
            Field::CanonicalizeMaxDots
        );
        assert_eq!(
            Field::from_str("CanonicalizePermittedCNAMEs").ok().unwrap(),
            Field::CanonicalizePermittedCNAMEs
        );
        assert_eq!(
            Field::from_str("CheckHostIP").ok().unwrap(),
            Field::CheckHostIP
        );
        assert_eq!(
            Field::from_str("ClearAllForwardings").ok().unwrap(),
            Field::ClearAllForwardings
        );
        assert_eq!(
            Field::from_str("ControlMaster").ok().unwrap(),
            Field::ControlMaster
        );
        assert_eq!(
            Field::from_str("ControlPath").ok().unwrap(),
            Field::ControlPath
        );
        assert_eq!(
            Field::from_str("ControlPersist").ok().unwrap(),
            Field::ControlPersist
        );
        assert_eq!(
            Field::from_str("DynamicForward").ok().unwrap(),
            Field::DynamicForward
        );
        assert_eq!(
            Field::from_str("EnableSSHKeysign").ok().unwrap(),
            Field::EnableSSHKeysign
        );
        assert_eq!(
            Field::from_str("EscapeChar").ok().unwrap(),
            Field::EscapeChar
        );
        assert_eq!(
            Field::from_str("ExitOnForwardFailure").ok().unwrap(),
            Field::ExitOnForwardFailure
        );
        assert_eq!(
            Field::from_str("FingerprintHash").ok().unwrap(),
            Field::FingerprintHash
        );
        assert_eq!(
            Field::from_str("ForkAfterAuthentication").ok().unwrap(),
            Field::ForkAfterAuthentication
        );
        assert_eq!(
            Field::from_str("ForwardAgent").ok().unwrap(),
            Field::ForwardAgent
        );
        assert_eq!(
            Field::from_str("ForwardX11").ok().unwrap(),
            Field::ForwardX11
        );
        assert_eq!(
            Field::from_str("ForwardX11Timeout").ok().unwrap(),
            Field::ForwardX11Timeout
        );
        assert_eq!(
            Field::from_str("ForwardX11Trusted").ok().unwrap(),
            Field::ForwardX11Trusted,
        );
        assert_eq!(
            Field::from_str("GatewayPorts").ok().unwrap(),
            Field::GatewayPorts
        );
        assert_eq!(
            Field::from_str("GlobalKnownHostsFile").ok().unwrap(),
            Field::GlobalKnownHostsFile
        );
        assert_eq!(
            Field::from_str("GSSAPIAuthentication").ok().unwrap(),
            Field::GSSAPIAuthentication
        );
        assert_eq!(
            Field::from_str("GSSAPIDelegateCredentials").ok().unwrap(),
            Field::GSSAPIDelegateCredentials
        );
        assert_eq!(
            Field::from_str("HashKnownHosts").ok().unwrap(),
            Field::HashKnownHosts
        );
        assert_eq!(
            Field::from_str("HostbasedAcceptedAlgorithms").ok().unwrap(),
            Field::HostbasedAcceptedAlgorithms
        );
        assert_eq!(
            Field::from_str("HostbasedAuthentication").ok().unwrap(),
            Field::HostbasedAuthentication
        );
        assert_eq!(
            Field::from_str("HostKeyAlgorithms").ok().unwrap(),
            Field::HostKeyAlgorithms
        );
        assert_eq!(
            Field::from_str("HostKeyAlias").ok().unwrap(),
            Field::HostKeyAlias
        );
        assert_eq!(
            Field::from_str("HostbasedKeyTypes").ok().unwrap(),
            Field::HostbasedKeyTypes
        );
        assert_eq!(
            Field::from_str("IdentitiesOnly").ok().unwrap(),
            Field::IdentitiesOnly
        );
        assert_eq!(
            Field::from_str("IdentityAgent").ok().unwrap(),
            Field::IdentityAgent
        );
        assert_eq!(Field::from_str("Include").ok().unwrap(), Field::Include);
        assert_eq!(Field::from_str("IPQoS").ok().unwrap(), Field::IPQoS);
        assert_eq!(
            Field::from_str("KbdInteractiveAuthentication")
                .ok()
                .unwrap(),
            Field::KbdInteractiveAuthentication
        );
        assert_eq!(
            Field::from_str("KbdInteractiveDevices").ok().unwrap(),
            Field::KbdInteractiveDevices
        );
        assert_eq!(
            Field::from_str("KnownHostsCommand").ok().unwrap(),
            Field::KnownHostsCommand
        );
        assert_eq!(
            Field::from_str("LocalCommand").ok().unwrap(),
            Field::LocalCommand
        );
        assert_eq!(
            Field::from_str("LocalForward").ok().unwrap(),
            Field::LocalForward
        );
        assert_eq!(Field::from_str("LogLevel").ok().unwrap(), Field::LogLevel);
        assert_eq!(
            Field::from_str("LogVerbose").ok().unwrap(),
            Field::LogVerbose
        );
        assert_eq!(
            Field::from_str("NoHostAuthenticationForLocalhost")
                .ok()
                .unwrap(),
            Field::NoHostAuthenticationForLocalhost
        );
        assert_eq!(
            Field::from_str("NumberOfPasswordPrompts").ok().unwrap(),
            Field::NumberOfPasswordPrompts
        );
        assert_eq!(
            Field::from_str("PasswordAuthentication").ok().unwrap(),
            Field::PasswordAuthentication
        );
        assert_eq!(
            Field::from_str("PermitLocalCommand").ok().unwrap(),
            Field::PermitLocalCommand
        );
        assert_eq!(
            Field::from_str("PermitRemoteOpen").ok().unwrap(),
            Field::PermitRemoteOpen
        );
        assert_eq!(
            Field::from_str("PKCS11Provider").ok().unwrap(),
            Field::PKCS11Provider
        );
        assert_eq!(Field::from_str("Port").ok().unwrap(), Field::Port);
        assert_eq!(
            Field::from_str("PreferredAuthentications").ok().unwrap(),
            Field::PreferredAuthentications
        );
        assert_eq!(
            Field::from_str("ProxyCommand").ok().unwrap(),
            Field::ProxyCommand
        );
        assert_eq!(Field::from_str("ProxyJump").ok().unwrap(), Field::ProxyJump);
        assert_eq!(
            Field::from_str("ProxyUseFdpass").ok().unwrap(),
            Field::ProxyUseFdpass
        );
        assert_eq!(
            Field::from_str("PubkeyAcceptedKeyTypes").ok().unwrap(),
            Field::PubkeyAcceptedKeyTypes
        );
        assert_eq!(
            Field::from_str("RekeyLimit").ok().unwrap(),
            Field::RekeyLimit
        );
        assert_eq!(
            Field::from_str("RequestTTY").ok().unwrap(),
            Field::RequestTTY
        );
        assert_eq!(
            Field::from_str("RevokedHostKeys").ok().unwrap(),
            Field::RevokedHostKeys
        );
        assert_eq!(
            Field::from_str("SecruityKeyProvider").ok().unwrap(),
            Field::SecruityKeyProvider
        );
        assert_eq!(Field::from_str("SendEnv").ok().unwrap(), Field::SendEnv);
        assert_eq!(
            Field::from_str("ServerAliveCountMax").ok().unwrap(),
            Field::ServerAliveCountMax
        );
        assert_eq!(
            Field::from_str("ServerAliveInterval").ok().unwrap(),
            Field::ServerAliveInterval
        );
        assert_eq!(
            Field::from_str("SessionType").ok().unwrap(),
            Field::SessionType
        );
        assert_eq!(Field::from_str("SetEnv").ok().unwrap(), Field::SetEnv);
        assert_eq!(Field::from_str("StdinNull").ok().unwrap(), Field::StdinNull);
        assert_eq!(
            Field::from_str("StreamLocalBindMask").ok().unwrap(),
            Field::StreamLocalBindMask
        );
        assert_eq!(
            Field::from_str("StrictHostKeyChecking").ok().unwrap(),
            Field::StrictHostKeyChecking
        );
        assert_eq!(
            Field::from_str("SyslogFacility").ok().unwrap(),
            Field::SyslogFacility
        );
        assert_eq!(
            Field::from_str("UpdateHostKeys").ok().unwrap(),
            Field::UpdateHostKeys
        );
        assert_eq!(
            Field::from_str("UserKnownHostsFile").ok().unwrap(),
            Field::UserKnownHostsFile
        );
        assert_eq!(
            Field::from_str("VerifyHostKeyDNS").ok().unwrap(),
            Field::VerifyHostKeyDNS
        );
        assert_eq!(
            Field::from_str("VisualHostKey").ok().unwrap(),
            Field::VisualHostKey
        );
        assert_eq!(
            Field::from_str("XAuthLocation").ok().unwrap(),
            Field::XAuthLocation
        );
    }

    #[test]
    fn should_fail_parsing_field() {
        assert!(Field::from_str("CristinaDavena").is_err());
    }

    #[test]
    fn should_display_field() {
        // Test Display implementation for supported fields
        assert_eq!(Field::Host.to_string(), "host");
        assert_eq!(Field::AddKeysToAgent.to_string(), "addkeystoagent");
        assert_eq!(Field::BindAddress.to_string(), "bindaddress");
        assert_eq!(Field::BindInterface.to_string(), "bindinterface");
        assert_eq!(
            Field::CaSignatureAlgorithms.to_string(),
            "casignaturealgorithms"
        );
        assert_eq!(Field::CertificateFile.to_string(), "certificatefile");
        assert_eq!(Field::Ciphers.to_string(), "ciphers");
        assert_eq!(Field::Compression.to_string(), "compression");
        assert_eq!(Field::ConnectionAttempts.to_string(), "connectionattempts");
        assert_eq!(Field::ConnectTimeout.to_string(), "connecttimeout");
        assert_eq!(Field::ForwardAgent.to_string(), "forwardagent");
        assert_eq!(Field::HostKeyAlgorithms.to_string(), "hostkeyalgorithms");
        assert_eq!(Field::HostName.to_string(), "hostname");
        assert_eq!(Field::IdentityFile.to_string(), "identityfile");
        assert_eq!(Field::IgnoreUnknown.to_string(), "ignoreunknown");
        assert_eq!(Field::KexAlgorithms.to_string(), "kexalgorithms");
        assert_eq!(Field::Mac.to_string(), "macs");
        assert_eq!(Field::Port.to_string(), "port");
        assert_eq!(Field::ProxyJump.to_string(), "proxyjump");
        assert_eq!(
            Field::PubkeyAcceptedAlgorithms.to_string(),
            "pubkeyacceptedalgorithms"
        );
        assert_eq!(
            Field::PubkeyAuthentication.to_string(),
            "pubkeyauthentication"
        );
        assert_eq!(Field::RemoteForward.to_string(), "remoteforward");
        assert_eq!(
            Field::ServerAliveInterval.to_string(),
            "serveraliveinterval"
        );
        assert_eq!(Field::TcpKeepAlive.to_string(), "tcpkeepalive");
        #[cfg(target_os = "macos")]
        assert_eq!(Field::UseKeychain.to_string(), "usekeychain");
        assert_eq!(Field::User.to_string(), "user");

        // Test Display implementation for unsupported fields
        assert_eq!(Field::AddressFamily.to_string(), "addressfamily");
        assert_eq!(Field::BatchMode.to_string(), "batchmode");
        assert_eq!(Field::CanonicalDomains.to_string(), "canonicaldomains");
        assert_eq!(
            Field::CanonicalizeFallbackLock.to_string(),
            "canonicalizefallbacklock"
        );
        assert_eq!(
            Field::CanonicalizeHostname.to_string(),
            "canonicalizehostname"
        );
        assert_eq!(
            Field::CanonicalizeMaxDots.to_string(),
            "canonicalizemaxdots"
        );
        assert_eq!(
            Field::CanonicalizePermittedCNAMEs.to_string(),
            "canonicalizepermittedcnames"
        );
        assert_eq!(Field::CheckHostIP.to_string(), "checkhostip");
        assert_eq!(
            Field::ClearAllForwardings.to_string(),
            "clearallforwardings"
        );
        assert_eq!(Field::ControlMaster.to_string(), "controlmaster");
        assert_eq!(Field::ControlPath.to_string(), "controlpath");
        assert_eq!(Field::ControlPersist.to_string(), "controlpersist");
        assert_eq!(Field::DynamicForward.to_string(), "dynamicforward");
        assert_eq!(Field::EnableSSHKeysign.to_string(), "enablesshkeysign");
        assert_eq!(Field::EscapeChar.to_string(), "escapechar");
        assert_eq!(
            Field::ExitOnForwardFailure.to_string(),
            "exitonforwardfailure"
        );
        assert_eq!(Field::FingerprintHash.to_string(), "fingerprinthash");
        assert_eq!(
            Field::ForkAfterAuthentication.to_string(),
            "forkafterauthentication"
        );
        assert_eq!(Field::ForwardX11.to_string(), "forwardx11");
        assert_eq!(Field::ForwardX11Timeout.to_string(), "forwardx11timeout");
        assert_eq!(Field::ForwardX11Trusted.to_string(), "forwardx11trusted");
        assert_eq!(Field::GatewayPorts.to_string(), "gatewayports");
        assert_eq!(
            Field::GlobalKnownHostsFile.to_string(),
            "globalknownhostsfile"
        );
        assert_eq!(
            Field::GSSAPIAuthentication.to_string(),
            "gssapiauthentication"
        );
        assert_eq!(
            Field::GSSAPIDelegateCredentials.to_string(),
            "gssapidelegatecredentials"
        );
        assert_eq!(Field::HashKnownHosts.to_string(), "hashknownhosts");
        assert_eq!(
            Field::HostbasedAcceptedAlgorithms.to_string(),
            "hostbasedacceptedalgorithms"
        );
        assert_eq!(
            Field::HostbasedAuthentication.to_string(),
            "hostbasedauthentication"
        );
        assert_eq!(Field::HostKeyAlias.to_string(), "hostkeyalias");
        assert_eq!(Field::HostbasedKeyTypes.to_string(), "hostbasedkeytypes");
        assert_eq!(Field::IdentitiesOnly.to_string(), "identitiesonly");
        assert_eq!(Field::IdentityAgent.to_string(), "identityagent");
        assert_eq!(Field::Include.to_string(), "include");
        assert_eq!(Field::IPQoS.to_string(), "ipqos");
        assert_eq!(
            Field::KbdInteractiveAuthentication.to_string(),
            "kbdinteractiveauthentication"
        );
        assert_eq!(
            Field::KbdInteractiveDevices.to_string(),
            "kbdinteractivedevices"
        );
        assert_eq!(Field::KnownHostsCommand.to_string(), "knownhostscommand");
        assert_eq!(Field::LocalCommand.to_string(), "localcommand");
        assert_eq!(Field::LocalForward.to_string(), "localforward");
        assert_eq!(Field::LogLevel.to_string(), "loglevel");
        assert_eq!(Field::LogVerbose.to_string(), "logverbose");
        assert_eq!(
            Field::NoHostAuthenticationForLocalhost.to_string(),
            "nohostauthenticationforlocalhost"
        );
        assert_eq!(
            Field::NumberOfPasswordPrompts.to_string(),
            "numberofpasswordprompts"
        );
        assert_eq!(
            Field::PasswordAuthentication.to_string(),
            "passwordauthentication"
        );
        assert_eq!(Field::PermitLocalCommand.to_string(), "permitlocalcommand");
        assert_eq!(Field::PermitRemoteOpen.to_string(), "permitremoteopen");
        assert_eq!(Field::PKCS11Provider.to_string(), "pkcs11provider");
        assert_eq!(
            Field::PreferredAuthentications.to_string(),
            "preferredauthentications"
        );
        assert_eq!(Field::ProxyCommand.to_string(), "proxycommand");
        assert_eq!(Field::ProxyUseFdpass.to_string(), "proxyusefdpass");
        assert_eq!(
            Field::PubkeyAcceptedKeyTypes.to_string(),
            "pubkeyacceptedkeytypes"
        );
        assert_eq!(Field::RekeyLimit.to_string(), "rekeylimit");
        assert_eq!(Field::RequestTTY.to_string(), "requesttty");
        assert_eq!(Field::RevokedHostKeys.to_string(), "revokedhostkeys");
        assert_eq!(
            Field::SecruityKeyProvider.to_string(),
            "secruitykeyprovider"
        );
        assert_eq!(Field::SendEnv.to_string(), "sendenv");
        assert_eq!(
            Field::ServerAliveCountMax.to_string(),
            "serveralivecountmax"
        );
        assert_eq!(Field::SessionType.to_string(), "sessiontype");
        assert_eq!(Field::SetEnv.to_string(), "setenv");
        assert_eq!(Field::StdinNull.to_string(), "stdinnull");
        assert_eq!(
            Field::StreamLocalBindMask.to_string(),
            "streamlocalbindmask"
        );
        assert_eq!(
            Field::StrictHostKeyChecking.to_string(),
            "stricthostkeychecking"
        );
        assert_eq!(Field::SyslogFacility.to_string(), "syslogfacility");
        assert_eq!(Field::UpdateHostKeys.to_string(), "updatehostkeys");
        assert_eq!(Field::UserKnownHostsFile.to_string(), "userknownhostsfile");
        assert_eq!(Field::VerifyHostKeyDNS.to_string(), "verifyhostkeydns");
        assert_eq!(Field::VisualHostKey.to_string(), "visualhostkey");
        assert_eq!(Field::XAuthLocation.to_string(), "xauthlocation");
    }
}
