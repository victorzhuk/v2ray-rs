use std::collections::HashSet;

use v2ray_rs_core::models::BackendType;

use crate::connection::strip_leading_timestamp;

const MAX_QUOTE_CHARS: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PatternId {
    Deprecated,
    RealityMitm,
}

const PATTERNS: &[(BackendType, PatternId, &[&str])] = &[
    (BackendType::Xray, PatternId::Deprecated, &["is deprecated"]),
    (
        BackendType::V2ray,
        PatternId::Deprecated,
        &["is deprecated"],
    ),
    (
        BackendType::Xray,
        PatternId::RealityMitm,
        &["potential MITM or redirection"],
    ),
    (
        BackendType::SingBox,
        PatternId::Deprecated,
        &["WARN", "deprecated"],
    ),
];

pub(crate) fn match_backend_warning(backend: BackendType, line: &str) -> Option<PatternId> {
    PATTERNS
        .iter()
        .find(|(b, _, needles)| *b == backend && needles.iter().all(|n| line.contains(n)))
        .map(|&(_, id, _)| id)
}

pub(crate) fn warning_toast(line: &str) -> String {
    let quote = strip_leading_timestamp(line.trim());
    if quote.chars().count() > MAX_QUOTE_CHARS {
        let head: String = quote.chars().take(MAX_QUOTE_CHARS - 1).collect();
        format!("Backend warning: {head}…")
    } else {
        format!("Backend warning: {quote}")
    }
}

/// The classified cause of a candidate failure, matched against the whole
/// captured backend output when the candidate is marked failed. Table-driven so
/// future signatures (DNS bootstrap timeout, TLS verification refusal) slot in
/// without touching the reporting path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FailureClassification {
    pub cause: &'static str,
}

/// Signatures of known failure modes: `(backend, all-needles, classification)`.
/// A signature matches when every needle appears in the captured output.
const FAILURE_SIGNATURES: &[(BackendType, &[&str], &str)] = &[(
    BackendType::Xray,
    &["REALITY: received real certificate"],
    "server rejected REALITY authentication; the subscription's REALITY keys may be stale",
)];

/// Classifies a candidate failure from its captured backend output, or `None`
/// when nothing matches and the generic report applies. One call over one
/// candidate's output yields at most one classification, however many times the
/// backend repeated the signature.
pub(crate) fn classify_failure(
    backend: BackendType,
    captured_output: &str,
) -> Option<FailureClassification> {
    FAILURE_SIGNATURES
        .iter()
        .filter(|(b, needles, _)| {
            *b == backend && needles.iter().all(|n| captured_output.contains(n))
        })
        .map(|(_, _, cause)| FailureClassification { cause })
        .next()
}

/// Remembers which patterns already toasted, so a warning the backend repeats
/// for every connection it opens surfaces once per app connection.
pub(crate) struct BackendWarnings {
    backend: BackendType,
    seen: HashSet<PatternId>,
}

impl BackendWarnings {
    pub(crate) fn new(backend: BackendType) -> Self {
        Self {
            backend,
            seen: HashSet::new(),
        }
    }

    pub(crate) fn toast_for(&mut self, line: &str) -> Option<String> {
        let id = match_backend_warning(self.backend, line)?;
        self.seen.insert(id).then(|| warning_toast(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XRAY_WEBSOCKET: &str = "2026/09/15 08:51:54.138024 [Warning] common/errors: The feature WebSocket transport (with ALPN http/1.1, etc.) is deprecated, not recommended for using and might be removed. Please migrate to XHTTP H2 & H3 as soon as possible.";
    const XRAY_REALITY: &str = "2026/09/14 09:57:30.532567 [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)";
    const SINGBOX_DEPRECATED: &str = "WARN[0000] implicit default HTTP client using default outbound for remote rule-sets is deprecated in sing-box 1.14.0 and will be removed in sing-box 1.16.0.";

    #[test]
    fn matches_xray_websocket_deprecation() {
        assert_eq!(
            match_backend_warning(BackendType::Xray, XRAY_WEBSOCKET),
            Some(PatternId::Deprecated)
        );
        assert_eq!(
            match_backend_warning(BackendType::V2ray, "[Warning] feature X is deprecated"),
            Some(PatternId::Deprecated)
        );
    }

    #[test]
    fn matches_xray_reality_mitm() {
        assert_eq!(
            match_backend_warning(BackendType::Xray, XRAY_REALITY),
            Some(PatternId::RealityMitm)
        );
    }

    #[test]
    fn matches_singbox_warn_deprecation() {
        assert_eq!(
            match_backend_warning(BackendType::SingBox, SINGBOX_DEPRECATED),
            Some(PatternId::Deprecated)
        );
        assert_eq!(
            match_backend_warning(BackendType::SingBox, "INFO[0000] feature is deprecated"),
            None
        );
    }

    #[test]
    fn ignores_ordinary_xray_dial_warning() {
        for line in [
            "2026/09/14 09:57:31.000000 [Warning] [123] app/dispatcher: failed to dial tcp 203.0.113.1:443 > dial tcp: i/o timeout",
            "[Warning] core: Xray 26.9.9 started",
        ] {
            assert_eq!(match_backend_warning(BackendType::Xray, line), None);
        }
    }

    #[test]
    fn ignores_patterns_of_other_backends() {
        assert_eq!(
            match_backend_warning(BackendType::SingBox, XRAY_WEBSOCKET),
            None
        );
        assert_eq!(
            match_backend_warning(BackendType::V2ray, XRAY_REALITY),
            None
        );
    }

    #[test]
    fn toast_drops_xray_timestamp() {
        assert_eq!(
            warning_toast(XRAY_REALITY),
            "Backend warning: [Error] [1944052120] transport/internet/reality: REALITY: received real certificate (potential MITM or redirection)"
        );
        assert!(
            warning_toast(SINGBOX_DEPRECATED).starts_with("Backend warning: WARN[0000] implicit")
        );
    }

    #[test]
    fn toast_truncates_to_160_chars() {
        let toast = warning_toast(XRAY_WEBSOCKET);
        let quote = toast.strip_prefix("Backend warning: ").unwrap();
        assert_eq!(quote.chars().count(), 160);
        assert!(quote.starts_with("[Warning] common/errors: The feature WebSocket"));
        assert!(quote.ends_with('…'));

        let short = "x".repeat(160);
        assert_eq!(warning_toast(&short), format!("Backend warning: {short}"));
    }

    #[test]
    fn toasts_once_per_pattern() {
        let mut warnings = BackendWarnings::new(BackendType::Xray);
        assert!(warnings.toast_for(XRAY_REALITY).is_some());
        assert_eq!(warnings.toast_for(XRAY_REALITY), None);
        assert_eq!(warnings.toast_for(XRAY_REALITY), None);
        assert!(warnings.toast_for(XRAY_WEBSOCKET).is_some());
        assert_eq!(warnings.toast_for(XRAY_WEBSOCKET), None);
    }

    #[test]
    fn classifies_reality_auth_rejection() {
        let classification = classify_failure(BackendType::Xray, XRAY_REALITY)
            .expect("the REALITY signature classifies");
        assert_eq!(
            classification.cause,
            "server rejected REALITY authentication; the subscription's REALITY keys may be stale"
        );
    }

    #[test]
    fn classification_is_once_per_output_regardless_of_repeats() {
        // The backend repeats the signature on every connection it opens before
        // the candidate gives up; one output still classifies exactly once.
        let output = format!("{XRAY_REALITY}\n{XRAY_REALITY}\n{XRAY_REALITY}");
        let classification = classify_failure(BackendType::Xray, &output).expect("classified");
        assert_eq!(
            classification.cause,
            "server rejected REALITY authentication; the subscription's REALITY keys may be stale"
        );
    }

    #[test]
    fn unmatched_failure_has_no_classification() {
        assert_eq!(
            classify_failure(
                BackendType::Xray,
                "2026/09/14 09:57:31 [Warning] failed to dial tcp: i/o timeout",
            ),
            None
        );
        assert_eq!(classify_failure(BackendType::Xray, ""), None);
    }

    #[test]
    fn signatures_match_only_their_own_backend() {
        assert_eq!(classify_failure(BackendType::SingBox, XRAY_REALITY), None);
        assert_eq!(classify_failure(BackendType::V2ray, XRAY_REALITY), None);
    }
}
