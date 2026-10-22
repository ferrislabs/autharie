use std::fmt;

use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

const NAMESPACE_SLOT: &str = "{namespace}";
const NAME_SLOT: &str = "{name}";

#[derive(Debug, Clone)]
pub struct Upstream {
    template: String,
    root: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Instance {
    base: Url,
}

impl Upstream {
    pub fn new(template: &str, root_path: &str) -> Result<Self, ConfigError> {
        if !template.contains(NAMESPACE_SLOT) || !template.contains(NAME_SLOT) {
            return Err(ConfigError::new(
                "the API URL template must contain {namespace} and {name}",
            ));
        }
        if template.contains("/../")
            || template.contains("/./")
            || template.ends_with("/..")
            || template.ends_with("/.")
        {
            return Err(ConfigError::new(
                "the API URL template must not contain dot segments",
            ));
        }
        let upstream = Self {
            template: template.to_owned(),
            root: root_segments(root_path)?,
        };
        upstream.build("a", "a")?;
        Ok(upstream)
    }

    pub fn resolve(&self, namespace: &str, name: &str) -> Option<Instance> {
        if !is_dns_label(namespace) || !is_dns_label(name) {
            return None;
        }
        self.build(namespace, name).ok()
    }

    fn build(&self, namespace: &str, name: &str) -> Result<Instance, ConfigError> {
        let filled = self
            .template
            .replace(NAMESPACE_SLOT, namespace)
            .replace(NAME_SLOT, name);
        let mut base = Url::parse(&filled)
            .map_err(|error| ConfigError::new(format!("the API URL is not valid: {error}")))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(ConfigError::new("the API URL must be http or https"));
        }
        if base.host_str().is_none() {
            return Err(ConfigError::new("the API URL must name a host"));
        }
        if !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(ConfigError::new(
                "the API URL must not carry credentials, a query or a fragment",
            ));
        }
        base.path_segments_mut()
            .map_err(|()| ConfigError::new("the API URL cannot be a base"))?
            .pop_if_empty()
            .extend(&self.root);
        Ok(Instance { base })
    }
}

impl Instance {
    pub(crate) fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }
}

fn root_segments(root_path: &str) -> Result<Vec<String>, ConfigError> {
    let trimmed = root_path.strip_prefix('/').unwrap_or(root_path);
    let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    trimmed
        .split('/')
        .map(|segment| {
            let plain = !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
            if plain {
                Ok(segment.to_owned())
            } else {
                Err(ConfigError::new(
                    "the root path must be plain path segments",
                ))
            }
        })
        .collect()
}

fn is_dns_label(value: &str) -> bool {
    let bytes = value.as_bytes();
    let edge = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    (1..=63).contains(&bytes.len())
        && bytes.first().is_some_and(edge)
        && bytes.last().is_some_and(edge)
        && bytes.iter().all(|b| edge(b) || *b == b'-')
}
