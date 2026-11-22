//! The d1's recommendation for an approval (design page 31): the model-calling half of `kalango-d1`. The d1 reads the
//! action and answers two typed questions with probabilities; `kalango_d1` turns that into a verdict and a reason
//! from its closed list, with the hard rules over it. The recommendation is advisory: nothing here approves.
//!
//! `kalango-agente recomendar` (the approval screen's way in: JSON of the action on stdin, the recommendation on
//! stdout) and the engine's own "Aprovar sozinho" check both come through [`recommend`].

use crate::modelo::{self, Kind, Model};
use kalango_d1::{ActionInfo, Reason, Recommendation};
use std::time::{Duration, Instant};

/// Asks the d1 about an action. `start`: start the model when it is not running (the approval screen waits for it);
/// without it a model that is not running is "offline" at once (the engine never waits for a cold start to decide).
pub fn recommend(a: &ActionInfo, timeout: Duration, start: bool) -> Recommendation {
    if !Kind::Decision.installed() {
        return Recommendation::unknown(Reason::NotInstalled);
    }
    if !start && !modelo::running(Kind::Decision) {
        return kalango_d1::recommend_from(a, Err(Reason::Offline));
    }
    let t0 = Instant::now();
    let model = match Model::ensure_named(Kind::Decision, &|_| {}) {
        Ok(m) => m,
        Err(_) => return kalango_d1::recommend_from(a, Err(Reason::Offline)),
    };
    let left = timeout.saturating_sub(t0.elapsed()).max(Duration::from_millis(200));
    let raw = model.systemone(&kalango_d1::build_request(a), left).map_err(|e| why_not(&e));
    kalango_d1::recommend_from(a, raw)
}

/// As [`recommend`], with one hard deadline for the whole path (finding the server, asking it, reading the answer):
/// past it the answer is "too slow" and the person decides. The engine's "Aprovar sozinho" uses this (1.5 s).
pub fn recommend_bounded(a: &ActionInfo, limit: Duration, start: bool) -> Recommendation {
    let info = a.clone();
    match kalango_d1::within(limit, move || recommend(&info, limit, start)) {
        Some(r) => r,
        None => kalango_d1::recommend_from(a, Err(Reason::Slow)),
    }
}

/// Why a request to the model failed, as a reason the screen can show.
fn why_not(e: &str) -> Reason {
    let e = e.to_lowercase();
    if e.contains("timed out") || e.contains("timeout") || e.contains("temporarily unavailable") || e.contains("would block") {
        Reason::Slow
    } else if e.starts_with("400") || e.starts_with("500") || e.contains("invalid") {
        // the model answered, but not with an answer
        Reason::NoOpinion
    } else {
        Reason::Offline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_map_to_reasons() {
        assert_eq!(why_not("Resource temporarily unavailable (os error 11)"), Reason::Slow);
        assert_eq!(why_not("operation timed out"), Reason::Slow);
        assert_eq!(why_not("400: {\"error\":{}}"), Reason::NoOpinion);
        assert_eq!(why_not("No such file or directory (os error 2)"), Reason::Offline);
        assert_eq!(why_not("Connection refused (os error 111)"), Reason::Offline);
    }
}
