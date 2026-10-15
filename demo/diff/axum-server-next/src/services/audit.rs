use crate::{models::Order, store::Store};

pub(crate) async fn record(store: &Store, order: &Order) {
    let line = describe(order);
    store.append_audit(line).await;
}

fn describe(order: &Order) -> String {
    format!(
        "order {} by user {}: {} x {} for {} cents",
        order.id, order.user_id, order.quantity, order.sku, order.total_cents
    )
}
