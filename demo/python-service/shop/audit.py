"""Audit logging around handlers."""

import functools
import logging

log = logging.getLogger(__name__)


def audited(action):
    """Logs the start and end of every call to the decorated coroutine."""

    def decorate(handler):
        @functools.wraps(handler)
        async def wrapper(*args, **kwargs):
            log.info("begin %s", action)
            try:
                return await handler(*args, **kwargs)
            except Exception:
                log.exception("failed %s", action)
                raise
            finally:
                log.info("end %s", action)

        return wrapper

    return decorate
