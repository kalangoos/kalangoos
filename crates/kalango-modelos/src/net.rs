//! HTTP for the models, with the rules that keep the access token safe:
//! - the `Authorization: Bearer` header goes only to the private server's own origin (same scheme, host and port);
//! - a redirect is followed by hand: never from https to http, and, for the private server, never to another origin;
//! - nothing here prints the token, and it is never put in a URL.

use crate::urlpolicy::{redirect_target, ServerUrl};
use crate::Secret;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// no network at all (no interface up)
    Offline,
    /// the connection or the answer took too long
    Timeout,
    /// the server could not be reached (DNS, refused, reset)
    Unreachable(String),
    /// 401 or 403
    Denied(u16),
    /// 404 or 410
    NotFound,
    /// any other status
    Status(u16),
    /// a redirect that was refused
    Redirect(String),
}

/// Is any network interface other than loopback up? (`KALANGO_REDE=up|down` forces the answer: session tests.)
pub fn network_up() -> bool {
    match std::env::var("KALANGO_REDE").as_deref() {
        Ok("up") => return true,
        Ok("down") => return false,
        _ => {}
    }
    let Ok(rd) = std::fs::read_dir("/sys/class/net") else { return true };
    rd.flatten().filter(|e| e.file_name() != "lo").any(|e| {
        std::fs::read_to_string(e.path().join("operstate")).map(|s| matches!(s.trim(), "up" | "unknown")).unwrap_or(false)
    })
}

pub struct Client {
    agent: ureq::Agent,
    pinned: Option<ServerUrl>,
    token: Option<Secret>,
    online: fn() -> bool,
    pub connect_timeout: Duration,
}

impl Client {
    /// `pinned`: the private server (the only origin that gets the token); None for the public source.
    pub fn new(pinned: Option<ServerUrl>, token: Option<Secret>) -> Client {
        Client::with_timeouts(pinned, token, Duration::from_secs(10), Duration::from_secs(30))
    }

    pub fn with_timeouts(pinned: Option<ServerUrl>, token: Option<Secret>, connect: Duration, read: Duration) -> Client {
        let agent = ureq::AgentBuilder::new().redirects(0).timeout_connect(connect).timeout_read(read).timeout_write(read).user_agent("KalangoOS-modelos").build();
        // (a token without a pinned server would go nowhere: it is dropped)
        let token = if pinned.is_some() { token } else { None };
        Client { agent, pinned, token, online: network_up, connect_timeout: connect }
    }

    /// Tests: say whether the network is up.
    pub fn with_online(mut self, online: fn() -> bool) -> Client {
        self.online = online;
        self
    }

    /// Does this request carry the token? (only the pinned server's own origin)
    pub fn sends_token_to(&self, url: &str) -> bool {
        self.token.is_some() && self.pinned.as_ref().is_some_and(|p| p.same_origin(url))
    }

    /// One request, following redirects under the rules above. `from`: a byte offset for `Range: bytes=N-`.
    pub fn request(&self, method: &str, url: &str, from: Option<u64>) -> Result<ureq::Response, NetError> {
        let mut url = url.to_string();
        for _ in 0..6 {
            let mut req = self.agent.request(method, &url);
            if self.sends_token_to(&url) {
                req = req.set("Authorization", &format!("Bearer {}", self.token.as_ref().map(Secret::expose).unwrap_or("")));
            }
            if let Some(n) = from.filter(|n| *n > 0) {
                req = req.set("Range", &format!("bytes={n}-"));
            }
            match req.call() {
                Ok(r) if (300..400).contains(&r.status()) && r.status() != 304 => {
                    let loc = r.header("Location").ok_or_else(|| NetError::Redirect("redirect without a destination".into()))?.to_string();
                    url = redirect_target(&url, &loc, self.pinned.as_ref()).map_err(NetError::Redirect)?;
                }
                Ok(r) => return Ok(r),
                Err(ureq::Error::Status(c, _)) => {
                    return Err(match c {
                        401 | 403 => NetError::Denied(c),
                        404 | 410 => NetError::NotFound,
                        c => NetError::Status(c),
                    })
                }
                Err(ureq::Error::Transport(t)) => return Err(self.classify(&t)),
            }
        }
        Err(NetError::Redirect("too many redirects".into()))
    }

    fn classify(&self, t: &ureq::Transport) -> NetError {
        let msg = t.to_string();
        let lower = msg.to_lowercase();
        if !(self.online)() {
            NetError::Offline
        } else if lower.contains("timed out") || lower.contains("timeout") || lower.contains("would block") {
            NetError::Timeout
        } else {
            // (the message names the host, never a header)
            NetError::Unreachable(msg)
        }
    }
}
