use std::fmt::Write;

use autharie_crds::v1alpha::identity_instance::{AgentAccessSpec, IdentityInstance};

use super::{FERRISKEY_SERVER_NAME, GATEWAY_CLIENT_ID, evaluator_audience};
use crate::infrastructure::ferriskey_admin::api_base_url;
use crate::infrastructure::identity_instance::{ferriskey_api_base_url, public_origin};

const FERRISKEY_MCP_SERVICE: &str = "ferriskey-mcp";
const FERRISKEY_MCP_PORT: u16 = 8080;
const JWKS_REFRESH_SECS: u64 = 300;
const PDP_TIMEOUT_MS: u64 = 2000;

#[derive(Clone, PartialEq, Eq)]
pub struct Upstream {
    pub canonical_uri: String,
    pub issuer: String,
    pub jwks_uri: String,
    pub evaluation_url: String,
    pub evaluations_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub token_url: String,
    pub audience: String,
    pub upstream_url: String,
    pub server_name: String,
    pub scopes_supported: Vec<String>,
    pub access: AgentAccessSpec,
}

impl std::fmt::Debug for Upstream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Upstream")
            .field("canonical_uri", &self.canonical_uri)
            .field("server_name", &self.server_name)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsEnvironment {
    pub release_namespace: String,
    pub public_https_port: Option<u16>,
}

impl AgentsEnvironment {
    pub fn new(release_namespace: impl Into<String>, public_https_port: Option<u16>) -> Self {
        Self {
            release_namespace: release_namespace.into(),
            public_https_port,
        }
    }

    pub fn canonical_uri(&self, instance: &IdentityInstance) -> String {
        format!(
            "{}/mcp",
            public_origin(&instance.spec.hostname, self.public_https_port)
        )
    }

    fn public_api_base(&self, instance: &IdentityInstance) -> String {
        let base = ferriskey_api_base_url(instance, self.public_https_port);
        let base = base.trim_end_matches('/');
        if base.starts_with('/') {
            format!(
                "{}{base}",
                public_origin(&instance.spec.hostname, self.public_https_port)
            )
        } else {
            base.to_string()
        }
    }

    pub fn upstream(&self, instance: &IdentityInstance, secret: &str) -> Option<Upstream> {
        let agents = instance.spec.agents.as_ref()?;
        let name = instance.metadata.name.as_deref()?;
        let namespace = instance.metadata.namespace.as_deref()?;
        let realm = agents.realm.as_str();
        let cluster = format!("{}/realms/{realm}", api_base_url(name, namespace));

        Some(Upstream {
            canonical_uri: self.canonical_uri(instance),
            issuer: format!("{}/realms/{realm}", self.public_api_base(instance)),
            jwks_uri: format!("{cluster}/protocol/openid-connect/certs"),
            evaluation_url: format!("{cluster}/authzen/access/v1/evaluation"),
            evaluations_url: format!("{cluster}/authzen/access/v1/evaluations"),
            client_id: GATEWAY_CLIENT_ID.to_string(),
            client_secret: secret.to_string(),
            token_url: format!("{cluster}/protocol/openid-connect/token"),
            audience: evaluator_audience(realm),
            upstream_url: format!(
                "http://{FERRISKEY_MCP_SERVICE}.{}.svc:{FERRISKEY_MCP_PORT}/instances/{namespace}/{name}/mcp",
                self.release_namespace
            ),
            server_name: FERRISKEY_SERVER_NAME.to_string(),
            scopes_supported: Vec::new(),
            access: agents.access.clone(),
        })
    }
}

fn quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn quoted_list(values: &[String]) -> String {
    let mut sorted: Vec<&String> = values.iter().collect();
    sorted.sort();
    sorted.dedup();
    sorted
        .into_iter()
        .map(|value| quoted(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_access(out: &mut String, access: &AgentAccessSpec) {
    let mode = if access.approval_required {
        "approval"
    } else {
        "open"
    };
    let _ = write!(
        out,
        "\n[upstreams.access]\nmode = {}\napproved = [{}]\nblocked = [{}]\n",
        quoted(mode),
        quoted_list(&access.approved),
        quoted_list(&access.blocked),
    );
}

pub fn render(upstreams: &[Upstream]) -> String {
    let mut sorted: Vec<&Upstream> = upstreams.iter().collect();
    sorted.sort_by(|a, b| a.canonical_uri.cmp(&b.canonical_uri));

    let mut out = String::new();
    let _ = writeln!(out, "jwks_refresh_secs = {JWKS_REFRESH_SECS}");
    let _ = writeln!(out, "pdp_timeout_ms = {PDP_TIMEOUT_MS}");
    if sorted.is_empty() {
        out.push_str("upstreams = []\n");
        return out;
    }

    for upstream in sorted {
        let scopes = upstream
            .scopes_supported
            .iter()
            .map(|scope| quoted(scope))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(
            out,
            "\n[[upstreams]]\ncanonical_uri = {}\nissuer = {}\njwks_uri = {}\nupstream_url = {}\nserver_name = {}\nscopes_supported = [{scopes}]\n\n[upstreams.pdp]\nevaluation_url = {}\nevaluations_url = {}\nclient_id = {}\nclient_secret = {}\n\n[upstreams.exchange]\ntoken_url = {}\nclient_id = {}\nclient_secret = {}\naudience = {}\n",
            quoted(&upstream.canonical_uri),
            quoted(&upstream.issuer),
            quoted(&upstream.jwks_uri),
            quoted(&upstream.upstream_url),
            quoted(&upstream.server_name),
            quoted(&upstream.evaluation_url),
            quoted(&upstream.evaluations_url),
            quoted(&upstream.client_id),
            quoted(&upstream.client_secret),
            quoted(&upstream.token_url),
            quoted(&upstream.client_id),
            quoted(&upstream.client_secret),
            quoted(&upstream.audience),
        );
        render_access(&mut out, &upstream.access);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agents::test_spec;
    use autharie_crds::v1alpha::identity_instance::{AgentsSpec, IdentityInstanceSpec};

    fn instance(name: &str, namespace: &str, host: &str, realm: &str) -> IdentityInstance {
        let spec: IdentityInstanceSpec = test_spec(host);
        let mut instance = IdentityInstance::new(name, spec);
        instance.metadata.namespace = Some(namespace.to_string());
        instance.spec.agents = Some(AgentsSpec {
            enabled: true,
            realm: realm.to_string(),
            ..AgentsSpec::default()
        });
        instance
    }

    fn environment() -> AgentsEnvironment {
        AgentsEnvironment::new("autharie-system", None)
    }

    fn upstream(host: &str) -> Upstream {
        environment()
            .upstream(
                &instance("deployment-1", "tenant-a", host, "acme"),
                "s3cret",
            )
            .expect("an upstream")
    }

    #[test]
    fn the_canonical_uri_is_the_mcp_path_of_the_public_host() {
        let instance = instance("deployment-1", "tenant-a", "auth.acme.test", "acme");

        assert_eq!(
            environment().canonical_uri(&instance),
            "https://auth.acme.test/mcp"
        );
        assert_eq!(
            AgentsEnvironment::new("autharie-system", Some(8443)).canonical_uri(&instance),
            "https://auth.acme.test:8443/mcp"
        );
    }

    #[test]
    fn an_upstream_names_the_public_issuer_and_the_in_cluster_endpoints() {
        let found = upstream("auth.acme.test");

        assert_eq!(found.canonical_uri, "https://auth.acme.test/mcp");
        assert_eq!(found.issuer, "https://auth.acme.test/api/realms/acme");
        assert_eq!(
            found.jwks_uri,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/protocol/openid-connect/certs"
        );
        assert_eq!(
            found.evaluation_url,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluation"
        );
        assert_eq!(
            found.evaluations_url,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluations"
        );
        assert_eq!(
            found.token_url,
            "http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/protocol/openid-connect/token"
        );
    }

    #[test]
    fn an_upstream_exchanges_for_the_realm_api_and_reaches_the_instance_mcp_server() {
        let found = upstream("auth.acme.test");

        assert_eq!(found.audience, "acme-realm");
        assert_eq!(found.client_id, "mcp-gateway");
        assert_eq!(found.client_secret, "s3cret");
        assert_eq!(found.server_name, "ferriskey");
        assert_eq!(
            found.upstream_url,
            "http://ferriskey-mcp.autharie-system.svc:8080/instances/tenant-a/deployment-1/mcp"
        );
    }

    #[test]
    fn a_pooled_cell_issues_under_its_own_host() {
        let mut pooled = instance("deployment-1", "tenant-a", "acme.zone.test", "acme");
        pooled.spec.ferriskey = Some(
            serde_json::from_value(serde_json::json!({ "pooledCell": true }))
                .expect("a ferriskey config"),
        );

        let found = environment().upstream(&pooled, "s").expect("an upstream");

        assert_eq!(found.issuer, "https://acme.zone.test/api/realms/acme");
    }

    #[test]
    fn an_instance_without_agents_has_no_upstream() {
        let mut plain = instance("deployment-1", "tenant-a", "auth.acme.test", "acme");
        plain.spec.agents = None;

        assert_eq!(environment().upstream(&plain, "s"), None);
    }

    #[test]
    fn no_upstream_renders_an_empty_list() {
        let text = render(&[]);

        assert!(text.contains("upstreams = []"));
        assert!(!text.contains("[[upstreams]]"));
    }

    #[test]
    fn the_gateway_settings_come_first_so_they_stay_top_level() {
        let text = render(&[upstream("auth.acme.test")]);

        let first_table = text.find("[[upstreams]]").expect("a table");
        assert!(text.find("jwks_refresh_secs = 300").expect("a setting") < first_table);
        assert!(text.find("pdp_timeout_ms = 2000").expect("a setting") < first_table);
    }

    #[test]
    fn an_upstream_renders_every_field_of_the_gateway_contract() {
        let text = render(&[upstream("auth.acme.test")]);

        for line in [
            "[[upstreams]]",
            "canonical_uri = \"https://auth.acme.test/mcp\"",
            "issuer = \"https://auth.acme.test/api/realms/acme\"",
            "upstream_url = \"http://ferriskey-mcp.autharie-system.svc:8080/instances/tenant-a/deployment-1/mcp\"",
            "server_name = \"ferriskey\"",
            "scopes_supported = []",
            "[upstreams.pdp]",
            "evaluation_url = \"http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluation\"",
            "evaluations_url = \"http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/authzen/access/v1/evaluations\"",
            "client_id = \"mcp-gateway\"",
            "client_secret = \"s3cret\"",
            "[upstreams.exchange]",
            "token_url = \"http://deployment-1-api.tenant-a.svc:3333/api/realms/acme/protocol/openid-connect/token\"",
            "audience = \"acme-realm\"",
        ] {
            assert!(text.lines().any(|l| l == line), "{line}\n{text}");
        }
    }

    #[test]
    fn the_pdp_and_exchange_tables_follow_their_upstream() {
        let text = render(&[upstream("auth.acme.test")]);

        let upstream_at = text.find("[[upstreams]]").expect("table");
        let pdp_at = text.find("[upstreams.pdp]").expect("table");
        let exchange_at = text.find("[upstreams.exchange]").expect("table");
        assert!(upstream_at < pdp_at && pdp_at < exchange_at);
    }

    #[test]
    fn upstreams_are_rendered_in_a_stable_order_whatever_the_input_order() {
        let a = upstream("a.acme.test");
        let b = upstream("b.acme.test");

        assert_eq!(render(&[a.clone(), b.clone()]), render(&[b, a]));
    }

    #[test]
    fn two_upstreams_make_two_tables() {
        let text = render(&[upstream("a.acme.test"), upstream("b.acme.test")]);

        assert_eq!(text.matches("[[upstreams]]").count(), 2);
        assert!(text.find("a.acme.test/mcp").unwrap() < text.find("b.acme.test/mcp").unwrap());
    }

    #[test]
    fn strings_are_escaped() {
        let mut found = upstream("auth.acme.test");
        found.client_secret = "a\"b\\c\nd".to_string();

        let text = render(&[found]);

        assert!(text.contains("client_secret = \"a\\\"b\\\\c\\nd\""));
    }

    fn rendered_access(access: AgentAccessSpec) -> toml::Value {
        let mut found = upstream("auth.acme.test");
        found.access = access;

        let parsed: toml::Value = toml::from_str(&render(&[found])).expect("valid TOML");
        parsed["upstreams"][0]["access"].clone()
    }

    #[test]
    fn an_upstream_without_approval_renders_open_and_empty() {
        let access = rendered_access(AgentAccessSpec::default());

        assert_eq!(access["mode"].as_str(), Some("open"));
        assert!(access["approved"].as_array().unwrap().is_empty());
        assert!(access["blocked"].as_array().unwrap().is_empty());
    }

    #[test]
    fn spec_ar_3_approval_mode_renders_the_approved_and_blocked_lists_sorted() {
        let access = rendered_access(AgentAccessSpec {
            approval_required: true,
            approved: vec!["zed".to_string(), "alpha".to_string(), "zed".to_string()],
            blocked: vec![
                "https://cimd.example/c.json".to_string(),
                "a\"b".to_string(),
            ],
        });

        assert_eq!(access["mode"].as_str(), Some("approval"));
        let approved: Vec<_> = access["approved"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let blocked: Vec<_> = access["blocked"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(approved, vec!["alpha", "zed"]);
        assert_eq!(blocked, vec!["a\"b", "https://cimd.example/c.json"]);
    }

    #[test]
    fn the_access_table_follows_its_upstream_and_a_change_changes_the_document() {
        let open = upstream("auth.acme.test");
        let mut closed = open.clone();
        closed.access.approval_required = true;

        assert_ne!(render(&[open]), render(&[closed]));
    }

    #[test]
    fn the_upstream_of_an_instance_carries_its_access_lists() {
        let mut found = instance("deployment-1", "tenant-a", "auth.acme.test", "acme");
        found.spec.agents.as_mut().unwrap().access = AgentAccessSpec {
            approval_required: true,
            approved: vec!["a".to_string()],
            blocked: vec![],
        };

        let upstream = environment().upstream(&found, "s").expect("an upstream");

        assert!(upstream.access.approval_required);
        assert_eq!(upstream.access.approved, vec!["a".to_string()]);
    }

    #[test]
    fn an_upstream_never_prints_its_secret() {
        let printed = format!("{:?}", upstream("auth.acme.test"));

        assert!(!printed.contains("s3cret"));
    }
}
