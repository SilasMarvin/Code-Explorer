from fastapi import APIRouter

import shop.services.orders as order_service
from shop.audit import audited
from shop.errors import AppError
from shop.store import get_store

router = APIRouter()


@router.post("/users/{user_id}/orders")
@audited("place_order")
async def place_order(user_id: int, body: dict):
    try:
        return await order_service.place(get_store(), user_id, body["sku"], body["quantity"])
    except AppError as error:
        return error.to_response()


@router.post("/users/{user_id}/orders/bulk")
async def place_orders(user_id: int, body: list):
    store = get_store()
    return [await order_service.place(store, user_id, line["sku"], line["quantity"]) for line in body]
