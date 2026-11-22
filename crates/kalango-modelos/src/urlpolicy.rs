//! Which server addresses are accepted, and where a request may go. A private server must be reached over HTTPS;
//! plain `http://` is accepted only for this computer (loopback: tests, a local mirror) and, when the person confirmed
//! it, for an address of a private network. The access token goes only to the configured server's own origin: a
//! redirect to another host, or to plain HTTP, is refused instead of followed with the token attached.

use std::net::IpAddr;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    Invalid,
    /// not http(s)
    Scheme,
    /// `http://` to a host that is neither this computer nor (confirmed) a private network
    PlainHttp,
    /// `user:password@` in the address (a secret in a URL)
    Credentials,
    /// a query or fragment (a token could hide there)
    QueryOrFragment,
}

/// The configured server: scheme, host, port and the base path (no trailing slash).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerUrl {
    url: Url,
}

pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// An address of a private network: RFC 1918, link-local, unique-local IPv6, or a `.local` name.
pub fn is_private_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    if h.to_ascii_lowercase().ends_with(".local") {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(a)) => a.is_private() || a.is_link_local(),
        Ok(IpAddr::V6(a)) => (a.segments()[0] & 0xfe00) == 0xfc00 || (a.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

/// Reads and checks a server address. `lan_confirmed`: the person ticked "this server is on my private network"
/// (the only way a non-loopback `http://` is accepted).
pub fn parse_server(input: &str, lan_confirmed: bool) -> Result<ServerUrl, UrlError> {
    let t = input.trim();
    if t.is_empty() {
        return Err(UrlError::Empty);
    }
    let url = Url::parse(t).map_err(|_| UrlError::Invalid)?;
    let host = url.host_str().ok_or(UrlError::Invalid)?.to_string();
    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(&host) || (lan_confirmed && is_private_host(&host)) => {}
        "http" => return Err(UrlError::PlainHttp),
        _ => return Err(UrlError::Scheme),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlError::Credentials);
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(UrlError::QueryOrFragment);
    }
    Ok(ServerUrl { url })
}

fn origin_of(u: &Url) -> (String, String, Option<u16>) {
    (u.scheme().to_string(), u.host_str().unwrap_or("").to_ascii_lowercase(), u.port_or_known_default())
}

impl ServerUrl {
    pub fn host(&self) -> &str {
        self.url.host_str().unwrap_or("")
    }
    pub fn is_https(&self) -> bool {
        self.url.scheme() == "https"
    }
    /// The address as the person typed it, normalised (no trailing slash).
    pub fn base(&self) -> String {
        self.url.as_str().trim_end_matches('/').to_string()
    }
    /// Where a file is served: `<base>/<name>` (the name must be a safe file name).
    pub fn file_url(&self, name: &str) -> String {
        format!("{}/{name}", self.base())
    }
    /// "scheme://host:port": what an access token is bound to.
    pub fn origin(&self) -> String {
        let (s, h, p) = origin_of(&self.url);
        format!("{s}://{h}:{}", p.unwrap_or(0))
    }
    /// Is this address the server's own origin (same scheme, host and port)? Only these get the token.
    pub fn same_origin(&self, target: &str) -> bool {
        Url::parse(target).is_ok_and(|t| origin_of(&t) == origin_of(&self.url))
    }
}

/// Where a redirect from `from` to `location` (absolute or relative) leads, when it may be followed:
/// never from https to http; and, when `pinned` is the private server, never to another origin.
pub fn redirect_target(from: &str, location: &str, pinned: Option<&ServerUrl>) -> Result<String, String> {
    let base = Url::parse(from).map_err(|e| e.to_string())?;
    let to = base.join(location).map_err(|e| e.to_string())?;
    if !matches!(to.scheme(), "http" | "https") {
        return Err("redirect to a non-HTTP address".into());
    }
    if base.scheme() == "https" && to.scheme() == "http" {
        return Err("redirect from HTTPS to plain HTTP".into());
    }
    if let Some(s) = pinned {
        if !s.same_origin(to.as_str()) {
            return Err(format!("redirect to another host ({})", to.host_str().unwrap_or("?")));
        }
    }
    Ok(to.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_is_accepted_and_normalised() {
        let s = parse_server(" https://modelos.example.org/kalango/ ", false).unwrap();
        assert_eq!(s.base(), "https://modelos.example.org/kalango");
        assert_eq!(s.host(), "modelos.example.org");
        assert_eq!(s.file_url("a.gguf"), "https://modelos.example.org/kalango/a.gguf");
    }

    #[test]
    fn plain_http_only_for_loopback_or_a_confirmed_private_network() {
        assert!(parse_server("http://127.0.0.1:8080/m", false).is_ok());
        assert!(parse_server("http://localhost/m", false).is_ok());
        assert!(parse_server("http://[::1]:9/m", false).is_ok());
        assert_eq!(parse_server("http://modelos.example.org", true), Err(UrlError::PlainHttp));
        assert_eq!(parse_server("http://192.168.1.20/m", false), Err(UrlError::PlainHttp));
        assert!(parse_server("http://192.168.1.20/m", true).is_ok());
        assert!(parse_server("http://nas.local/m", true).is_ok());
        assert_eq!(parse_server("http://8.8.8.8/m", true), Err(UrlError::PlainHttp));
    }

    #[test]
    fn refuses_secrets_in_the_address_and_other_schemes() {
        assert_eq!(parse_server("https://u:p@example.org", false), Err(UrlError::Credentials));
        assert_eq!(parse_server("https://example.org/?token=abc", false), Err(UrlError::QueryOrFragment));
        assert_eq!(parse_server("ftp://example.org", false), Err(UrlError::Scheme));
        assert_eq!(parse_server("file:///etc", false), Err(UrlError::Invalid));
        assert_eq!(parse_server("", false), Err(UrlError::Empty));
        assert_eq!(parse_server("not a url", false), Err(UrlError::Invalid));
    }

    #[test]
    fn the_token_origin_is_exact() {
        let s = parse_server("https://modelos.example.org/k", false).unwrap();
        assert!(s.same_origin("https://modelos.example.org/k/a.gguf"));
        assert!(s.same_origin("https://MODELOS.example.org:443/x"));
        assert!(!s.same_origin("https://cdn.example.org/a.gguf"));
        assert!(!s.same_origin("https://modelos.example.org.evil.test/a"));
        assert!(!s.same_origin("http://modelos.example.org/a"));
        assert!(!s.same_origin("https://modelos.example.org:8443/a"));
    }

    #[test]
    fn redirects_never_downgrade_and_never_leave_the_pinned_server() {
        let s = parse_server("https://modelos.example.org/k", false).unwrap();
        let from = "https://modelos.example.org/k/a.gguf";
        assert_eq!(redirect_target(from, "/k/b.gguf", Some(&s)).unwrap(), "https://modelos.example.org/k/b.gguf");
        assert!(redirect_target(from, "https://cdn.example.org/b", Some(&s)).is_err());
        assert!(redirect_target(from, "http://modelos.example.org/b", Some(&s)).is_err());
        // the public source follows its CDN (no token is attached there) but still never to plain http
        assert!(redirect_target(from, "https://cdn.example.org/b", None).is_ok());
        assert!(redirect_target(from, "http://cdn.example.org/b", None).is_err());
        assert!(redirect_target(from, "ftp://cdn.example.org/b", None).is_err());
    }
}
