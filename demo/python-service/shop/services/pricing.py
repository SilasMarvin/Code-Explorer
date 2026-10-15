from shop.errors import Invalid

BULK_THRESHOLD = 10
PRICES = {"widget": 1_250, "gadget": 4_999}


def quote(sku, quantity):
    subtotal = unit_price(sku) * quantity
    return apply_discount(subtotal, quantity)


def unit_price(sku):
    match sku:
        case "widget" | "gadget":
            return PRICES[sku]
        case _:
            raise Invalid(f"unknown sku {sku}")


def apply_discount(subtotal, quantity):
    if quantity >= BULK_THRESHOLD * 10:
        return subtotal - bulk_discount(subtotal) * 2
    elif quantity >= BULK_THRESHOLD:
        return subtotal - bulk_discount(subtotal)
    else:
        return subtotal


def bulk_discount(subtotal):
    return subtotal // 10


def quote_all(lines):
    totals = [quote(sku, quantity) for sku, quantity in lines if quantity > 0]
    cheapest_first = sorted(lines, key=lambda line: unit_price(line[0]))
    return totals, cheapest_first
