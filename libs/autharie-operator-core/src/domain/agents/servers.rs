use std::future::Future;
use std::time::Duration;

use autharie_crds::v1alpha::identity_instance::{
    AgentServerPhase, AgentServerSpec, AgentServerStatus, AgentServerTool, AgentsSpec,
    IdentityInstance,
};
use chrono::{DateTime, Utc};

use super::AgentsError;

pub const MAX_TOOLS: usize = 200;
pub const MAX_TOOL_NAME_CHARS: usize = 64;
pub const MAX_TOOL_DESCRIPTION_CHARS: usize = 512;
pub const MAX_MESSAGE_CHARS: usize = 300;
pub const REDISCOVER_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
pub const RETRY_FAILED_AFTER: Duration = Duration::from_secs(30 * 60);

const RESERVED_SLUGS: [&str; 3] = ["ferriskey", "autharie", "gateway"];

pub fn audience(slug: &str) -> String {
    format!("mcp-{slug}")
}

pub fn is_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    (2..=31).contains(&slug.len())
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !RESERVED_SLUGS.contains(&slug)
}

pub fn is_tool_name(name: &str) -> bool {
    (1..=MAX_TOOL_NAME_CHARS).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

pub fn serving(agents: &AgentsSpec) -> Vec<&AgentServerSpec> {
    agents
        .servers
        .iter()
        .filter(|server| server.enabled)
        .collect()
}

pub fn known(instance: &IdentityInstance) -> &[AgentServerStatus] {
    instance
        .status
        .as_ref()
        .and_then(|status| status.agents.as_ref())
        .map_or(&[], |agents| agents.servers.as_slice())
}

pub fn cut(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub phase: AgentServerPhase,
    pub tools: Vec<AgentServerTool>,
    pub dropped: u32,
    pub message: Option<String>,
}

impl Discovered {
    pub fn failed(phase: AgentServerPhase, message: impl AsRef<str>) -> Self {
        Self {
            phase,
            tools: Vec::new(),
            dropped: 0,
            message: Some(cut(message.as_ref(), MAX_MESSAGE_CHARS)),
        }
    }

    pub fn listed(tools: Vec<AgentServerTool>, mut dropped: u32) -> Self {
        let mut kept: Vec<AgentServerTool> = Vec::new();
        for tool in tools {
            if kept.len() >= MAX_TOOLS
                || !is_tool_name(&tool.name)
                || kept.iter().any(|known| known.name == tool.name)
            {
                dropped = dropped.saturating_add(1);
                continue;
            }
            kept.push(AgentServerTool {
                description: tool
                    .description
                    .map(|text| cut(&text, MAX_TOOL_DESCRIPTION_CHARS)),
                ..tool
            });
        }

        Self {
            phase: AgentServerPhase::Ready,
            tools: kept,
            dropped,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DiscoveryTarget<'a> {
    pub token_url: &'a str,
    pub client_id: &'a str,
    pub client_secret: &'a str,
    pub audience: &'a str,
    pub url: &'a str,
}

pub trait AgentServersPort: Send + Sync {
    fn ensure_server_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        slug: &str,
        canonical_uri: &str,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn remove_server_client(
        &self,
        instance: &IdentityInstance,
        realm: &str,
        slug: &str,
        canonical_uri: &str,
    ) -> impl Future<Output = Result<(), AgentsError>> + Send;

    fn discover(&self, target: &DiscoveryTarget<'_>) -> impl Future<Output = Discovered> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Now,
    After(Duration),
}

fn instant(text: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text?)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

pub fn discovery_due(
    spec: &AgentServerSpec,
    last: Option<&AgentServerStatus>,
    now: DateTime<Utc>,
) -> Due {
    let Some(last) = last.filter(|last| last.id == spec.id) else {
        return Due::Now;
    };
    let Some(discovered_at) = instant(last.discovered_at.as_deref()) else {
        return Due::Now;
    };

    if let Some(asked) = instant(spec.discovery_requested_at.as_deref()) {
        let answered = instant(last.discovery_requested_at.as_deref());
        if answered.is_none_or(|answered| asked > answered) {
            return Due::Now;
        }
    }

    let every = if last.phase == AgentServerPhase::Ready {
        REDISCOVER_AFTER
    } else {
        RETRY_FAILED_AFTER
    };
    let elapsed = (now - discovered_at).to_std().unwrap_or(Duration::ZERO);
    if elapsed >= every {
        Due::Now
    } else {
        Due::After(every - elapsed)
    }
}

pub fn entry(
    spec: &AgentServerSpec,
    found: Discovered,
    started: DateTime<Utc>,
) -> AgentServerStatus {
    AgentServerStatus {
        id: spec.id.clone(),
        slug: spec.slug.clone(),
        phase: found.phase,
        tools: found.tools,
        dropped: found.dropped,
        message: found.message,
        discovered_at: Some(started.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        discovery_requested_at: spec.discovery_requested_at.clone(),
    }
}

pub fn waiting(spec: &AgentServerSpec, message: &str) -> AgentServerStatus {
    AgentServerStatus {
        id: spec.id.clone(),
        slug: spec.slug.clone(),
        phase: AgentServerPhase::Unreachable,
        tools: Vec::new(),
        dropped: 0,
        message: Some(message.to_string()),
        discovered_at: None,
        discovery_requested_at: None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration as Span;

    use super::*;

    const ID: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";

    fn spec(requested: Option<&str>) -> AgentServerSpec {
        AgentServerSpec {
            id: ID.to_string(),
            slug: "crm".to_string(),
            url: "https://tools.example.com/mcp".to_string(),
            enabled: true,
            discovery_requested_at: requested.map(str::to_string),
        }
    }

    fn status(
        phase: AgentServerPhase,
        discovered_at: Option<DateTime<Utc>>,
        answered: Option<&str>,
    ) -> AgentServerStatus {
        AgentServerStatus {
            id: ID.to_string(),
            slug: "crm".to_string(),
            phase,
            tools: Vec::new(),
            dropped: 0,
            message: None,
            discovered_at: discovered_at.map(|at| at.to_rfc3339()),
            discovery_requested_at: answered.map(str::to_string),
        }
    }

    fn tool(name: &str) -> AgentServerTool {
        AgentServerTool {
            name: name.to_string(),
            description: None,
            read_only: false,
            destructive: false,
        }
    }

    #[test]
    fn the_audience_client_of_a_server_is_named_after_its_slug() {
        assert_eq!(audience("crm"), "mcp-crm");
    }

    #[test]
    fn a_slug_follows_the_control_planes_rule() {
        for good in ["crm", "billing-v2", "ab"] {
            assert!(is_slug(good), "{good}");
        }
        for bad in [
            "",
            "a",
            "Crm",
            "1crm",
            "cr_m",
            "ferriskey",
            "autharie",
            "gateway",
        ] {
            assert!(!is_slug(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_server_never_discovered_is_due_at_once() {
        assert_eq!(discovery_due(&spec(None), None, Utc::now()), Due::Now);
        let unfinished = status(AgentServerPhase::Unreachable, None, None);
        assert_eq!(
            discovery_due(&spec(None), Some(&unfinished), Utc::now()),
            Due::Now
        );
    }

    #[test]
    fn a_new_request_makes_a_discovery_due_even_if_the_last_was_a_minute_ago() {
        let now = Utc::now();
        let last = status(
            AgentServerPhase::Ready,
            Some(now - Span::minutes(1)),
            Some("2026-10-23T10:00:00.000Z"),
        );

        assert_eq!(
            discovery_due(&spec(Some("2026-10-23T10:05:00.000Z")), Some(&last), now),
            Due::Now
        );
        assert_ne!(
            discovery_due(&spec(Some("2026-10-23T10:00:00.000Z")), Some(&last), now),
            Due::Now
        );
    }

    #[test]
    fn a_discovery_that_never_named_its_request_is_redone_once_one_is_asked_for() {
        let now = Utc::now();
        let last = status(AgentServerPhase::Ready, Some(now), None);

        assert_eq!(
            discovery_due(&spec(Some("2026-10-23T10:00:00.000Z")), Some(&last), now),
            Due::Now
        );
    }

    #[test]
    fn a_listed_server_is_looked_at_again_after_six_hours() {
        let now = Utc::now();
        let recent = status(AgentServerPhase::Ready, Some(now - Span::hours(2)), None);
        let stale = status(AgentServerPhase::Ready, Some(now - Span::hours(7)), None);

        match discovery_due(&spec(None), Some(&recent), now) {
            Due::After(wait) => assert!(
                wait > Duration::from_secs(3 * 3600 + 59 * 60)
                    && wait <= Duration::from_secs(4 * 3600),
                "{wait:?}"
            ),
            Due::Now => panic!("not due yet"),
        }
        assert_eq!(discovery_due(&spec(None), Some(&stale), now), Due::Now);
    }

    #[test]
    fn a_server_that_failed_is_tried_again_sooner_but_not_at_once() {
        let now = Utc::now();
        let failed = status(
            AgentServerPhase::Unreachable,
            Some(now - Span::minutes(10)),
            None,
        );
        let old_failure = status(
            AgentServerPhase::Refused,
            Some(now - Span::minutes(31)),
            None,
        );

        assert!(matches!(
            discovery_due(&spec(None), Some(&failed), now),
            Due::After(wait) if wait <= RETRY_FAILED_AFTER
        ));
        assert_eq!(
            discovery_due(&spec(None), Some(&old_failure), now),
            Due::Now
        );
    }

    #[test]
    fn a_status_of_another_server_does_not_count() {
        let mut other = status(AgentServerPhase::Ready, Some(Utc::now()), None);
        other.id = "someone-else".to_string();

        assert_eq!(
            discovery_due(&spec(None), Some(&other), Utc::now()),
            Due::Now
        );
    }

    #[test]
    fn a_listing_keeps_two_hundred_tools_and_counts_every_one_it_leaves_out() {
        let tools = (0..500).map(|i| tool(&format!("t{i}"))).collect();

        let found = Discovered::listed(tools, 0);

        assert_eq!(found.phase, AgentServerPhase::Ready);
        assert_eq!(found.tools.len(), 200);
        assert_eq!(found.dropped, 300);
    }

    #[test]
    fn a_listing_drops_names_outside_the_alphabet_and_twins() {
        let tools = vec![
            tool("list_invoices"),
            tool("bad name"),
            tool("a/b"),
            tool(""),
            tool(&"x".repeat(65)),
            tool("list_invoices"),
            tool("ok.v2-beta"),
        ];

        let found = Discovered::listed(tools, 1);

        let names: Vec<_> = found.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["list_invoices", "ok.v2-beta"]);
        assert_eq!(found.dropped, 1 + 5);
    }

    #[test]
    fn a_description_is_cut_to_512_characters() {
        let long = AgentServerTool {
            description: Some("é".repeat(900)),
            ..tool("t")
        };

        let found = Discovered::listed(vec![long], 0);

        assert_eq!(
            found.tools[0].description.as_ref().unwrap().chars().count(),
            512
        );
    }

    #[test]
    fn a_failure_keeps_its_message_short_and_lists_nothing() {
        let found = Discovered::failed(AgentServerPhase::Unreachable, "x".repeat(1000));

        assert_eq!(found.phase, AgentServerPhase::Unreachable);
        assert!(found.tools.is_empty());
        assert_eq!(found.message.unwrap().chars().count(), 300);
    }

    #[test]
    fn an_entry_echoes_the_request_it_answers_and_when_it_started() {
        let started = "2026-10-23T10:00:01.250Z".parse::<DateTime<Utc>>().unwrap();
        let asked = spec(Some("2026-10-23T10:00:00.000Z"));

        let written = entry(&asked, Discovered::listed(vec![tool("t")], 0), started);

        assert_eq!(written.slug, "crm");
        assert_eq!(
            written.discovered_at.as_deref(),
            Some("2026-10-23T10:00:01.250Z")
        );
        assert_eq!(
            written.discovery_requested_at.as_deref(),
            Some("2026-10-23T10:00:00.000Z")
        );
    }
}
