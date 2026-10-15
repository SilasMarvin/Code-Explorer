use crate::{
    error::AppError,
    models::{NewOrder, Order, UserId},
    store::Store,
};

use super::{pricing, users, validation};

pub(crate) async fn place(
    store: &Store,
    user_id: UserId,
    input: NewOrder,
) -> Result<Order, AppError> {
    let user = users::find_user(store, user_id).await?;
    let quantity = validation::check_quantity(input.quantity)?;
    let total_cents = pricing::quote(&input.sku, quantity)?;
    let order = store
        .insert_order(user.id, input.sku, quantity, total_cents)
        .await;
    tracing::info!(
        order = order.id,
        user = user.id,
        total_cents,
        "placed order"
    );
    Ok(order)
}
