use crate::error::AppError;

const MAX_QUANTITY: u32 = 1_000;

pub(crate) fn normalize_name(raw: &str) -> Result<String, AppError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("name must not be empty".into()));
    }
    Ok(name.to_owned())
}

pub(crate) fn normalize_email(raw: &str) -> Result<String, AppError> {
    let email = raw.trim().to_lowercase();
    if !looks_like_email(&email) {
        return Err(AppError::Invalid(format!(
            "{email} is not an email address"
        )));
    }
    Ok(email)
}

pub(crate) fn check_quantity(quantity: u32) -> Result<u32, AppError> {
    if quantity == 0 {
        return Err(AppError::Invalid("quantity must be at least 1".into()));
    }
    if quantity > MAX_QUANTITY {
        return Err(AppError::Invalid(format!(
            "quantity must be at most {MAX_QUANTITY}"
        )));
    }
    Ok(quantity)
}

fn looks_like_email(candidate: &str) -> bool {
    candidate
        .split_once('@')
        .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
}
