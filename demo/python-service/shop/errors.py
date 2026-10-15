"""Errors the service maps to HTTP responses."""


class AppError(Exception):
    """Base class for errors with an HTTP status."""

    status = 500

    def to_response(self):
        return {"status": self.status, "error": str(self)}


class NotFound(AppError):
    status = 404

    def __init__(self, what):
        super().__init__(f"{what} not found")


class Invalid(AppError):
    status = 422
