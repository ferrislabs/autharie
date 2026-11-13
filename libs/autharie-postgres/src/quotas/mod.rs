#[cfg_attr(coverage_nightly, coverage(off))]
pub mod quota_repository;

pub use quota_repository::PostgresQuotaRepository;
