from fastapi import APIRouter

from ..audit import audited
from ..services import users as user_service
from ..store import get_store

router = APIRouter()


@router.get("/health")
async def health():
    return "ok"


@router.get("/users/{user_id}")
async def get_user(user_id: int):
    return await user_service.find_user(get_store(), user_id)


@router.post("/users")
@audited("create_user")
async def create_user(body: dict):
    return await user_service.register(get_store(), body["name"], body["email"])
