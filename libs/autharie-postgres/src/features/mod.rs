#[cfg_attr(coverage_nightly, coverage(off))]
pub mod feature_repository;

pub use feature_repository::PostgresFeatureRepository;
