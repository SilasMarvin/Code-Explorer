from ..errors import Invalid

MAX_QUANTITY = 1_000


def normalize_name(raw):
    name = raw.strip()
    if not name:
        raise Invalid("name must not be empty")
    return name


def normalize_email(raw):
    email = raw.strip().lower()
    if not looks_like_email(email):
        raise Invalid(f"{email} is not an email address")
    return email


def check_quantity(quantity):
    if quantity == 0 or quantity > MAX_QUANTITY:
        raise Invalid(f"quantity must be between 1 and {MAX_QUANTITY}")
    return quantity


def looks_like_email(candidate):
    local, _, domain = candidate.partition("@")
    return bool(local) and "." in domain
