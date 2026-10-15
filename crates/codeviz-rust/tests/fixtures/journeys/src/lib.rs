//! Each `pub fn` here is a journey entry with a golden outline in `../expected/`.

mod model;
mod store;
mod work;

use model::{Order, Shape};
use store::Store;

/// Arguments are evaluated before the call that receives them; chains run left to right.
pub fn evaluation_order() {
    outer(inner(), work::helper());
    Store::open().fetch(1).finish();
}

/// `?` propagates; the tail call of a `Result` function propagates too.
pub fn propagation(store: &Store) -> Result<u32, String> {
    let order = store.load(1)?;
    work::validate(&order)?;
    work::total(&order)
}

/// Branches, early returns, and a match whose arms exit differently.
pub fn branching(order: &Order) -> Result<(), String> {
    if order.quantity == 0 {
        return Err("empty".into());
    }
    match order.kind {
        0 => work::small(order),
        1 => work::large(order),
        _ => return Err("unknown kind".into()),
    }
    work::audit(order);
    Ok(())
}

/// Loop bodies run repeatedly; the condition of a `while` runs every iteration.
pub fn looping(orders: &[Order]) {
    for order in orders {
        work::price(order);
    }
    while work::pending() {
        work::drain();
    }
}

/// Closures handed to iterator adapters run during the call; spawned work runs
/// concurrently; a stored closure runs where it is called.
pub fn closures(orders: Vec<Order>) -> u32 {
    let total = orders.iter().map(|o| work::price(o)).sum();
    std::thread::spawn(move || work::drain());
    let later = || work::audit_all();
    later();
    total
}

/// `.await` keeps order; `tokio::spawn` detaches.
pub async fn asynchronous(store: &Store) {
    store.refresh().await;
    tokio::spawn(async move { work::drain() });
    work::notify().await;
}

/// Trait dispatch: through a generic parameter or a trait object.
pub fn dispatch<S: Shape>(shape: &S, dynamic: &dyn Shape) -> f64 {
    shape.area() + dynamic.area()
}

/// Recursion is cut where a function is already on the stack.
pub fn recursion(n: u32) -> u32 {
    if n == 0 { 0 } else { recursion(n - 1) + work::ping(n) }
}

/// Methods reached through struct fields and `Self`.
pub fn fields() {
    let service = work::Service::new();
    service.run();
}

fn outer(_a: u32, _b: u32) {}
fn inner() -> u32 {
    1
}

/// `?` applies to the awaited call even when its arguments are plain bindings.
pub async fn awaited(store: &Store, id: u32) -> Result<(), String> {
    let order = store.load(id)?;
    work::confirm(&order, id).await?;
    Ok(())
}
