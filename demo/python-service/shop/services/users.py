from ..errors import Invalid, NotFound
from . import validation


async def find_user(store, user_id):
    user = await store.user(user_id)
    if user is None:
        raise NotFound("user")
    return user


async def register(store, raw_name, raw_email):
    name = validation.normalize_name(raw_name)
    email = validation.normalize_email(raw_email)
    if await store.email_taken(email):
        raise Invalid(f"{email} is already registered")
    return await store.insert_user(name, email)
