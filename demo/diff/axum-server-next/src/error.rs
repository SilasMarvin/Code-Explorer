use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug)]
pub(crate) enum AppError {
    NotFound(&'static str),
    Invalid(String),
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl AppError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::NotFound(what) => format!("{what} not found"),
            Self::Invalid(reason) => reason.clone(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: self.message(),
        };
        (self.status(), Json(body)).into_response()
    }
}
