use axum::{
    Router,
    routing::{get, post},
};

use crate::{handlers, store::Store};

pub(crate) fn router(store: Store) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/users", post(handlers::create_user))
        .route("/users/{id}", get(handlers::get_user))
        .route("/users/{id}/orders", post(handlers::place_order))
        .with_state(store)
}
