use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{WebhookError, events::EVENTS};

pub const MAX_ENDPOINT_LENGTH: usize = 2048;
pub const MAX_NAME_LENGTH: usize = 100;
pub const MAX_DESCRIPTION_LENGTH: usize = 500;
pub const MAX_EVENTS: usize = 50;
pub const MAX_HEADERS: usize = 10;
pub const MAX_HEADER_NAME_LENGTH: usize = 100;
pub const MAX_HEADER_VALUE_LENGTH: usize = 1024;
pub const MAX_REALM_LENGTH: usize = 100;
pub const MAX_DELIVERY_ID_LENGTH: usize = 200;
pub const DEFAULT_REALM: &str = "master";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, format = Uuid)]
pub struct WebhookId(pub Uuid);

impl fmt::Display for WebhookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "https://example.com/hooks/autharie")]
pub struct WebhookEndpoint(String);

impl WebhookEndpoint {
    pub fn parse(value: &str) -> Result<Self, WebhookError> {
        if value.len() > MAX_ENDPOINT_LENGTH {
            return Err(WebhookError::invalid(format!(
                "the endpoint may hold at most {MAX_ENDPOINT_LENGTH} characters"
            )));
        }

        if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(WebhookError::invalid(
                "the endpoint may not hold whitespace or control characters",
            ));
        }

        let Some((scheme, rest)) = value.split_once("://") else {
            return Err(WebhookError::invalid(
                "the endpoint must be an absolute https:// URL",
            ));
        };

        if !scheme.eq_ignore_ascii_case("https") {
            return Err(WebhookError::invalid(
                "only https:// endpoints are accepted",
            ));
        }

        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();

        if authority.contains('@') {
            return Err(WebhookError::invalid(
                "the endpoint may not carry a user name or password",
            ));
        }

        let (host, port) = split_host_port(authority)?;

        if host.is_empty() {
            return Err(WebhookError::invalid("the endpoint has no host"));
        }

        if !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']' | ':'))
        {
            return Err(WebhookError::invalid(
                "the endpoint host holds characters a host cannot",
            ));
        }

        if let Some(port) = port
            && (port.is_empty() || port.parse::<u16>().is_err())
        {
            return Err(WebhookError::invalid("the endpoint port is not valid"));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn split_host_port(authority: &str) -> Result<(&str, Option<&str>), WebhookError> {
    if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((address, after)) = bracketed.split_once(']') else {
            return Err(WebhookError::invalid("the endpoint host is not valid"));
        };

        return match after.strip_prefix(':') {
            Some(port) => Ok((address, Some(port))),
            None if after.is_empty() => Ok((address, None)),
            None => Err(WebhookError::invalid("the endpoint host is not valid")),
        };
    }

    Ok(match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct WebhookName(String);

impl WebhookName {
    pub fn parse(value: &str) -> Result<Self, WebhookError> {
        let value = value.trim();
        let length = value.chars().count();

        if length == 0 || length > MAX_NAME_LENGTH {
            return Err(WebhookError::invalid(format!(
                "the name must hold between 1 and {MAX_NAME_LENGTH} characters"
            )));
        }

        if value.chars().any(char::is_control) {
            return Err(WebhookError::invalid(
                "the name may not hold control characters",
            ));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn parse_description(value: Option<&str>) -> Result<Option<String>, WebhookError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if value.chars().count() > MAX_DESCRIPTION_LENGTH {
        return Err(WebhookError::invalid(format!(
            "the description may hold at most {MAX_DESCRIPTION_LENGTH} characters"
        )));
    }

    Ok(Some(value.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "master")]
pub struct WebhookRealm(String);

impl WebhookRealm {
    pub fn parse(value: &str) -> Result<Self, WebhookError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_REALM_LENGTH
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));

        if !valid {
            return Err(WebhookError::invalid(format!(
                "a realm is 1 to {MAX_REALM_LENGTH} letters, digits, hyphens or underscores"
            )));
        }

        Ok(Self(value.to_string()))
    }

    pub fn master() -> Self {
        Self(DEFAULT_REALM.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "user.created")]
pub struct WebhookEvent(&'static str);

impl WebhookEvent {
    pub fn parse(value: &str) -> Result<Self, WebhookError> {
        EVENTS
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(name, _)| Self(name))
            .ok_or_else(|| WebhookError::invalid(format!("'{value}' is not a known event")))
    }

    pub fn name(&self) -> &'static str {
        self.0
    }

    pub fn group(&self) -> &'static str {
        EVENTS
            .iter()
            .find(|(name, _)| *name == self.0)
            .map(|(_, group)| *group)
            .unwrap_or_default()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        EVENTS.iter().map(|(name, _)| Self(name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct WebhookEventInfo {
    pub name: &'static str,
    pub group: &'static str,
}

pub fn known_events() -> Vec<WebhookEventInfo> {
    WebhookEvent::all()
        .map(|event| WebhookEventInfo {
            name: event.name(),
            group: event.group(),
        })
        .collect()
}

pub fn parse_events(values: &[String]) -> Result<Vec<WebhookEvent>, WebhookError> {
    if values.is_empty() || values.len() > MAX_EVENTS {
        return Err(WebhookError::invalid(format!(
            "a webhook listens to between 1 and {MAX_EVENTS} events"
        )));
    }

    let mut seen = HashSet::new();
    let mut events = Vec::with_capacity(values.len());

    for value in values {
        let event = WebhookEvent::parse(value)?;
        if !seen.insert(event) {
            return Err(WebhookError::invalid(format!(
                "the event '{value}' is listed twice"
            )));
        }
        events.push(event);
    }

    Ok(events)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "x-api-key")]
pub struct HeaderName(String);

impl HeaderName {
    pub fn parse(value: &str) -> Result<Self, WebhookError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_HEADER_NAME_LENGTH
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));

        if !valid {
            return Err(WebhookError::invalid(format!(
                "a header name is 1 to {MAX_HEADER_NAME_LENGTH} letters, digits, hyphens, dots or underscores"
            )));
        }

        let lowered = value.to_ascii_lowercase();
        if matches!(lowered.as_str(), "content-type" | "content-length" | "host")
            || lowered.starts_with("x-ferriskey-")
        {
            return Err(WebhookError::invalid(format!(
                "the header '{value}' is reserved"
            )));
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn same_as(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HeaderValue(String);

impl HeaderValue {
    pub fn parse(value: &str) -> Result<Self, WebhookError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_HEADER_VALUE_LENGTH
            && value
                .chars()
                .all(|c| c == '\t' || (c.is_ascii() && !c.is_ascii_control()));

        if !valid {
            return Err(WebhookError::invalid(format!(
                "a header value is 1 to {MAX_HEADER_VALUE_LENGTH} printable ASCII characters"
            )));
        }

        Ok(Self(value.to_string()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HeaderValue(..)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderInput {
    pub name: HeaderName,
    pub value: Option<HeaderValue>,
}

pub fn parse_headers(raw: &[(String, Option<String>)]) -> Result<Vec<HeaderInput>, WebhookError> {
    if raw.len() > MAX_HEADERS {
        return Err(WebhookError::invalid(format!(
            "a webhook carries at most {MAX_HEADERS} custom headers"
        )));
    }

    let mut headers: Vec<HeaderInput> = Vec::with_capacity(raw.len());

    for (name, value) in raw {
        let name = HeaderName::parse(name)?;
        if headers.iter().any(|header| header.name.same_as(&name)) {
            return Err(WebhookError::invalid(format!(
                "the header '{}' is listed twice",
                name.as_str()
            )));
        }
        let value = value.as_deref().map(HeaderValue::parse).transpose()?;
        headers.push(HeaderInput { name, value });
    }

    Ok(headers)
}

pub fn parse_delivery_id(value: &str) -> Result<String, WebhookError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_DELIVERY_ID_LENGTH
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));

    if !valid {
        return Err(WebhookError::invalid("the delivery id is not valid"));
    }

    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_https_endpoint_is_accepted() {
        for accepted in [
            "https://example.com",
            "https://example.com/hooks/a?b=c#d",
            "https://hooks.example.com:8443/x",
            "HTTPS://example.com/",
            "https://[2001:db8::1]:443/x",
        ] {
            assert!(WebhookEndpoint::parse(accepted).is_ok(), "{accepted}");
        }
    }

    #[test]
    fn anything_but_an_absolute_https_url_is_refused() {
        for refused in [
            "http://example.com",
            "ftp://example.com",
            "example.com/hook",
            "/relative",
            "https://",
            "https:///path",
            "https://user@example.com",
            "https://user:pass@example.com/x",
            "https://example.com:99999/x",
            "https://example.com:/x",
            "https://exa mple.com",
            "https://exämple.com",
            "",
            "javascript:alert(1)",
        ] {
            assert!(WebhookEndpoint::parse(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn an_endpoint_is_at_most_2048_characters() {
        let long = format!("https://example.com/{}", "a".repeat(2048));
        assert!(WebhookEndpoint::parse(&long).is_err());

        let exact = format!("https://example.com/{}", "a".repeat(2048 - 20));
        assert_eq!(exact.len(), 2048);
        assert!(WebhookEndpoint::parse(&exact).is_ok());
    }

    #[test]
    fn a_userinfo_hidden_after_the_path_is_not_userinfo() {
        assert!(WebhookEndpoint::parse("https://example.com/a@b").is_ok());
    }

    #[test]
    fn a_name_holds_one_to_a_hundred_characters() {
        assert!(WebhookName::parse("orders").is_ok());
        assert!(WebhookName::parse("   ").is_err());
        assert!(WebhookName::parse("").is_err());
        assert!(WebhookName::parse(&"a".repeat(100)).is_ok());
        assert!(WebhookName::parse(&"a".repeat(101)).is_err());
    }

    #[test]
    fn a_description_is_optional_and_at_most_five_hundred() {
        assert_eq!(parse_description(None).unwrap(), None);
        assert_eq!(parse_description(Some("  ")).unwrap(), None);
        assert!(parse_description(Some(&"a".repeat(500))).unwrap().is_some());
        assert!(parse_description(Some(&"a".repeat(501))).is_err());
    }

    #[test]
    fn every_event_has_a_group() {
        let groups: HashSet<_> = WebhookEvent::all().map(|event| event.group()).collect();

        for expected in [
            "user",
            "auth",
            "client",
            "redirect_uri",
            "web_origin",
            "role",
            "realm",
            "webhook",
        ] {
            assert!(groups.contains(expected), "{expected}");
        }
        assert_eq!(groups.len(), 8);
        assert_eq!(
            WebhookEvent::parse("user.role.assigned").unwrap().group(),
            "user"
        );
    }

    #[test]
    fn an_unknown_event_is_refused() {
        assert!(WebhookEvent::parse("user.exploded").is_err());
        assert!(WebhookEvent::parse("").is_err());
        assert!(WebhookEvent::parse("USER.CREATED").is_err());
    }

    #[test]
    fn events_are_one_to_fifty_and_unique() {
        let one = vec!["user.created".to_string()];
        assert!(parse_events(&one).is_ok());
        assert!(parse_events(&[]).is_err());

        let twice = vec!["user.created".to_string(), "user.created".to_string()];
        assert!(parse_events(&twice).is_err());

        let every: Vec<String> = WebhookEvent::all().map(|e| e.name().to_string()).collect();
        assert!(parse_events(&every).is_ok());
    }

    #[test]
    fn the_event_list_has_no_duplicates() {
        let names: HashSet<_> = WebhookEvent::all().map(|event| event.name()).collect();
        assert_eq!(names.len(), EVENTS.len());
    }

    #[test]
    fn a_header_name_is_a_plain_token_and_not_reserved() {
        assert!(HeaderName::parse("x-api-key").is_ok());
        assert!(HeaderName::parse("Authorization").is_ok());

        for refused in [
            "",
            "bad name",
            "bad:name",
            "Content-Type",
            "content-length",
            "HOST",
            "x-ferriskey-signature",
            "X-FerrisKey-Anything",
            "x\n",
        ] {
            assert!(HeaderName::parse(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn a_header_value_has_no_line_break() {
        assert!(HeaderValue::parse("Bearer abc.def").is_ok());
        assert!(HeaderValue::parse("a\r\nx-evil: 1").is_err());
        assert!(HeaderValue::parse("").is_err());
    }

    #[test]
    fn a_header_value_does_not_print() {
        let value = HeaderValue::parse("hunter2").unwrap();
        assert!(!format!("{value:?}").contains("hunter2"));
    }

    #[test]
    fn headers_are_at_most_ten_and_unique_ignoring_case() {
        let raw = |names: &[&str]| -> Vec<(String, Option<String>)> {
            names
                .iter()
                .map(|name| (name.to_string(), Some("v".to_string())))
                .collect()
        };

        assert!(parse_headers(&raw(&["a", "b"])).is_ok());
        assert!(parse_headers(&raw(&["a", "A"])).is_err());

        let eleven: Vec<String> = (0..11).map(|i| format!("h{i}")).collect();
        let eleven: Vec<&str> = eleven.iter().map(String::as_str).collect();
        assert!(parse_headers(&raw(&eleven)).is_err());
        assert!(parse_headers(&raw(&eleven[..10])).is_ok());
    }

    #[test]
    fn a_realm_is_a_simple_name() {
        assert!(WebhookRealm::parse("master").is_ok());
        assert!(WebhookRealm::parse("my-realm_2").is_ok());
        assert!(WebhookRealm::parse("").is_err());
        assert!(WebhookRealm::parse("a/b").is_err());
        assert!(WebhookRealm::parse("a b").is_err());
    }

    #[test]
    fn a_delivery_id_is_a_token() {
        assert!(parse_delivery_id("0191a0e0-aaaa-7000-8000-000000000001").is_ok());
        assert!(parse_delivery_id("").is_err());
        assert!(parse_delivery_id("a b").is_err());
        assert!(parse_delivery_id("../x").is_err());
    }
}
