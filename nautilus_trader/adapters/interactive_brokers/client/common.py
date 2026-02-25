# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------

import asyncio
import functools
from abc import ABC
from abc import abstractmethod
from collections.abc import Callable
from decimal import Decimal
from typing import Annotated
from typing import Any
from typing import NamedTuple

import msgspec
from ibapi.client import EClient
from ibapi.commission_and_fees_report import CommissionAndFeesReport
from ibapi.common import BarData
from ibapi.execution import Execution

from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import MessageBus
from nautilus_trader.model.data import BarType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import VenueOrderId


class AccountOrderRef(NamedTuple):
    account_id: str
    order_id: str


def get_venue_order_id(order_id: int, perm_id: int) -> VenueOrderId:
    """
    获取交易所订单 ID。对于外部订单（orderId=0），使用 permId。

    IB 将 orderId=0 分配给外部订单（通过 TWS 或其他客户端下达的订单）以及已完成
    的订单。由于可能有多个订单的 orderId 均为 0，我们使用唯一的 permId 来标识它们。

    参数
    ----------
    order_id : int
        IB 订单 ID。
    perm_id : int
        永久订单 ID（在所有订单中唯一）。

    返回
    -------
    VenueOrderId

    """
    if order_id != 0:
        return VenueOrderId(str(order_id))
    return VenueOrderId(f"PERM-{perm_id}")


class IBPosition(NamedTuple):
    account_id: str
    contract: IBContract
    quantity: Decimal
    avg_cost: float


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
    维护订阅和数据请求的请求 ID（Request Id）映射的抽象基类。
    """

    def __init__(self) -> None:
        self._req_id_to_name: dict[int, str | tuple] = {}
        self._req_id_to_handle: dict[int, Callable] = {}
        self._req_id_to_cancel: dict[int, Callable] = {}

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}:\n{[self.get(req_id=k) for k in self._req_id_to_name]!r}"

    def _name_to_req_id(self, name: Any) -> int | None:
        """
        将给定名称映射到其对应的请求 ID。

        参数
        ----------
        name : Any
            要查找其对应请求 ID 的名称。

        返回
        -------
        str

        """
        for req_id, req_name in self._req_id_to_name.items():
            if req_name == name:
                return req_id

        return None

    def _validation_check(self, req_id: int, name: Any) -> None:
        """
        验证提供的请求 ID 和名称是否尚未被使用。

        参数
        ----------
        req_id : int
            要验证的请求 ID。
        name : Any
            要验证的名称。

        引发
        ------
        KeyError
            如果请求 ID 或名称已被使用。

        """
        if req_id in self._req_id_to_name:
            existing = self.get(req_id=req_id)
            raise KeyError(f"不允许重复输入 {req_id=}，现有条目：{existing}")
        if name in self._req_id_to_name.values():
            existing = self.get(name=name)
            raise KeyError(f"不允许重复输入 {name=}，现有条目：{existing}")

    def add_req_id(
        self,
        req_id: int,
        name: str | tuple,
        handle: Callable,
        cancel: Callable,
    ) -> None:
        """
        向映射中添加新的请求 ID 及其关联的名称、处理函数和取消回调。

        参数
        ----------
        req_id : int
            要添加的请求 ID。
        name : str | tuple
            与请求 ID 关联的名称。
        handle : Callable
            请求的处理函数。
        cancel : Callable
            请求的取消回调函数。

        """
        self._validation_check(req_id, name)
        self._req_id_to_name[req_id] = name
        self._req_id_to_handle[req_id] = handle
        self._req_id_to_cancel[req_id] = cancel

    def remove_req_id(self, req_id: int) -> None:
        """
        从此类中移除请求 ID 及其关联的映射。

        参数
        ----------
        req_id : int
            要移除的请求 ID。

        """
        self._req_id_to_name.pop(req_id, None)
        self._req_id_to_handle.pop(req_id, None)
        self._req_id_to_cancel.pop(req_id, None)

    def remove(
        self,
        req_id: int | None = None,
        name: InstrumentId | (BarType | str) | None = None,
    ) -> None:
        """
        通过请求 ID 或名称移除请求 ID 及其关联的映射。

        参数
        ----------
        req_id : int, 可选
            要移除的请求 ID。如果为 None，则使用名称来确定请求 ID。
        name : InstrumentId | (BarType | str), 可选
            与请求 ID 关联的名称。

        """
        if req_id is None:
            req_id = self._name_to_req_id(name)

            if req_id is None:
                return  # 如果找不到匹配的 req_id，则退出方法

        self._req_id_to_name.pop(req_id, None)
        self._req_id_to_handle.pop(req_id, None)
        self._req_id_to_cancel.pop(req_id, None)

    def get_all(self) -> list[Request | Subscription]:
        """
        检索所有存储的映射，并以各自的请求或订阅对象列表形式返回。

        返回
        -------
        list[Request | Subscription]

        """
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
        """
        根据请求 ID 或名称检索 Request 或 Subscription 对象的抽象方法。

        参数
        ----------
        req_id : int
            要检索的对象的请求 ID。如果为 None，则使用名称。
        name : str | tuple, 可选
            与请求 ID 关联的名称。

        返回
        -------
        Request | Subscription | ``None``

        """


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
    ) -> Subscription | None:
        """
        添加具有给定请求 ID、名称、处理函数和可选取消回调的新订阅。
        此方法存储订阅详情并将其“last”值初始化为 None。如果已存在具有给定请求
        ID 的订阅，它将被覆盖。

        参数
        ----------
        req_id : int
            新订阅的请求 ID。
        name : str | tuple
            与订阅关联的名称。
        handle : Callable
            订阅的处理函数。
        cancel : Callable, 可选
            订阅的取消回调函数。默认为空操作（no-op）的 lambda。

        返回
        -------
        Subscription | ``None``

        """
        super().add_req_id(req_id, name, handle, cancel)
        self._req_id_to_last[req_id] = None

        return self.get(req_id=req_id)

    def remove(self, req_id: int | None = None, name: str | tuple | None = None) -> None:
        """
        移除通过请求 ID 或名称标识的订阅。如果通过名称标识订阅，则首先确定对应
        的请求 ID。如果既未提供 req_id 也未提供 name，或者找不到指定的订阅，
        则不执行任何操作。

        参数
        ----------
        req_id : int, 可选
            要移除的订阅的请求 ID。如果为 None，则使用名称。
        name : str | tuple, 可选
            要移除的订阅的名称。

        """
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
        """
        根据请求 ID 或名称检索订阅。

        参数
        ----------
        req_id : int, 可选
            要检索的订阅的请求 ID。如果为 None，则使用名称。
        name : str | tuple, 可选
            与请求 ID 关联的名称。

        返回
        -------
        Subscription | ``None``

        """
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
        """
        更新给定订阅的“last”值。

        参数
        ----------
        req_id : int
            要更新的订阅的请求 ID。
        value : Any
            要设置为该订阅“last”值的新值。

        """
        self._req_id_to_last[req_id] = value


class Requests(Base):
    """
    管理和存储数据请求，继承自 Base 类的通用功能。

    请求通过请求 ID 标识和访问。

    """

    def __init__(self) -> None:
        super().__init__()
        self._req_id_to_future: dict[int, asyncio.Future] = {}
        self._req_id_to_result: dict[int, Any] = {}

    def get_futures(self) -> list[asyncio.Future]:
        """
        检索与存储的请求关联的所有 asyncio Future。

        返回
        -------
        list[asyncio.Future]

        """
        return list(self._req_id_to_future.values())

    def add(
        self,
        req_id: int,
        name: str | tuple,
        handle: Callable,
        cancel: Callable = lambda: None,
    ) -> Request | None:
        """
        添加具有指定请求 ID、名称、处理函数和可选取消回调的新数据请求。
        此方法存储数据请求详情并初始化其 future 和 result。如果已存在具有
        给定请求 ID 的数据请求，它将被覆盖。

        参数
        ----------
        req_id : int
            新数据请求的请求 ID。
        name : str | tuple
            与数据请求关联的名称。
        handle : Callable
            数据请求的处理函数。
        cancel : Callable, 可选
            数据请求的取消回调函数。默认为空操作（no-op）的 lambda。

        返回
        -------
        Request | ``None``

        """
        super().add_req_id(req_id, name, handle, cancel)
        self._req_id_to_future[req_id] = asyncio.Future()
        self._req_id_to_result[req_id] = []

        return self.get(req_id=req_id)

    def remove(self, req_id: int | None = None, name: str | tuple | None = None) -> None:
        """
        移除通过请求 ID 或名称标识的数据请求。此方法从内部存储中移除数据请求
        详情。如果通过名称标识数据请求，则首先确定对应的请求 ID。如果既未提供
        req_id 也未提供 name，或者找不到指定的数据请求，则不执行任何操作。

        参数
        ----------
        req_id : int, 可选
            要移除的数据请求的请求 ID。如果为 None，则使用名称。
        name : str | tuple, 可选
            要移除的数据请求的名称。

        """
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
        """
        根据请求 ID 或名称检索 Request。

        参数
        ----------
        req_id : int, 可选
            要检索的请求的请求 ID。如果为 None，则使用名称。
        name : str | tuple, 可选
            与请求 ID 关联的名称。

        返回
        -------
        Request | ``None``

        """
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


class BaseMixin:
    """
    为 InteractiveBrokerClient Mixins 提供类型提示。
    """

    # Client
    is_running: bool
    _loop: asyncio.AbstractEventLoop
    _log: Logger
    _cache: Cache
    _clock: LiveClock
    _msgbus: MessageBus
    _host: str
    _port: int
    _client_id: int
    _request_timeout_secs: int
    _requests: Requests
    _instrument_provider: (
        Any  # InteractiveBrokersInstrumentProvider | None - 将由数据/执行客户端设置
    )
    _subscriptions: Subscriptions
    _event_subscriptions: dict[str, Callable]
    _eclient: EClient
    _is_ib_connected: asyncio.Event
    _start: Callable
    _startup: Callable
    _reset: Callable
    _stop: Callable
    _resume: Callable
    _degrade: Callable
    _end_request: Callable
    _await_request: Callable
    _next_req_id: Callable
    _resubscribe_all: Callable
    _create_task: Callable
    logAnswer: Callable

    # Account
    accounts: Callable

    # Connection
    _reconnect_attempts: int
    _reconnect_delay: int
    _max_reconnect_attempts: int
    _indefinite_reconnect: bool
    _last_disconnection_ns: int | None

    # MarketData
    _bar_type_to_last_bar: dict[str, BarData | None]
    _bar_timeout_tasks: dict[str, Any]  # asyncio.Task
    _order_id_to_order_ref: dict[VenueOrderId, AccountOrderRef]

    # Order
    _next_valid_order_id: int
    _exec_id_details: dict[
        str,
        dict[str, Execution | (CommissionAndFeesReport | str)],
    ]


class IBKRBookLevel(msgspec.Struct, frozen=True):
    """
    订单簿中的单个价格层级。

    属性
    ----------
    price : float
        此层级的价格。
    size : Decimal
        此价格下的总数量。
    side : OrderSide
        此价格下的订单方向。
    market_maker : str
        提供此报价的做市商标识符。

    """

    price: float
    size: Decimal
    side: OrderSide
    market_maker: str
