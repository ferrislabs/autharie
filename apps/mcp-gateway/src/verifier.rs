use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio::sync::RwLock;
use tracing::warn;

#[derive(Debug, Clone, PartialEq)]
pub struct Principal {
    pub sub: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub exp: u64,
    pub roles: Vec<String>,
    pub groups: Vec<String>,
    pub organisation: Option<String>,
    pub act: Option<Value>,
    pub delegated: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("invalid token")]
    Invalid,
    #[error("key set unavailable")]
    Unavailable,
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    azp: Option<String>,
    client_id: Option<String>,
    scope: Option<String>,
    exp: u64,
    sid: Option<String>,
    act: Option<Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl Claims {
    fn strings(&self, key: &str) -> Vec<String> {
        self.extra.get(key).map(strings_of).unwrap_or_default()
    }

    fn roles(&self) -> Vec<String> {
        let direct = self.strings("roles");
        if !direct.is_empty() {
            return direct;
        }
        self.extra
            .get("realm_access")
            .and_then(|realm| realm.get("roles"))
            .map(strings_of)
            .unwrap_or_default()
    }

    fn organisation(&self) -> Option<String> {
        ["organisation", "organization"]
            .iter()
            .find_map(|key| self.extra.get(*key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
}

fn strings_of(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: Option<String>,
    kty: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

struct KeySet {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

pub struct JwksVerifier {
    http: Client,
    issuer: String,
    audience: String,
    jwks_url: String,
    refresh_interval: Duration,
    unknown_kid_cooldown: Duration,
    keys: RwLock<KeySet>,
}

impl JwksVerifier {
    pub fn new(
        http: Client,
        issuer: &str,
        audience: impl Into<String>,
        jwks_url: impl Into<String>,
        refresh_interval: Duration,
    ) -> Self {
        Self {
            http,
            issuer: issuer.trim_end_matches('/').to_owned(),
            audience: audience.into(),
            jwks_url: jwks_url.into(),
            refresh_interval,
            unknown_kid_cooldown: Duration::from_secs(5),
            keys: RwLock::new(KeySet {
                keys: HashMap::new(),
                fetched_at: None,
            }),
        }
    }

    pub fn with_unknown_kid_cooldown(mut self, cooldown: Duration) -> Self {
        self.unknown_kid_cooldown = cooldown;
        self
    }

    async fn refresh(&self) -> Result<(), VerifyError> {
        let response = self
            .http
            .get(&self.jwks_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|error| {
                warn!(%error, "jwks fetch failed");
                VerifyError::Unavailable
            })?;
        let jwks: Jwks = response.json().await.map_err(|error| {
            warn!(%error, "jwks body unreadable");
            VerifyError::Unavailable
        })?;
        let keys = jwks
            .keys
            .into_iter()
            .filter(|k| k.kty.as_deref().is_none_or(|t| t == "RSA"))
            .filter_map(|k| {
                let key = DecodingKey::from_rsa_components(k.n.as_deref()?, k.e.as_deref()?);
                Some((k.kid?, key.ok()?))
            })
            .collect();
        *self.keys.write().await = KeySet {
            keys,
            fetched_at: Some(Instant::now()),
        };
        Ok(())
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey, VerifyError> {
        let stale = {
            let set = self.keys.read().await;
            if let Some(key) = set.keys.get(kid)
                && set
                    .fetched_at
                    .is_some_and(|at| at.elapsed() < self.refresh_interval)
            {
                return Ok(key.clone());
            }
            set.fetched_at
        };
        let may_refetch = stale.is_none_or(|at| at.elapsed() >= self.unknown_kid_cooldown);
        if may_refetch {
            self.refresh().await?;
        }
        self.keys
            .read()
            .await
            .keys
            .get(kid)
            .cloned()
            .ok_or(VerifyError::Invalid)
    }

    pub async fn verify(&self, token: &str) -> Result<Principal, VerifyError> {
        let header = decode_header(token).map_err(|_| VerifyError::Invalid)?;
        if header.alg != Algorithm::RS256 {
            return Err(VerifyError::Invalid);
        }
        let kid = header.kid.ok_or(VerifyError::Invalid)?;
        let key = self.key_for(&kid).await?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = 5;

        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| VerifyError::Invalid)?
            .claims;
        let roles = claims.roles();
        let groups = claims.strings("groups");
        let organisation = claims.organisation();
        let client_id = claims
            .azp
            .or(claims.client_id)
            .ok_or(VerifyError::Invalid)?;
        Ok(Principal {
            delegated: claims.sid.is_some(),
            sub: claims.sub,
            client_id,
            scopes: claims
                .scope
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
            exp: claims.exp,
            roles,
            groups,
            organisation,
            act: claims.act,
        })
    }
}
