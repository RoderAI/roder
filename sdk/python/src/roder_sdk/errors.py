from __future__ import annotations

from typing import Any


class RoderRpcError(Exception):
    def __init__(
        self,
        *,
        code: int,
        message: str,
        data: Any = None,
        method: str,
        request_id: str | int | None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.data = data
        self.method = method
        self.request_id = request_id


class RoderTransportError(Exception):
    pass
