//! RFC 9457 problem details (PLAN.md §17.1).

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use kabados_db::DbError;
use kabados_domain::DomainError;
use kabados_domain::api::Problem;
use kabados_domain::ruleset::ValidationIssue;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("sign in required")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("validation failed")]
    Invalid(Vec<ValidationIssue>),
    #[error("too many requests")]
    RateLimited,
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn bad(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Forbidden(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let detail = match &self {
            Self::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                None
            }
            Self::Invalid(_) => Some("Fix the listed fields and try again.".to_owned()),
            other => Some(other.to_string()),
        };
        let errors = match self {
            Self::Invalid(v) => v,
            _ => vec![],
        };
        let problem = Problem {
            type_: format!("https://kabados.in/problems/{}", slug_for(status)),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            detail,
            instance: None,
            errors,
        };
        (status, [(header::CONTENT_TYPE, "application/problem+json")], axum::Json(problem)).into_response()
    }
}

fn slug_for(s: StatusCode) -> &'static str {
    match s {
        StatusCode::BAD_REQUEST => "bad-request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not-found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::UNPROCESSABLE_ENTITY => "validation",
        StatusCode::TOO_MANY_REQUESTS => "rate-limited",
        StatusCode::SERVICE_UNAVAILABLE => "unavailable",
        _ => "internal",
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        if e.is_conflict() {
            ApiError::Conflict("This was changed by someone else. Refresh and try again.".into())
        } else {
            ApiError::Internal(anyhow::anyhow!(e))
        }
    }
}

impl From<DomainError> for ApiError {
    fn from(e: DomainError) -> Self {
        match e {
            DomainError::InvalidTransition { from, to } => {
                ApiError::Conflict(format!("Not possible right now (state {from}, wanted {to})."))
            }
            DomainError::InvalidId(m) => ApiError::BadRequest(m),
            DomainError::Validation(m) => ApiError::BadRequest(m),
            DomainError::UnknownCategory(c) => ApiError::BadRequest(format!("unknown category {c}")),
            DomainError::Overflow => ApiError::BadRequest("amount out of range".into()),
        }
    }
}
