//! `velosctl` REST helpers (the kubectl analog). The pure URL/kind helpers live
//! here so they can be unit-tested; the binary in `main.rs` wires them to reqwest.

/// Map a singular or plural resource word to its REST path segment.
/// Returns `None` for unknown kinds (fail closed).
pub fn plural_for(kind: &str) -> Option<&'static str> {
    match kind {
        "container" | "containers" => Some("containers"),
        "worker" | "workers" => Some("workers"),
        "lease" | "leases" => Some("leases"),
        _ => None,
    }
}

/// Build a collection URL, optionally with a label selector.
pub fn collection_url(base: &str, plural: &str, label_selector: Option<&str>) -> String {
    let base = base.trim_end_matches('/');
    match label_selector {
        Some(ls) => format!("{base}/api/v1/{plural}?labelSelector={ls}"),
        None => format!("{base}/api/v1/{plural}"),
    }
}

/// Build a single-object URL.
pub fn object_url(base: &str, plural: &str, name: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/api/v1/{plural}/{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plural_normalizes_singular_and_plural() {
        assert_eq!(plural_for("container"), Some("containers"));
        assert_eq!(plural_for("workers"), Some("workers"));
        assert_eq!(plural_for("lease"), Some("leases"));
        assert_eq!(plural_for("widget"), None);
    }

    #[test]
    fn urls_trim_trailing_slash_and_add_selector() {
        assert_eq!(
            collection_url("http://h:8080/", "containers", None),
            "http://h:8080/api/v1/containers"
        );
        assert_eq!(
            collection_url("http://h:8080", "containers", Some("team=a")),
            "http://h:8080/api/v1/containers?labelSelector=team=a"
        );
        assert_eq!(
            object_url("http://h:8080/", "workers", "w1"),
            "http://h:8080/api/v1/workers/w1"
        );
    }
}
