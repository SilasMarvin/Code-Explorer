"""In-memory persistence."""

import itertools


class Store:
    """Users and orders, keyed by id."""

    def __init__(self):
        self._users = {}
        self._orders = {}
        self._ids = itertools.count(1)

    @classmethod
    def seeded(cls):
        store = cls()
        store._put_user("Ada", "ada@example.com")
        return store

    async def user(self, user_id):
        return self._users.get(user_id)

    async def email_taken(self, email):
        return any(user["email"] == email for user in self._users.values())

    async def insert_user(self, name, email):
        return self._put_user(name, email)

    async def insert_order(self, user_id, sku, quantity, total_cents):
        order = {
            "id": self._next_id(),
            "user_id": user_id,
            "sku": sku,
            "quantity": quantity,
            "total_cents": total_cents,
        }
        self._orders[order["id"]] = order
        return order

    def _put_user(self, name, email):
        user = {"id": self._next_id(), "name": name, "email": email}
        self._users[user["id"]] = user
        return user

    def _next_id(self):
        return next(self._ids)


_store = None


def get_store():
    if _store is None:
        raise RuntimeError("store not initialised")
    return _store


async def init_db():
    global _store
    _store = Store.seeded() if _store is None else Store()
