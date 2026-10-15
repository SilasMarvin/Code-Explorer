use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    error::AppError,
    models::{NewOrder, NewUser, Order, User, UserId},
    services::{orders, users},
    store::Store,
};

pub(crate) async fn health() -> &'static str {
    "ok"
}

pub(crate) async fn get_user(
    State(store): State<Store>,
    Path(id): Path<UserId>,
) -> Result<Json<User>, AppError> {
    let user = users::find_user(&store, id).await?;
    Ok(Json(user))
}

pub(crate) async fn create_user(
    State(store): State<Store>,
    Json(input): Json<NewUser>,
) -> Result<Json<User>, AppError> {
    let user = users::register(&store, input).await?;
    Ok(Json(user))
}

pub(crate) async fn place_order(
    State(store): State<Store>,
    Path(user_id): Path<UserId>,
    Json(input): Json<NewOrder>,
) -> Result<Json<Order>, AppError> {
    let order = orders::place(&store, user_id, input).await?;
    Ok(Json(order))
}
