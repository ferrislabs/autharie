pub mod envelope;
pub mod ports;
pub mod result;
pub mod service;

use std::fmt;

use serde::Deserialize;
use uuid::Uuid;

pub const MAX_PAGE_SIZE: u32 = 50;
pub const DEFAULT_PAGE_SIZE: u32 = 25;
pub const MAX_RESULT_BYTES: usize = 512 * 1024;
const MAX_SEARCH_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IamError {
    Retry { message: String },
    Rejected { message: String },
}

impl IamError {
    pub fn message(&self) -> &str {
        match self {
            Self::Retry { message } | Self::Rejected { message } => message,
        }
    }

    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }

    pub fn rejected(message: impl Into<String>) -> Self {
        Self::Rejected {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSize(u32);

impl PageSize {
    pub fn clamped(requested: u32) -> Self {
        Self(requested.clamp(1, MAX_PAGE_SIZE))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageNumber(u32);

impl PageNumber {
    pub fn clamped(requested: u32) -> Self {
        Self(requested.max(1))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SearchTerm(String);

impl SearchTerm {
    fn parse(raw: &str) -> Result<Option<Self>, IamError> {
        let term = raw.trim();
        if term.is_empty() {
            return Ok(None);
        }
        if term.chars().count() > MAX_SEARCH_CHARS {
            return Err(IamError::rejected("the search term is too long"));
        }
        Ok(Some(Self(term.to_string())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SearchTerm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SearchTerm(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IamOperation {
    ApplicationsList {
        page: PageNumber,
        limit: PageSize,
        search: Option<SearchTerm>,
    },
    ApplicationsGet {
        id: Uuid,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawOperation {
    #[serde(rename = "applications.list")]
    List {
        #[serde(default = "first_page")]
        page: u32,
        #[serde(default = "default_page_size")]
        limit: u32,
        #[serde(default)]
        search: Option<String>,
    },
    #[serde(rename = "applications.get")]
    Get { id: Uuid },
}

fn first_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    DEFAULT_PAGE_SIZE
}

impl TryFrom<&str> for IamOperation {
    type Error = IamError;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        let parsed: RawOperation = serde_json::from_str(raw).map_err(|_| {
            IamError::rejected("the request names an operation that does not exist")
        })?;

        Ok(match parsed {
            RawOperation::List {
                page,
                limit,
                search,
            } => Self::ApplicationsList {
                page: PageNumber::clamped(page),
                limit: PageSize::clamped(limit),
                search: search
                    .as_deref()
                    .map(SearchTerm::parse)
                    .transpose()?
                    .flatten(),
            },
            RawOperation::Get { id } => Self::ApplicationsGet { id },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestId(Uuid);

impl RequestId {
    pub fn parse(raw: &str) -> Result<Self, IamError> {
        Uuid::parse_str(raw)
            .map(Self)
            .map_err(|_| IamError::rejected("the request id is not a UUID"))
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0.hyphenated())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b9ad3c4-4a5f-4c3b-8d57-0b2d6d9e1a11";

    fn list(page: u32, limit: u32) -> IamOperation {
        IamOperation::try_from(
            format!(r#"{{"type":"applications.list","page":{page},"limit":{limit}}}"#).as_str(),
        )
        .expect("parsed")
    }

    #[test]
    fn a_list_without_page_or_limit_takes_the_defaults() {
        let operation = IamOperation::try_from(r#"{"type":"applications.list"}"#).unwrap();

        assert_eq!(
            operation,
            IamOperation::ApplicationsList {
                page: PageNumber::clamped(1),
                limit: PageSize::clamped(DEFAULT_PAGE_SIZE),
                search: None,
            }
        );
    }

    #[test]
    fn a_limit_above_fifty_is_clamped_to_fifty() {
        for requested in [51, 100, 10_000, u32::MAX] {
            let IamOperation::ApplicationsList { limit, .. } = list(1, requested) else {
                panic!("a list");
            };
            assert_eq!(limit.get(), MAX_PAGE_SIZE, "{requested}");
        }
    }

    #[test]
    fn a_limit_of_zero_and_a_page_of_zero_are_raised_to_one() {
        let IamOperation::ApplicationsList { page, limit, .. } = list(0, 0) else {
            panic!("a list");
        };

        assert_eq!(page.get(), 1);
        assert_eq!(limit.get(), 1);
    }

    #[test]
    fn a_limit_within_bounds_is_kept() {
        let IamOperation::ApplicationsList { limit, .. } = list(3, 50) else {
            panic!("a list");
        };

        assert_eq!(limit.get(), 50);
    }

    #[test]
    fn a_search_term_is_trimmed_and_a_blank_one_is_no_search() {
        let kept = IamOperation::try_from(r#"{"type":"applications.list","search":"  web  "}"#);
        let blank = IamOperation::try_from(r#"{"type":"applications.list","search":"   "}"#);

        let Ok(IamOperation::ApplicationsList { search, .. }) = kept else {
            panic!("a list");
        };
        assert_eq!(search.as_ref().map(SearchTerm::as_str), Some("web"));
        let Ok(IamOperation::ApplicationsList { search, .. }) = blank else {
            panic!("a list");
        };
        assert_eq!(search, None);
    }

    #[test]
    fn a_search_term_never_reaches_a_log_line() {
        let operation =
            IamOperation::try_from(r#"{"type":"applications.list","search":"jane.doe"}"#).unwrap();

        assert!(!format!("{operation:?}").contains("jane.doe"));
    }

    #[test]
    fn a_search_term_over_the_cap_is_refused() {
        let raw = format!(
            r#"{{"type":"applications.list","search":"{}"}}"#,
            "a".repeat(MAX_SEARCH_CHARS + 1)
        );

        assert!(matches!(
            IamOperation::try_from(raw.as_str()),
            Err(IamError::Rejected { .. })
        ));
    }

    #[test]
    fn a_get_carries_the_application_uuid() {
        let operation = IamOperation::try_from(
            format!(r#"{{"type":"applications.get","id":"{ID}"}}"#).as_str(),
        )
        .unwrap();

        assert_eq!(
            operation,
            IamOperation::ApplicationsGet {
                id: Uuid::parse_str(ID).unwrap()
            }
        );
    }

    #[test]
    fn only_the_named_operations_exist() {
        for raw in [
            r#"{"type":"applications.delete","id":"x"}"#,
            r#"{"type":"users.list"}"#,
            r#"{"type":"applications.get","id":"../../admin"}"#,
            r#"{"type":"applications.get"}"#,
            r#"{"path":"/realms/master/clients"}"#,
            "not json",
            "",
        ] {
            assert!(
                matches!(IamOperation::try_from(raw), Err(IamError::Rejected { .. })),
                "{raw}"
            );
        }
    }

    #[test]
    fn a_request_id_is_spelled_lowercase_and_hyphenated() {
        let id = RequestId::parse("0B9AD3C44A5F4C3B8D570B2D6D9E1A11").unwrap();

        assert_eq!(id.to_string(), ID);
        assert!(RequestId::parse("not-a-uuid").is_err());
    }

    #[test]
    fn only_a_retry_is_worth_retrying() {
        assert!(
            IamError::Retry {
                message: "x".to_string()
            }
            .is_retry()
        );
        assert!(!IamError::rejected("x").is_retry());
    }
}
