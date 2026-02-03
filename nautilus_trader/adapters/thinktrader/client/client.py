import asyncio
from typing import Any


try:
    from xtquant.xttrader import XtQuantTraderCallback
except ModuleNotFoundError:  # pragma: no cover
    class XtQuantTraderCallback:  # type: ignore[no-redef]
        pass

from nautilus_trader.adapters.thinktrader.client.account import ThinkTraderClientAccountMixin
from nautilus_trader.adapters.thinktrader.client.common import Request
from nautilus_trader.adapters.thinktrader.client.common import Requests
from nautilus_trader.adapters.thinktrader.client.common import Subscriptions
from nautilus_trader.adapters.thinktrader.client.connection import ThinkTraderClientConnectionMixin
from nautilus_trader.adapters.thinktrader.client.contract import ThinkTraderClientContractMixin
from nautilus_trader.adapters.thinktrader.client.error import ThinkTraderClientErrorMixin
from nautilus_trader.adapters.thinktrader.client.market_data import ThinkTraderClientMarketDataMixin
from nautilus_trader.adapters.thinktrader.client.order import ThinkTraderClientOrderMixin
from nautilus_trader.common.component import Logger


class ThinkTraderClientCallback(XtQuantTraderCallback):
    """XtQuant 回调实现"""

    def __init__(self, client: "ThinkTraderClient") -> None:
        super().__init__()
        self._client = client

    def on_disconnected(self) -> None:
        """连接断开"""
        self._client._on_disconnected()

    def on_account_status(self, status: Any) -> None:
        """账号状态变更"""
        self._client._on_account_status(status)

    def on_stock_order(self, order: Any) -> None:
        """委托回报"""
        self._client._on_order(order)

    def on_stock_trade(self, trade: Any) -> None:
        """成交回报"""
        self._client._on_trade(trade)

    def on_order_error(self, order_error: Any) -> None:
        """下单错误"""
        self._client._handle_order_error(order_error)

    def on_cancel_error(self, cancel_error: Any) -> None:
        """撤单错误"""
        self._client._handle_cancel_error(cancel_error)

    def on_order_stock_async_response(self, response: Any) -> None:
        """异步下单回报"""
        self._client._on_order_async_response(response)


class ThinkTraderClient(
    ThinkTraderClientConnectionMixin,
    ThinkTraderClientMarketDataMixin,
    ThinkTraderClientAccountMixin,
    ThinkTraderClientOrderMixin,
    ThinkTraderClientContractMixin,
    ThinkTraderClientErrorMixin,
):
    """
    ThinkTrader 底层客户端

    聚合所有 Mixin 功能, 与 XtQuant API 交互。
    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        logger: Logger,
        miniqmt_path: str,
        session_id: int,
        account_id: str,
    ) -> None:
        self._loop = loop
        self._log = logger
        self._miniqmt_path = miniqmt_path
        self._session_id = session_id
        self._account_id = account_id

        self._trader = None
        self._account = None
        self._callback = ThinkTraderClientCallback(self)

        self._is_connected = asyncio.Event()
        self._subscriptions = Subscriptions()
        self._requests = Requests()
        self._req_id = 1000  # Start from 1000 to avoid conflicts with system seq
        self._event_handlers: dict = {}

    def _next_req_id(self) -> int:
        """生成下一个请求 ID"""
        self._req_id += 1
        return self._req_id

    async def _await_request(
        self,
        request: Request,
        timeout: int,
        default_value: Any = None,
    ) -> Any:
        """等待异步请求完成"""
        try:
            return await asyncio.wait_for(request.future, timeout=timeout)
        except TimeoutError:
            self._log.error(f"请求 {request.req_id} ({request.name}) 超时 ({timeout}s)")
            self._requests.remove(req_id=request.req_id)
            return default_value

    def register_event_handler(self, event_name: str, handler: Any) -> None:
        """注册事件处理器"""
        self._event_handlers[event_name] = handler

    def _on_disconnected(self) -> None:
        """处理连接断开"""
        self._log.warning("与 MiniQmt 的连接已断开")
        self._is_connected.clear()
        if handler := self._event_handlers.get("disconnected"):
            self._loop.call_soon_threadsafe(handler)

    def _on_account_status(self, status: Any) -> None:
        """处理账号状态变更"""
        self._log.info(
            f"账号状态: {status.account_id}, "
            f"type={status.account_type}, status={status.status}"
        )

    def _on_order(self, order: Any) -> None:
        """处理委托回报"""
        self._loop.call_soon_threadsafe(
            self._handle_order_update,
            order,
        )

    def _on_trade(self, trade: Any) -> None:
        """处理成交回报"""
        self._loop.call_soon_threadsafe(
            self._handle_trade,
            trade,
        )

    def _on_order_async_response(self, response: Any) -> None:
        """处理异步下单回报"""
        self._log.debug(
            f"异步下单回报: account={response.account_id}, "
            f"order_id={response.order_id}, seq={response.seq}"
        )

    def _handle_order_update(self, order: Any) -> None:
        """处理委托更新 (在主循环中执行)"""
        if handler := self._event_handlers.get("order_update"):
            handler(order)

    def _handle_trade(self, trade: Any) -> None:
        """处理成交 (在主循环中执行)"""
        if handler := self._event_handlers.get("trade"):
            handler(trade)
