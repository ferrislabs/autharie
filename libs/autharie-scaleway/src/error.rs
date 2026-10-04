#[derive(Debug, thiserror::Error)]
pub enum ScalewayError {
    #[error("the HTTP client could not be built: {0}")]
    Client(String),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ApiError {
    #[error("scaleway answered {status} ({kind}): {message}")]
    Status {
        status: u16,
        kind: String,
        message: String,
    },

    #[error("scaleway could not be reached: {0}")]
    Transport(String),

    #[error("scaleway answered with a body that could not be read: {0}")]
    Decode(String),
}

impl ApiError {
    pub(crate) fn is_not_found(&self) -> bool {
        matches!(self, Self::Status { status: 404, .. })
    }

    pub(crate) fn is_transient(&self) -> bool {
        match self {
            Self::Status { status, .. } => *status >= 500,
            Self::Transport(_) => true,
            Self::Decode(_) => false,
        }
    }

    pub(crate) fn is_denied(&self) -> bool {
        matches!(
            self,
            Self::Status {
                status: 401 | 403,
                ..
            }
        ) && !self.has_kind("quotas_exceeded")
    }

    pub(crate) fn has_kind(&self, expected: &str) -> bool {
        matches!(self, Self::Status { kind, .. } if kind == expected)
    }

    pub(crate) fn mentions_location(&self) -> bool {
        match self {
            Self::Status {
                status: 400 | 404,
                message,
                ..
            } => {
                let message = message.to_ascii_lowercase();
                message.contains("region") || message.contains("zone")
            }
            _ => false,
        }
    }
}
