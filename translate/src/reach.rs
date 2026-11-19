//! Cheap "can this provider answer from here?" probe.
//!
//! The setup card orders providers by whether they answer, not by asking the
//! user which country they are in. A wrong guess is expensive (a key that
//! always fails, a support question), while this probe costs one unauthenticated
//! `GET /models` per provider with a short timeout.
//!
//! The distinction that matters to the user is only three-way:
//!
//! - [`Reach::Reachable`] — the service answered. Its *authorization* answer
//!   (401/402/429) still proves the network path works; the key is a separate
//!   question the Test button answers.
//! - [`Reach::Blocked`] — the service refused this network or origin (403).
//!   No key can fix that, so the card says so and points at another row.
//! - [`Reach::Unreachable`] — nothing answered: no route, DNS failure,
//!   timeout. For a local server this simply means "not running yet".

use std::time::Duration;

use crate::TranslateError;

/// Outcome of one reachability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Reachable,
    Blocked,
    Unreachable,
}

/// How long a probe may take before it counts as "no answer".
///
/// The card runs four of these at once on a background thread, so the whole
/// list settles in this time rather than four times it.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Ask `{base_url}/models` whether this provider answers from here.
///
/// No key is sent: the question is about the network path, and several
/// endpoints (OpenRouter) publish `/models` publicly. A provider that
/// requires a key for it still answers 401, which is a reachable service.
pub fn probe(base_url: &str) -> Reach {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Reach::Unreachable;
    }
    let url = format!("{base}/models");

    let client = match reqwest::blocking::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .connect_timeout(PROBE_TIMEOUT)
        .user_agent("dicto-translate/0.1")
        .build()
    {
        Ok(client) => client,
        Err(_) => return Reach::Unreachable,
    };

    match client.get(&url).send() {
        Ok(resp) => classify(resp.status().as_u16()),
        // No route, DNS failure, TLS failure, timeout — the service did not
        // answer at all.
        Err(_) => Reach::Unreachable,
    }
}

/// Map an HTTP status onto the three outcomes the card can act on.
///
/// Split out so the rule is testable without a network.
pub fn classify(status: u16) -> Reach {
    match status {
        200..=299 => Reach::Reachable,
        // The service answered and told us why it will not proceed. Only a
        // key, a payment, or patience fixes these — the network is fine.
        401 | 402 | 408 | 429 => Reach::Reachable,
        // Refused on the basis of who is asking: geo-block, origin policy.
        403 | 451 => Reach::Blocked,
        // Anything else is a server-side answer, not a network problem.
        _ => Reach::Reachable,
    }
}

/// Turn a probe failure into the message the card shows under a row.
pub fn describe(reach: Reach, local: bool) -> Option<&'static str> {
    match reach {
        Reach::Reachable => None,
        Reach::Blocked => Some("Blocked on your network — a VPN is required."),
        Reach::Unreachable if local => Some("No answer — is the server running?"),
        Reach::Unreachable => Some("No answer from this network."),
    }
}

/// Probe every provider at once and return the results in the input order.
///
/// One thread per provider keeps the card responsive: the whole list settles
/// in about one probe timeout instead of the sum of four.
pub fn probe_all(base_urls: &[&str]) -> Vec<Reach> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = base_urls
            .iter()
            .map(|url| scope.spawn(move || probe(url)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(Reach::Unreachable))
            .collect()
    })
}

/// Classify an error from a real request, so a failed Test can say whether
/// the cause was the network or the key.
///
/// The API errors carry the status in their message ("HTTP 403: …", "API
/// error 403 Forbidden: …"), which is the only place it survives the
/// `reqwest` error type. Anything without a recognisable status is left to
/// the caller's own wording.
pub fn classify_error(err: &TranslateError) -> Option<Reach> {
    let TranslateError::Api(message) = err else {
        return match err {
            TranslateError::Http(_) => Some(Reach::Unreachable),
            _ => None,
        };
    };
    for token in message.split(|c: char| !c.is_ascii_digit()) {
        if token.len() == 3
            && let Ok(code) = token.parse::<u16>()
            && (100..600).contains(&code)
        {
            return Some(classify(code));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_answer_is_reachable() {
        assert_eq!(classify(200), Reach::Reachable);
        assert_eq!(classify(204), Reach::Reachable);
    }

    #[test]
    fn an_authorization_answer_still_proves_the_network_works() {
        for status in [401, 402, 408, 429] {
            assert_eq!(classify(status), Reach::Reachable, "{status}");
        }
    }

    #[test]
    fn a_refusal_based_on_who_is_asking_is_blocked() {
        assert_eq!(classify(403), Reach::Blocked);
        assert_eq!(classify(451), Reach::Blocked);
    }

    #[test]
    fn a_server_side_failure_is_not_a_network_problem() {
        assert_eq!(classify(500), Reach::Reachable);
        assert_eq!(classify(404), Reach::Reachable);
    }

    #[test]
    fn only_a_problem_gets_a_message() {
        assert_eq!(describe(Reach::Reachable, false), None);
        assert!(describe(Reach::Blocked, false).is_some());
    }

    #[test]
    fn a_local_server_that_does_not_answer_is_not_a_network_block() {
        let local = describe(Reach::Unreachable, true).unwrap();
        assert!(local.contains("running"), "{local}");
        let remote = describe(Reach::Unreachable, false).unwrap();
        assert!(!remote.contains("running"), "{remote}");
    }

    #[test]
    fn an_empty_url_is_never_reachable() {
        assert_eq!(probe(""), Reach::Unreachable);
        assert_eq!(probe("   "), Reach::Unreachable);
    }

    #[test]
    fn a_failed_request_reports_the_status_it_carried() {
        let blocked = TranslateError::Api("HTTP 403 Forbidden: nope".into());
        assert_eq!(classify_error(&blocked), Some(Reach::Blocked));
        let denied = TranslateError::Api("API error 401 Unauthorized: bad key".into());
        assert_eq!(classify_error(&denied), Some(Reach::Reachable));
        let shape = TranslateError::Parse("unexpected".into());
        assert_eq!(classify_error(&shape), None);
    }

    #[test]
    fn probing_a_closed_port_reports_unreachable() {
        // The discarded port on the loopback: nothing listens, so the
        // connection is refused instead of waiting out the timeout.
        assert_eq!(probe("http://127.0.0.1:9/v1"), Reach::Unreachable);
    }
}
