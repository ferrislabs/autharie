use std::{fmt::Display, str::FromStr};

use autharie_domain::CoreError;

pub(super) fn failed(action: &'static str) -> impl Fn(sqlx::Error) -> CoreError {
    move |error| CoreError::DatabaseError {
        message: format!("Failed to {action}: {error}"),
    }
}

pub(super) fn invalid(column: &str, value: &str, reason: impl Display) -> CoreError {
    CoreError::DatabaseError {
        message: format!("Column {column} holds '{value}', which is not valid: {reason}"),
    }
}

pub(super) fn parsed<T>(column: &str, value: &str) -> Result<T, CoreError>
where
    T: FromStr,
    T::Err: Display,
{
    value
        .parse()
        .map_err(|reason| invalid(column, value, reason))
}
