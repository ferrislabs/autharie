use std::collections::HashSet;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AccessMode {
    #[default]
    Open,
    Approval,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct AccessConfig {
    #[serde(default)]
    pub mode: AccessMode,
    #[serde(default)]
    pub approved: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Blocked,
    Pending,
}

impl Refusal {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Blocked => "agent_blocked",
            Self::Pending => "agent_pending",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Access {
    mode: AccessMode,
    approved: HashSet<String>,
    blocked: HashSet<String>,
}

impl From<AccessConfig> for Access {
    fn from(config: AccessConfig) -> Self {
        Self {
            mode: config.mode,
            approved: config.approved.into_iter().collect(),
            blocked: config.blocked.into_iter().collect(),
        }
    }
}

impl Access {
    pub fn check(&self, client_id: &str) -> Result<(), Refusal> {
        if self.blocked.contains(client_id) {
            return Err(Refusal::Blocked);
        }
        match self.mode {
            AccessMode::Open => Ok(()),
            AccessMode::Approval if self.approved.contains(client_id) => Ok(()),
            AccessMode::Approval => Err(Refusal::Pending),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(mode: AccessMode, approved: &[&str], blocked: &[&str]) -> Access {
        AccessConfig {
            mode,
            approved: approved.iter().map(|id| (*id).to_owned()).collect(),
            blocked: blocked.iter().map(|id| (*id).to_owned()).collect(),
        }
        .into()
    }

    #[test]
    fn an_open_upstream_lets_an_unknown_agent_through() {
        assert_eq!(access(AccessMode::Open, &[], &[]).check("new"), Ok(()));
    }

    #[test]
    fn an_open_upstream_still_refuses_a_blocked_agent() {
        assert_eq!(
            access(AccessMode::Open, &[], &["bad"]).check("bad"),
            Err(Refusal::Blocked)
        );
    }

    #[test]
    fn approval_mode_refuses_an_agent_nobody_approved() {
        assert_eq!(
            access(AccessMode::Approval, &["known"], &[]).check("new"),
            Err(Refusal::Pending)
        );
        assert_eq!(
            access(AccessMode::Approval, &["known"], &[]).check("known"),
            Ok(())
        );
    }

    #[test]
    fn blocked_wins_over_approved() {
        assert_eq!(
            access(AccessMode::Approval, &["both"], &["both"]).check("both"),
            Err(Refusal::Blocked)
        );
    }

    #[test]
    fn refusals_name_their_reason() {
        assert_eq!(Refusal::Blocked.reason(), "agent_blocked");
        assert_eq!(Refusal::Pending.reason(), "agent_pending");
    }

    #[test]
    fn the_config_defaults_to_open_and_empty() {
        let config: AccessConfig = toml::from_str("").unwrap();
        assert_eq!(config, AccessConfig::default());
        assert_eq!(config.mode, AccessMode::Open);
    }

    #[test]
    fn the_config_reads_the_frozen_shape() {
        let config: AccessConfig =
            toml::from_str("mode = \"approval\"\napproved = [\"a\"]\nblocked = [\"b\"]").unwrap();
        assert_eq!(config.mode, AccessMode::Approval);
        assert_eq!(config.approved, ["a"]);
        assert_eq!(config.blocked, ["b"]);
    }
}
