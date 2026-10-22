mod api;
mod app;
mod tools;
mod upstream;

pub use api::{ApiClient, Fault};
pub use app::router;
pub use tools::{FerrisKeyTools, TOOL_NAMES};
pub use upstream::{ConfigError, Instance, Upstream};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

pub(crate) fn valid_realm(realm: &str) -> Option<&str> {
    let plain = (1..=128).contains(&realm.len())
        && realm.chars().any(|c| c != '.')
        && realm
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    plain.then_some(realm)
}

pub(crate) fn token_realm(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let claims: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    let issuer = claims.get("iss")?.as_str()?;
    let mut segments = issuer.trim_end_matches('/').rsplit('/');
    let realm = segments.next()?;
    (segments.next()? == "realms")
        .then(|| valid_realm(realm))
        .flatten()
        .map(str::to_owned)
}
