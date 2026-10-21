/// Paths that are platform plumbing -- probes and metric scrapes -- rather
/// than anything a customer's users asked for. Matched on whole trailing
/// segments, so `/healthcheck` or `/api/metrics-export` are not caught.
const OPERATIONAL_SUFFIXES: [&str; 4] = ["/health", "/health/live", "/health/ready", "/metrics"];

/// Whether a request path belongs to the platform itself.
///
/// Any root path prefix (`/api`) and one trailing slash are ignored, because
/// the prefix is a deployment setting and clients are not consistent about the
/// slash.
pub fn is_operational_path(path: &str) -> bool {
    let path = path.strip_suffix('/').unwrap_or(path);
    OPERATIONAL_SUFFIXES
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

/// Whether a span is named after an operational request, in the
/// `"<METHOD> <path>"` form HTTP server spans use. Any other shape is not an
/// HTTP request span, so it is never treated as noise.
pub fn is_operational_span_name(name: &str) -> bool {
    let Some((method, path)) = name.split_once(' ') else {
        return false;
    };
    !method.is_empty()
        && method.bytes().all(|b| b.is_ascii_uppercase())
        && path.starts_with('/')
        && is_operational_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_and_scrapes_are_operational_with_or_without_a_prefix() {
        for path in [
            "/health",
            "/health/",
            "/health/live",
            "/health/ready",
            "/metrics",
            "/api/health",
            "/api/health/live",
            "/api/health/ready/",
            "/api/metrics",
        ] {
            assert!(is_operational_path(path), "{path}");
        }
    }

    #[test]
    fn only_whole_segments_match() {
        for path in [
            "/healthcheck",
            "/api/metrics-export",
            "/realms/x/health-report",
            "/api/realms/x/health/other",
            "/",
            "",
            "health",
        ] {
            assert!(!is_operational_path(path), "{path}");
        }
    }

    #[test]
    fn a_span_name_is_a_method_then_a_path() {
        assert!(is_operational_span_name("GET /api/health/ready"));
        assert!(is_operational_span_name("GET /api/health/live"));
        assert!(is_operational_span_name("GET /api/metrics"));
        assert!(is_operational_span_name("HEAD /health/"));
    }

    #[test]
    fn anything_but_a_method_and_a_path_is_not_operational() {
        for name in [
            "/api/health/ready",
            "get /api/health",
            "GET",
            " /api/health",
            "GET  /api/health",
            "GET /api/healthcheck",
            "SELECT health",
            "GET /api/health extra",
            "",
        ] {
            assert!(!is_operational_span_name(name), "{name:?}");
        }
    }
}
