use crate::error::AppError;

use super::tax;

const BULK_THRESHOLD: u32 = 10;

pub(crate) fn quote(sku: &str, quantity: u32) -> Result<u64, AppError> {
    let unit = unit_price(sku)?;
    let subtotal = unit * u64::from(quantity);
    let discounted = apply_discount(subtotal, quantity);
    Ok(tax::apply(sku, discounted))
}

fn unit_price(sku: &str) -> Result<u64, AppError> {
    match sku {
        "widget" => Ok(1_250),
        "gadget" => Ok(4_999),
        other => Err(AppError::Invalid(format!("unknown sku {other}"))),
    }
}

fn apply_discount(subtotal: u64, quantity: u32) -> u64 {
    if quantity >= BULK_THRESHOLD {
        subtotal - subtotal / 10
    } else {
        subtotal
    }
}
