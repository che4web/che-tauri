use serde::Serialize;

use crate::{filters::FilterError, serializer::SerializerError};

#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub detail: String,
}

impl ApiError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new("bad_request", detail)
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new("not_found", detail)
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new("internal_error", detail)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.detail)
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

impl From<che_orm::Error> for ApiError {
    fn from(error: che_orm::Error) -> Self {
        match &error {
            che_orm::Error::UnknownField(_)
            | che_orm::Error::ReadonlyField(_)
            | che_orm::Error::EmptyUpdate
            | che_orm::Error::MissingPrimaryKey => Self::bad_request(error.to_string()),
            che_orm::Error::Database(che_orm::__private::sqlx::Error::RowNotFound) => {
                Self::not_found(error.to_string())
            }
            che_orm::Error::Database(_) => Self::new("database_error", error.to_string()),
            che_orm::Error::Io(_) | che_orm::Error::Json(_) => Self::internal(error.to_string()),
        }
    }
}

impl From<reqwest::Error> for ApiError {
    fn from(error: reqwest::Error) -> Self {
        Self::new("remote_error", error.to_string())
    }
}

impl From<SerializerError> for ApiError {
    fn from(error: SerializerError) -> Self {
        Self::bad_request(error.to_string())
    }
}

impl From<FilterError> for ApiError {
    fn from(error: FilterError) -> Self {
        Self::bad_request(error.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        Self::internal(error.to_string())
    }
}

impl From<toml::de::Error> for ApiError {
    fn from(error: toml::de::Error) -> Self {
        Self::bad_request(error.to_string())
    }
}
