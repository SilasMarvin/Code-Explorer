use crate::error::AppError;

const BULK_THRESHOLD: u32 = 10;

pub(crate) fn quote(sku: &str, quantity: u32) -> Result<u64, AppError> {
    let unit = unit_price(sku)?;
    let subtotal = unit * u64::from(quantity);
    Ok(apply_discount(subtotal, quantity))
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
        subtotal - bulk_discount(subtotal)
    } else {
        subtotal
    }
}

fn bulk_discount(subtotal: u64) -> u64 {
    subtotal / 10
}
