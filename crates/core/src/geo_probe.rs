//! Data-defined service availability checks and verdict derivation.
//!
//! Each built-in check is a table row: a check URL plus the markers whose
//! presence in the reply's final URL or body marks the node's exit as blocked
//! by the service. The gate logic is a pure function over the reply, so it is
//! unit-testable without a backend, and adding a service is a data change.

use serde::{Deserialize, Serialize};

/// Verdict of one availability check for one node and one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AvailabilityVerdict {
    Available,
    Blocked,
    Unknown,
}

impl AvailabilityVerdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Blocked => "Blocked",
            Self::Unknown => "Unknown",
        }
    }
}

/// One built-in service check.
///
/// `blocked_url_markers` match (case-insensitively) the final URL after
/// redirects; `blocked_body_markers` match the response body. Anything not
/// matching a marker is `available`; transport failures are `unknown` and are
/// decided by the probe runner, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCheck {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub blocked_url_markers: &'static [&'static str],
    pub blocked_body_markers: &'static [&'static str],
}

impl ServiceCheck {
    /// Creates a check, rejecting non-`https://` URLs: the probe must not send
    /// plain-HTTP requests through a node's proxy inbound.
    pub fn new(
        id: &'static str,
        name: &'static str,
        url: &'static str,
        blocked_url_markers: &'static [&'static str],
        blocked_body_markers: &'static [&'static str],
    ) -> Result<Self, String> {
        if !url.starts_with("https://") {
            return Err(format!("service check URL must be https://, got {url}"));
        }
        Ok(Self {
            id,
            name,
            url,
            blocked_url_markers,
            blocked_body_markers,
        })
    }
}

/// The built-in service checks, in display order.
pub fn builtin_service_checks() -> &'static [ServiceCheck] {
    BUILTIN_SERVICE_CHECKS
}

static BUILTIN_SERVICE_CHECKS: &[ServiceCheck] = &[
    ServiceCheck {
        id: "notebooklm",
        name: "NotebookLM",
        url: "https://notebooklm.google.com/",
        // A rejected exit is redirected to the main Notebook app with a
        // location=unsupported query; an accepted exit lands on the sign-in
        // flow or the landing page.
        blocked_url_markers: &["location=unsupported"],
        blocked_body_markers: &["not available in your country"],
    },
    ServiceCheck {
        id: "gemini",
        name: "Gemini",
        url: "https://gemini.google.com/about",
        blocked_url_markers: &[],
        // The refusal is served as a 200 landing page whose body carries the
        // availability-refusal string.
        blocked_body_markers: &[
            "not available in your country",
            "isn't available in your country",
        ],
    },
];

/// Derives a verdict for one service check from the reply of the request sent
/// through the node's tunnel.
///
/// `final_url` is the URL after following redirects; `status` is the final
/// HTTP status; `body` is the (capped) response body. Transport failures have
/// no reply at all and are mapped to [`AvailabilityVerdict::Unknown`] by the
/// caller.
pub fn derive_verdict(
    check: &ServiceCheck,
    final_url: &str,
    status: u16,
    body: &str,
) -> AvailabilityVerdict {
    // A non-2xx/3xx final status (rate limit, middlebox block page) is not
    // evidence either way.
    if !(200..400).contains(&status) {
        return AvailabilityVerdict::Unknown;
    }

    let url_lower = final_url.to_lowercase();
    if check
        .blocked_url_markers
        .iter()
        .any(|marker| url_lower.contains(&marker.to_lowercase()))
    {
        return AvailabilityVerdict::Blocked;
    }

    let body_lower = body.to_lowercase();
    if check
        .blocked_body_markers
        .iter()
        .any(|marker| body_lower.contains(&marker.to_lowercase()))
    {
        return AvailabilityVerdict::Blocked;
    }

    AvailabilityVerdict::Available
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(id: &str) -> &ServiceCheck {
        builtin_service_checks()
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no built-in check with id {id}"))
    }

    #[test]
    fn builtin_table_contains_notebooklm_and_gemini_over_https() {
        let checks = builtin_service_checks();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].id, "notebooklm");
        assert_eq!(checks[1].id, "gemini");
        for c in checks {
            assert!(c.url.starts_with("https://"), "{} must be https", c.id);
            assert!(!c.name.is_empty());
        }
    }

    #[test]
    fn new_rejects_non_https_url() {
        let err = ServiceCheck::new("x", "X", "http://example.com/", &[], &[])
            .expect_err("http URL must be rejected");
        assert!(err.contains("https://"));
    }

    #[test]
    fn new_accepts_https_url() {
        assert!(ServiceCheck::new("x", "X", "https://example.com/", &[], &[]).is_ok());
    }

    #[test]
    fn notebooklm_blocked_on_location_unsupported_final_url() {
        let verdict = derive_verdict(
            check("notebooklm"),
            "https://notebook.google/?location=unsupported",
            200,
            "<html>sign in</html>",
        );
        assert_eq!(verdict, AvailabilityVerdict::Blocked);
    }

    #[test]
    fn notebooklm_blocked_on_refusal_body_marker() {
        let verdict = derive_verdict(
            check("notebooklm"),
            "https://notebooklm.google.com/",
            200,
            "Sorry, NotebookLM is not available in your country.",
        );
        assert_eq!(verdict, AvailabilityVerdict::Blocked);
    }

    #[test]
    fn notebooklm_available_on_sign_in_flow() {
        let verdict = derive_verdict(
            check("notebooklm"),
            "https://accounts.google.com/v3/signin/identifier?continue=https%3A%2F%2Fnotebooklm.google.com%2F",
            200,
            "<html>Sign in to continue to NotebookLM</html>",
        );
        assert_eq!(verdict, AvailabilityVerdict::Available);
    }

    #[test]
    fn notebooklm_available_on_plain_landing_page() {
        let verdict = derive_verdict(
            check("notebooklm"),
            "https://notebooklm.google.com/",
            200,
            "<html>NotebookLM</html>",
        );
        assert_eq!(verdict, AvailabilityVerdict::Available);
    }

    #[test]
    fn gemini_blocked_on_refusal_body_marker() {
        let verdict = derive_verdict(
            check("gemini"),
            "https://gemini.google.com/about",
            200,
            "Gemini Apps isn't available in your country or region.",
        );
        assert_eq!(verdict, AvailabilityVerdict::Blocked);
    }

    #[test]
    fn gemini_available_on_plain_about_page() {
        let verdict = derive_verdict(
            check("gemini"),
            "https://gemini.google.com/about",
            200,
            "<html>Gemini — a helpful assistant</html>",
        );
        assert_eq!(verdict, AvailabilityVerdict::Available);
    }

    #[test]
    fn non_success_final_status_is_unknown() {
        let verdict = derive_verdict(
            check("notebooklm"),
            "https://notebooklm.google.com/",
            403,
            "rate limited",
        );
        assert_eq!(verdict, AvailabilityVerdict::Unknown);
    }

    #[test]
    fn verdicts_serialize_lowercase_and_round_trip() {
        for (verdict, wire) in [
            (AvailabilityVerdict::Available, "\"available\""),
            (AvailabilityVerdict::Blocked, "\"blocked\""),
            (AvailabilityVerdict::Unknown, "\"unknown\""),
        ] {
            let json = serde_json::to_string(&verdict).unwrap();
            assert_eq!(json, wire);
            let back: AvailabilityVerdict = serde_json::from_str(&json).unwrap();
            assert_eq!(back, verdict);
        }
    }
}
