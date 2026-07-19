//! Server address handling. Deliberately kept separate from the connection
//! logic so the app can offer a "server address" field at login/setup time
//! instead of a hardcoded host — anyone self-hosting Open OSCAR Server
//! should be able to point this client at their own instance.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub host: String,
    pub port: u16,
}

impl ServerAddress {
    pub const DEFAULT_PORT: u16 = 5190; // OSCAR's traditional port, matches OSCAR_ADVERTISED_LISTENERS_PLAIN

    /// Parses user-entered server input in any of these forms:
    ///   - `65.21.63.253`               -> port defaults to 5190
    ///   - `65.21.63.253:5190`
    ///   - `aim.example.com`
    ///   - `aim.example.com:5190`
    ///   - `oscar://aim.example.com:5190` (scheme is accepted and ignored —
    ///     OSCAR isn't URL-addressed on the wire, this is purely a UI convenience
    ///     for anyone who's used to typing addresses with a scheme)
    pub fn parse(input: &str) -> Result<Self, ServerAddressError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ServerAddressError::Empty);
        }

        let without_scheme = trimmed
            .strip_prefix("oscar://")
            .or_else(|| trimmed.strip_prefix("aim://"))
            .unwrap_or(trimmed);

        // Trailing slash is easy to leave in when copy-pasting a scheme'd URL.
        let without_scheme = without_scheme.trim_end_matches('/');

        match without_scheme.rsplit_once(':') {
            Some((host, port_str)) if !host.is_empty() => {
                let port = port_str
                    .parse::<u16>()
                    .map_err(|_| ServerAddressError::InvalidPort(port_str.to_string()))?;
                Ok(ServerAddress { host: host.to_string(), port })
            }
            _ => Ok(ServerAddress {
                host: without_scheme.to_string(),
                port: Self::DEFAULT_PORT,
            }),
        }
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServerAddressError {
    #[error("server address is empty")]
    Empty,
    #[error("invalid port: {0}")]
    InvalidPort(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_host_with_default_port() {
        let addr = ServerAddress::parse("65.21.63.253").unwrap();
        assert_eq!(addr.host, "65.21.63.253");
        assert_eq!(addr.port, ServerAddress::DEFAULT_PORT);
    }

    #[test]
    fn parses_host_with_explicit_port() {
        let addr = ServerAddress::parse("65.21.63.253:5190").unwrap();
        assert_eq!(addr.host, "65.21.63.253");
        assert_eq!(addr.port, 5190);
    }

    #[test]
    fn parses_domain_with_port() {
        let addr = ServerAddress::parse("aim.example.com:5190").unwrap();
        assert_eq!(addr.host, "aim.example.com");
        assert_eq!(addr.port, 5190);
    }

    #[test]
    fn strips_oscar_scheme() {
        let addr = ServerAddress::parse("oscar://aim.example.com:5190/").unwrap();
        assert_eq!(addr.host, "aim.example.com");
        assert_eq!(addr.port, 5190);
    }

    #[test]
    fn rejects_empty_input() {
        assert_eq!(ServerAddress::parse("   "), Err(ServerAddressError::Empty));
    }

    #[test]
    fn rejects_non_numeric_port() {
        assert!(matches!(
            ServerAddress::parse("host:notaport"),
            Err(ServerAddressError::InvalidPort(_))
        ));
    }

    #[test]
    fn display_formats_as_host_colon_port() {
        let addr = ServerAddress { host: "65.21.63.253".to_string(), port: 5190 };
        assert_eq!(addr.to_string(), "65.21.63.253:5190");
    }
}
