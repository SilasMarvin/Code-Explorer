pub struct Order {
    pub quantity: u32,
    pub kind: u8,
}

pub trait Shape {
    fn area(&self) -> f64;
}

pub struct Circle;
pub struct Square;

impl Shape for Circle {
    fn area(&self) -> f64 {
        3.14
    }
}

impl Shape for Square {
    fn area(&self) -> f64 {
        1.0
    }
}
