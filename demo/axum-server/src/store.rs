use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::sync::RwLock;

use crate::models::{Order, OrderId, User, UserId};

#[derive(Clone)]
pub(crate) struct Store {
    inner: Arc<Inner>,
}

struct Inner {
    users: RwLock<HashMap<UserId, User>>,
    orders: RwLock<HashMap<OrderId, Order>>,
    next_id: AtomicU64,
}

impl Store {
    pub(crate) fn seeded() -> Self {
        let ada = User {
            id: 1,
            name: "Ada".into(),
            email: "ada@example.com".into(),
        };
        let inner = Inner {
            users: RwLock::new(HashMap::from([(ada.id, ada)])),
            orders: RwLock::default(),
            next_id: AtomicU64::new(1),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    pub(crate) async fn user(&self, id: UserId) -> Option<User> {
        self.inner.users.read().await.get(&id).cloned()
    }

    pub(crate) async fn order(&self, id: OrderId) -> Option<Order> {
        self.inner.orders.read().await.get(&id).cloned()
    }

    pub(crate) async fn email_taken(&self, email: &str) -> bool {
        self.inner
            .users
            .read()
            .await
            .values()
            .any(|user| user.email == email)
    }

    pub(crate) async fn insert_user(&self, name: String, email: String) -> User {
        let user = User {
            id: self.next_id(),
            name,
            email,
        };
        self.inner.users.write().await.insert(user.id, user.clone());
        user
    }

    pub(crate) async fn insert_order(
        &self,
        user_id: UserId,
        sku: String,
        quantity: u32,
        total_cents: u64,
    ) -> Order {
        let order = Order {
            id: self.next_id(),
            user_id,
            sku,
            quantity,
            total_cents,
        };
        self.inner
            .orders
            .write()
            .await
            .insert(order.id, order.clone());
        order
    }

    fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }
}
