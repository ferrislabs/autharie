use axum::http::Method;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Segment {
    Literal(&'static str),
    Organisation,
    Param,
}

use Segment::{Literal, Organisation, Param};

const USER_ORGANISATIONS: &[Segment] =
    &[Literal("users"), Literal("@me"), Literal("organisations")];

const ALLOWLIST: &[(&str, &[Segment])] = &[
    ("GET", USER_ORGANISATIONS),
    (
        "GET",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
        ],
    ),
    (
        "GET",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
        ],
    ),
    (
        "GET",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
            Literal("availability"),
        ],
    ),
    (
        "GET",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
            Literal("usage-metrics"),
            Param,
        ],
    ),
    (
        "GET",
        &[
            Literal("organisations"),
            Organisation,
            Literal("logs"),
            Literal("search"),
        ],
    ),
    (
        "GET",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
            Literal("backups"),
        ],
    ),
    (
        "POST",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
        ],
    ),
    (
        "POST",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
            Literal("upgrade"),
        ],
    ),
    (
        "GET",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
            Literal("webhooks"),
        ],
    ),
    (
        "POST",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
            Literal("webhooks"),
        ],
    ),
    (
        "DELETE",
        &[
            Literal("organisations"),
            Organisation,
            Literal("deployments"),
            Param,
            Literal("webhooks"),
            Param,
        ],
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    UserOrganisations,
    Organisation(Uuid),
}

fn matches(
    allowed_method: &str,
    segments: &[Segment],
    method: &Method,
    parts: &[&str],
) -> Option<Target> {
    if allowed_method != method.as_str() || segments.len() != parts.len() {
        return None;
    }

    let mut organisation = None;
    for (segment, part) in segments.iter().zip(parts) {
        match segment {
            Literal(expected) if expected == part => {}
            Param if !part.is_empty() => {}
            Organisation => organisation = Some(Uuid::parse_str(part).ok()?),
            _ => return None,
        }
    }

    Some(match organisation {
        Some(id) => Target::Organisation(id),
        None => Target::UserOrganisations,
    })
}

pub fn classify(method: &Method, path: &str) -> Option<Target> {
    let parts: Vec<&str> = path.strip_prefix('/')?.split('/').collect();

    ALLOWLIST
        .iter()
        .find_map(|(allowed, segments)| matches(allowed, segments, method, &parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORG: &str = "6f1c1b0e-5a53-4b9b-8d1e-0a5f3f1f9c11";
    const DEP: &str = "0b9a7a52-2d35-4a6f-9a0e-3c3a5f6d7e88";

    fn org() -> Uuid {
        Uuid::parse_str(ORG).unwrap()
    }

    #[test]
    fn the_organisation_list_is_reachable() {
        assert_eq!(
            classify(&Method::GET, "/users/@me/organisations"),
            Some(Target::UserOrganisations)
        );
    }

    #[test]
    fn every_route_the_mcp_server_calls_is_reachable() {
        let reachable = [
            (Method::GET, format!("/organisations/{ORG}/deployments")),
            (
                Method::GET,
                format!("/organisations/{ORG}/deployments/{DEP}"),
            ),
            (
                Method::GET,
                format!("/organisations/{ORG}/deployments/{DEP}/availability"),
            ),
            (
                Method::GET,
                format!("/organisations/{ORG}/deployments/{DEP}/usage-metrics/cpu"),
            ),
            (Method::GET, format!("/organisations/{ORG}/logs/search")),
            (
                Method::GET,
                format!("/organisations/{ORG}/deployments/{DEP}/backups"),
            ),
            (Method::POST, format!("/organisations/{ORG}/deployments")),
            (
                Method::POST,
                format!("/organisations/{ORG}/deployments/{DEP}/upgrade"),
            ),
            (
                Method::GET,
                format!("/organisations/{ORG}/deployments/{DEP}/webhooks"),
            ),
            (
                Method::POST,
                format!("/organisations/{ORG}/deployments/{DEP}/webhooks"),
            ),
            (
                Method::DELETE,
                format!("/organisations/{ORG}/deployments/{DEP}/webhooks/{DEP}"),
            ),
        ];

        for (method, path) in reachable {
            assert_eq!(
                classify(&method, &path),
                Some(Target::Organisation(org())),
                "{method} {path}"
            );
        }
    }

    #[test]
    fn a_delegated_token_cannot_change_the_organisation() {
        for method in [Method::PATCH, Method::PUT, Method::POST, Method::DELETE] {
            assert_eq!(
                classify(&method, &format!("/organisations/{ORG}")),
                None,
                "{method}"
            );
        }
        assert_eq!(
            classify(&Method::GET, &format!("/organisations/{ORG}")),
            None
        );
    }

    #[test]
    fn the_method_must_match() {
        assert_eq!(
            classify(
                &Method::DELETE,
                &format!("/organisations/{ORG}/deployments/{DEP}")
            ),
            None
        );
        assert_eq!(
            classify(
                &Method::PATCH,
                &format!("/organisations/{ORG}/deployments/{DEP}")
            ),
            None
        );
        assert_eq!(classify(&Method::POST, "/users/@me/organisations"), None);
        assert_eq!(
            classify(
                &Method::POST,
                &format!("/organisations/{ORG}/deployments/{DEP}/backups")
            ),
            None
        );
    }

    #[test]
    fn routes_the_mcp_server_does_not_call_are_closed() {
        let closed = [
            format!("/organisations/{ORG}/members"),
            format!("/organisations/{ORG}/roles"),
            format!("/organisations/{ORG}/invitations"),
            format!("/organisations/{ORG}/billing/account"),
            format!("/organisations/{ORG}/cloud-credentials"),
            format!("/organisations/{ORG}/audit-log"),
            format!("/organisations/{ORG}/deployments/{DEP}/upgrade/extra"),
            format!("/organisations/{ORG}/deployments/{DEP}/iam-settings"),
            "/platform/organisations".to_string(),
            "/users/@me".to_string(),
            "/organisations".to_string(),
            "/".to_string(),
            String::new(),
        ];

        for path in closed {
            assert_eq!(classify(&Method::GET, &path), None, "GET {path}");
            assert_eq!(classify(&Method::POST, &path), None, "POST {path}");
        }
    }

    #[test]
    fn a_path_that_names_no_organisation_is_closed() {
        for path in [
            "/organisations/not-a-uuid/deployments",
            "/organisations/../deployments",
            "/organisations//deployments",
        ] {
            assert_eq!(classify(&Method::GET, path), None, "{path}");
        }
    }

    #[test]
    fn a_trailing_slash_or_an_empty_parameter_is_closed() {
        assert_eq!(
            classify(&Method::GET, &format!("/organisations/{ORG}/deployments/")),
            None
        );
        assert_eq!(
            classify(
                &Method::GET,
                &format!("/organisations/{ORG}/deployments//backups")
            ),
            None
        );
    }
}
