const STANDARD_RATE_PERCENT: u64 = 8;
const REDUCED_RATE_PERCENT: u64 = 5;

pub(crate) fn apply(sku: &str, subtotal: u64) -> u64 {
    subtotal + subtotal * rate_for(sku) / 100
}

fn rate_for(sku: &str) -> u64 {
    match sku {
        "widget" => REDUCED_RATE_PERCENT,
        _ => STANDARD_RATE_PERCENT,
    }
}
