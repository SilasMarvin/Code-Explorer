"""Application wiring: routers, lifecycle hooks and the dev server."""

import logging

from fastapi import FastAPI

from .routes import orders, users
from .store import init_db

app = FastAPI(title="shop")
app.include_router(users.router)
app.include_router(orders.router)
app.add_event_handler("startup", init_db)


def configure_logging(verbose):
    level = logging.DEBUG if verbose else logging.INFO
    logging.basicConfig(level=level)


if __name__ == "__main__":
    import sys

    import uvicorn

    configure_logging("-v" in sys.argv)
    uvicorn.run(app, host="127.0.0.1", port=3000)
