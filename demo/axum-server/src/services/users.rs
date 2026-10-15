use crate::{
    error::AppError,
    models::{NewUser, User, UserId},
    store::Store,
};

use super::validation;

pub(crate) async fn find_user(store: &Store, id: UserId) -> Result<User, AppError> {
    store.user(id).await.ok_or(AppError::NotFound("user"))
}

pub(crate) async fn register(store: &Store, input: NewUser) -> Result<User, AppError> {
    let name = validation::normalize_name(&input.name)?;
    let email = validation::normalize_email(&input.email)?;
    if store.email_taken(&email).await {
        return Err(AppError::Invalid(format!("{email} is already registered")));
    }
    let user = store.insert_user(name, email).await;
    tracing::info!(id = user.id, "registered user");
    Ok(user)
}
