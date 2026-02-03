import asyncio
import functools
from abc import ABC
from abc import abstractmethod
from collections.abc import Callable
from typing import Annotated
from typing import Any
from typing import NamedTuple

import msgspec


class TTPosition(NamedTuple):
    """ThinkTrader 持仓结构"""

    account_id: str
    stock_code: str
    volume: int
    available_volume: int
    avg_price: float
    market_value: float


class TTOrder(NamedTuple):
    """ThinkTrader 订单结构"""

    order_id: int
    order_sysid: str
    stock_code: str
    order_type: int
    order_volume: int
    traded_volume: int
    price: float
    traded_price: float
    order_status: int
    order_remark: str


class TTTrade(NamedTuple):
    """ThinkTrader 成交结构"""

    account_id: str
    order_id: int
    order_sysid: str
    stock_code: str
    traded_id: str
    traded_price: float
    traded_volume: int
    traded_time: int


class Request(msgspec.Struct, frozen=True):
    """
    数据请求详情的容器。
    """

    req_id: Annotated[int, msgspec.Meta(gt=0)]
    name: str | tuple
    handle: Callable
    cancel: Callable
    future: asyncio.Future
    result: list[Any]

    def __hash__(self) -> int:
        return hash((self.req_id, self.name))


class Subscription(msgspec.Struct, frozen=True):
    """
    订阅详情的容器。
    """

    req_id: Annotated[int, msgspec.Meta(gt=0)]
    name: str | tuple
    handle: functools.partial | Callable
    cancel: Callable
    last: Any

    def __hash__(self) -> int:
        return hash((self.req_id, self.name))


class Base(ABC):
    """
    维护订阅和数据请求的请求 ID (Request Id) 映射的抽象基类。
    """

    def __init__(self) -> None:
        self._req_id_to_name: dict[int, str | tuple] = {}
        self._req_id_to_handle: dict[int, Callable] = {}
        self._req_id_to_cancel: dict[int, Callable] = {}

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}:\n{[self.get(req_id=k) for k in self._req_id_to_name]!r}"

    def _name_to_req_id(self, name: Any) -> int | None:
        for req_id, req_name in self._req_id_to_name.items():
            if req_name == name:
                return req_id
        return None

    def _validation_check(self, req_id: int, name: Any) -> None:
        if req_id in self._req_id_to_name:
            existing = self.get(req_id=req_id)
            raise KeyError(f"不允许重复输入 {req_id=}, 现有条目: {existing}")
        if name in self._req_id_to_name.values():
            existing = self.get(name=name)
            raise KeyError(f"不允许重复输入 {name=}, 现有条目: {existing}")

    def add_req_id(
        self,
        req_id: int,
        name: str | tuple,
        handle: Callable,
        cancel: Callable,
    ) -> None:
        self._validation_check(req_id, name)
        self._req_id_to_name[req_id] = name
        self._req_id_to_handle[req_id] = handle
        self._req_id_to_cancel[req_id] = cancel

    def remove_req_id(self, req_id: int) -> None:
        self._req_id_to_name.pop(req_id, None)
        self._req_id_to_handle.pop(req_id, None)
        self._req_id_to_cancel.pop(req_id, None)

    def remove(
        self,
        req_id: int | None = None,
        name: str | tuple | None = None,
    ) -> None:
        if req_id is None:
            req_id = self._name_to_req_id(name)
            if req_id is None:
                return
        self.remove_req_id(req_id)

    def get_all(self) -> list[Request | Subscription]:
        result: list = []
        for req_id in self._req_id_to_name:
            result.append(self.get(req_id=req_id))
        return result

    @abstractmethod
    def get(
        self,
        req_id: int | None = None,
        name: str | tuple | None = None,
    ) -> Request | Subscription | None:
        """检索 Request 或 Subscription 对象的抽象方法。"""


class Subscriptions(Base):
    """
    管理和存储通过请求 ID 标识和访问的订阅。
    """

    def __init__(self) -> None:
        super().__init__()
        self._req_id_to_last: dict[int, Any] = {}

    def add(
        self,
        req_id: int,
        name: str | tuple,
        handle: Callable,
        cancel: Callable = lambda: None,
    ) -> Subscription:
        super().add_req_id(req_id, name, handle, cancel)
        self._req_id_to_last[req_id] = None
        subscription = self.get(req_id=req_id)
        assert subscription is not None
        return subscription

    def remove(self, req_id: int | None = None, name: str | tuple | None = None) -> None:
        if not req_id:
            req_id = self._name_to_req_id(name)
        if req_id:
            super().remove_req_id(req_id)
            self._req_id_to_last.pop(req_id, None)

    def get(
        self,
        req_id: int | None = None,
        name: str | tuple | None = None,
    ) -> Subscription | None:
        if not req_id:
            req_id = self._name_to_req_id(name)
        if not req_id or not (name := self._req_id_to_name.get(req_id, None)):
            return None
        return Subscription(
            req_id=req_id,
            name=name,
            last=self._req_id_to_last[req_id],
            handle=self._req_id_to_handle[req_id],
            cancel=self._req_id_to_cancel[req_id],
        )

    def update_last(self, req_id: int, value: Any) -> None:
        self._req_id_to_last[req_id] = value


class Requests(Base):
    """
    管理和存储数据请求。
    """

    def __init__(self) -> None:
        super().__init__()
        self._req_id_to_future: dict[int, asyncio.Future] = {}
        self._req_id_to_result: dict[int, Any] = {}

    def get_futures(self) -> list[asyncio.Future]:
        return list(self._req_id_to_future.values())

    def add(
        self,
        req_id: int,
        name: str | tuple,
        handle: Callable,
        cancel: Callable = lambda: None,
    ) -> Request:
        super().add_req_id(req_id, name, handle, cancel)
        self._req_id_to_future[req_id] = asyncio.Future()
        self._req_id_to_result[req_id] = []
        request = self.get(req_id=req_id)
        assert request is not None
        return request

    def remove(self, req_id: int | None = None, name: str | tuple | None = None) -> None:
        if not req_id:
            req_id = self._name_to_req_id(name)
        if req_id:
            super().remove_req_id(req_id)
            self._req_id_to_future.pop(req_id, None)
            self._req_id_to_result.pop(req_id, None)

    def get(
        self,
        req_id: int | None = None,
        name: str | tuple | None = None,
    ) -> Request | None:
        if not req_id:
            req_id = self._name_to_req_id(name)
        if not req_id or not (name := self._req_id_to_name.get(req_id, None)):
            return None
        return Request(
            req_id=req_id,
            name=name,
            handle=self._req_id_to_handle[req_id],
            cancel=self._req_id_to_cancel[req_id],
            future=self._req_id_to_future[req_id],
            result=self._req_id_to_result[req_id],
        )


class BaseMixin(ABC):
    """Mixin 基类, 提供类型提示"""

    _loop: asyncio.AbstractEventLoop
    _log: Any
    _trader: Any
    _clock: Any
    _cache: Any
    _msgbus: Any
    _account: Any
    _miniqmt_path: str
    _session_id: int
    _account_id: str
    _callback: Any
    _is_connected: asyncio.Event
    _event_handlers: dict[str, Any]
    _subscriptions: Subscriptions
    _requests: Requests
    _req_id: int
    _next_req_id: Callable[[], int]
    _await_request: Callable[..., Any]
