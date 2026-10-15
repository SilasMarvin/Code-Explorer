use crate::model::Order;

pub struct Store;
pub struct Row;

impl Store {
    pub fn open() -> Self {
        Store
    }

    pub fn fetch(&self, _id: u32) -> Row {
        Row
    }

    pub fn load(&self, id: u32) -> Result<Order, String> {
        if id == 0 {
            return Err("missing".into());
        }
        Ok(Order { quantity: 1, kind: 0 })
    }

    pub async fn refresh(&self) {}
}

impl Row {
    pub fn finish(self) {}
}
