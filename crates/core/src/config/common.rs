use crate::models::ProxyNode;

pub(crate) fn skipped_derived_warning(tag: &str, skipped: usize) -> Option<String> {
    (skipped > 0).then(|| {
        format!("DNS: no server tagged '{tag}' - skipping {skipped} auto-derived domain entries")
    })
}

pub(crate) fn outbound_tag(node: &ProxyNode, index: usize) -> String {
    match node.remark() {
        Some(name) if !name.is_empty() => format!("proxy-{index}-{name}"),
        _ => format!("proxy-{index}"),
    }
}

/// Removes exactly one leading `*.` from a domain pattern, leaving any
/// residual wildcard intact.
pub(crate) fn strip_suffix_wildcard(pattern: &str) -> &str {
    pattern.strip_prefix("*.").unwrap_or(pattern)
}

#[cfg(test)]
mod tests {
    use super::strip_suffix_wildcard;

    #[test]
    fn strips_one_leading_wildcard() {
        assert_eq!(strip_suffix_wildcard("*.google.com"), "google.com");
    }

    #[test]
    fn plain_name_unchanged() {
        assert_eq!(strip_suffix_wildcard("google.com"), "google.com");
    }

    #[test]
    fn strips_only_once() {
        assert_eq!(strip_suffix_wildcard("*.*.example.com"), "*.example.com");
    }

    #[test]
    fn empty_and_bare_star_unchanged() {
        assert_eq!(strip_suffix_wildcard(""), "");
        assert_eq!(strip_suffix_wildcard("*"), "*");
    }
}
