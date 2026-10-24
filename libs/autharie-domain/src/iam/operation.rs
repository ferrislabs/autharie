use std::num::NonZeroU32;

use uuid::Uuid;

use super::IamError;

pub const MAX_PAGE_SIZE: u32 = 50;
pub const DEFAULT_PAGE_SIZE: u32 = 25;
pub const MAX_SEARCH_CHARS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IamAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IamOperationKind {
    ApplicationsList,
    ApplicationsGet,
}

impl IamOperationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ApplicationsList => "applications.list",
            Self::ApplicationsGet => "applications.get",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "applications.list" => Some(Self::ApplicationsList),
            "applications.get" => Some(Self::ApplicationsGet),
            _ => None,
        }
    }

    pub fn access(&self) -> IamAccess {
        match self {
            Self::ApplicationsList | Self::ApplicationsGet => IamAccess::Read,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page(NonZeroU32);

impl Page {
    pub fn parse(value: u32) -> Result<Self, IamError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or_else(|| IamError::invalid("pages start at 1"))
    }

    pub fn first() -> Self {
        Self(NonZeroU32::MIN)
    }

    pub fn value(&self) -> u32 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSize(NonZeroU32);

impl PageSize {
    pub fn parse(value: u32) -> Result<Self, IamError> {
        NonZeroU32::new(value)
            .filter(|size| size.get() <= MAX_PAGE_SIZE)
            .map(Self)
            .ok_or_else(|| IamError::invalid(format!("a page holds 1 to {MAX_PAGE_SIZE} objects")))
    }

    pub fn value(&self) -> u32 {
        self.0.get()
    }
}

impl Default for PageSize {
    fn default() -> Self {
        const SIZE: NonZeroU32 = match NonZeroU32::new(DEFAULT_PAGE_SIZE) {
            Some(size) => size,
            None => NonZeroU32::MIN,
        };

        Self(SIZE)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchTerm(String);

impl SearchTerm {
    pub fn parse(value: &str) -> Result<Option<Self>, IamError> {
        let value = value.trim();

        if value.is_empty() {
            return Ok(None);
        }
        if value.chars().count() > MAX_SEARCH_CHARS {
            return Err(IamError::invalid(format!(
                "a search is at most {MAX_SEARCH_CHARS} characters"
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(IamError::invalid(
                "a search may not hold control characters",
            ));
        }

        Ok(Some(Self(value.to_string())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IamOperation {
    ApplicationsList {
        page: Page,
        limit: PageSize,
        search: Option<SearchTerm>,
    },
    ApplicationsGet {
        id: Uuid,
    },
}

impl IamOperation {
    pub fn applications_list(
        page: Option<u32>,
        limit: Option<u32>,
        search: Option<&str>,
    ) -> Result<Self, IamError> {
        Ok(Self::ApplicationsList {
            page: page.map_or(Ok(Page::first()), Page::parse)?,
            limit: limit.map_or(Ok(PageSize::default()), PageSize::parse)?,
            search: search.map(SearchTerm::parse).transpose()?.flatten(),
        })
    }

    pub fn kind(&self) -> IamOperationKind {
        match self {
            Self::ApplicationsList { .. } => IamOperationKind::ApplicationsList,
            Self::ApplicationsGet { .. } => IamOperationKind::ApplicationsGet,
        }
    }

    pub fn access(&self) -> IamAccess {
        self.kind().access()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_without_parameters_takes_the_first_page_of_25() {
        let operation = IamOperation::applications_list(None, None, None).unwrap();

        assert_eq!(
            operation,
            IamOperation::ApplicationsList {
                page: Page::first(),
                limit: PageSize::parse(25).unwrap(),
                search: None,
            }
        );
    }

    #[test]
    fn a_page_holds_between_one_and_fifty_objects() {
        assert!(PageSize::parse(1).is_ok());
        assert!(PageSize::parse(50).is_ok());
        assert!(PageSize::parse(0).is_err());
        assert!(PageSize::parse(51).is_err());
        assert!(IamOperation::applications_list(None, Some(51), None).is_err());
    }

    #[test]
    fn pages_start_at_one() {
        assert!(Page::parse(0).is_err());
        assert_eq!(Page::parse(3).unwrap().value(), 3);
        assert!(IamOperation::applications_list(Some(0), None, None).is_err());
    }

    #[test]
    fn a_blank_search_is_no_search_and_a_term_is_trimmed() {
        assert_eq!(SearchTerm::parse("   ").unwrap(), None);
        assert_eq!(
            SearchTerm::parse("  alice ").unwrap().unwrap().as_str(),
            "alice"
        );
        assert!(matches!(
            IamOperation::applications_list(None, None, Some("")).unwrap(),
            IamOperation::ApplicationsList { search: None, .. }
        ));
    }

    #[test]
    fn a_search_is_bounded_and_holds_no_control_characters() {
        assert!(SearchTerm::parse(&"a".repeat(MAX_SEARCH_CHARS)).is_ok());
        assert!(SearchTerm::parse(&"a".repeat(MAX_SEARCH_CHARS + 1)).is_err());
        assert!(SearchTerm::parse("a\nb").is_err());
    }

    #[test]
    fn every_named_operation_reads_for_now() {
        let id = Uuid::nil();

        assert_eq!(
            IamOperation::applications_list(None, None, None)
                .unwrap()
                .access(),
            IamAccess::Read
        );
        assert_eq!(
            IamOperation::ApplicationsGet { id }.access(),
            IamAccess::Read
        );
    }

    #[test]
    fn a_kind_round_trips_through_its_name() {
        for kind in [
            IamOperationKind::ApplicationsList,
            IamOperationKind::ApplicationsGet,
        ] {
            assert_eq!(IamOperationKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(IamOperationKind::parse("applications.delete"), None);
        assert_eq!(IamOperationKind::parse("/realms/master/clients"), None);
    }
}
