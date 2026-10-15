use crate::{model::Order, store::Store};

pub fn helper() -> u32 {
    2
}

pub fn validate(order: &Order) -> Result<(), String> {
    if order.quantity > 100 {
        return Err("too many".into());
    }
    Ok(())
}

pub fn total(order: &Order) -> Result<u32, String> {
    price(order).checked_mul(2).ok_or_else(|| "overflow".to_owned())
}

pub fn price(order: &Order) -> u32 {
    order.quantity * 10
}

pub fn small(_order: &Order) {}
pub fn large(order: &Order) {
    price(order);
}
pub fn audit(_order: &Order) {}
pub fn audit_all() {}
pub fn pending() -> bool {
    false
}
pub fn drain() {}
pub async fn notify() {}
pub fn ping(n: u32) -> u32 {
    crate::recursion(n.saturating_sub(1))
}

pub struct Service {
    store: Store,
}

impl Service {
    pub fn new() -> Self {
        Service { store: Store::open() }
    }

    pub fn run(&self) {
        self.store.fetch(7).finish();
        Self::log();
    }

    fn log() {}
}

pub async fn confirm(order: &Order, id: u32) -> Result<(), String> {
    if id == 0 {
        return Err("unconfirmed".into());
    }
    validate(order)
}
