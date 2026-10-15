import asyncio
import logging

from . import pricing, users
from .validation import check_quantity

log = logging.getLogger(__name__)


async def place(store, user_id, sku, quantity):
    """Places an order for a user."""
    user = await users.find_user(store, user_id)
    quantity = check_quantity(quantity)
    total_cents = pricing.quote(sku, quantity)
    order = await store.insert_order(user["id"], sku, quantity, total_cents)
    asyncio.create_task(notify(order))
    log.info("placed order %s for user %s", order["id"], user["id"])
    return order


async def notify(order):
    def describe():
        return f"order {order['id']}: {order['quantity']} x {order['sku']}"

    attempts = 0
    while attempts < 3:
        attempts += 1
        try:
            await send(describe())
            return
        except ConnectionError:
            await asyncio.sleep(attempts)
    log.warning("gave up notifying about order %s", order["id"])


async def send(message):
    for chunk in message.split("\n"):
        log.info("notify: %s", chunk)
