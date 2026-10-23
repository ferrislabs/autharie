pub mod plan;
pub mod ports;
pub mod service;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalThemeError {
    Retry { message: String },
    Rejected { message: String },
}

impl PortalThemeError {
    pub fn message(&self) -> &str {
        match self {
            Self::Retry { message } | Self::Rejected { message } => message,
        }
    }

    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_retry_is_worth_retrying() {
        assert!(
            PortalThemeError::Retry {
                message: "x".to_string()
            }
            .is_retry()
        );
        assert!(
            !PortalThemeError::Rejected {
                message: "x".to_string()
            }
            .is_retry()
        );
    }

    #[test]
    fn the_message_is_the_one_given() {
        let error = PortalThemeError::Rejected {
            message: "FerrisKey answered 400".to_string(),
        };

        assert_eq!(error.message(), "FerrisKey answered 400");
    }
}
