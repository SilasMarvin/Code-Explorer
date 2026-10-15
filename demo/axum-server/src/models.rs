use serde::{Deserialize, Serialize};

pub(crate) type UserId = u64;
pub(crate) type OrderId = u64;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct User {
    pub(crate) id: UserId,
    pub(crate) name: String,
    pub(crate) email: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Order {
    pub(crate) id: OrderId,
    pub(crate) user_id: UserId,
    pub(crate) sku: String,
    pub(crate) quantity: u32,
    pub(crate) total_cents: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NewUser {
    pub(crate) name: String,
    pub(crate) email: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NewOrder {
    pub(crate) sku: String,
    pub(crate) quantity: u32,
}
