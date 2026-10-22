use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use tracing::{info, warn};

use crate::{app::AppState, config::GatewayConfig, decision::DecisionSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    Unchanged,
    Reloaded(usize),
    Rejected,
}

fn digest(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

pub fn initial(path: &Path) -> Result<(GatewayConfig, Option<u64>), Box<dyn std::error::Error>> {
    match fs::read_to_string(path) {
        Ok(source) => Ok((GatewayConfig::from_toml(&source)?, Some(digest(&source)))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok((GatewayConfig::from_toml("")?, None))
        }
        Err(error) => Err(error.into()),
    }
}

pub fn poll<S>(state: &AppState<S>, path: &Path, last: &mut Option<u64>) -> Poll
where
    S: DecisionSink,
{
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                warn!(%error, "gateway config unreadable, keeping the previous one");
            }
            return Poll::Unchanged;
        }
    };
    let current = digest(&source);
    if *last == Some(current) {
        return Poll::Unchanged;
    }
    *last = Some(current);
    let config = match GatewayConfig::from_toml(&source) {
        Ok(config) => config,
        Err(error) => {
            warn!(%error, "gateway config invalid, keeping the previous one");
            return Poll::Rejected;
        }
    };
    let upstreams = config.upstreams.len();
    match state.reload(config) {
        Ok(()) => {
            info!(upstreams, "gateway config reloaded");
            Poll::Reloaded(upstreams)
        }
        Err(error) => {
            warn!(%error, "gateway config not applied, keeping the previous one");
            Poll::Rejected
        }
    }
}

pub async fn watch<S>(state: AppState<S>, path: PathBuf, mut last: Option<u64>, interval: Duration)
where
    S: DecisionSink + Send + Sync + 'static,
{
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        poll(&state, &path, &mut last);
    }
}
