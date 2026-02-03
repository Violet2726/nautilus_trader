from typing import Any
from typing import Final

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin


class ThinkTraderClientErrorMixin(BaseMixin):
    """处理 ThinkTrader 客户端的错误和警告"""

    # 常见错误码分类
    CONNECTION_ERRORS: Final[set[int]] = {-1, -2, -3}
    ORDER_REJECTION_CODES: Final[set[int]] = {50001, 50002, 50003}
    AUTHENTICATION_ERRORS: Final[set[int]] = {10001, 10002}

    def _handle_order_error(self, order_error: Any) -> None:
        """处理下单错误"""
        error_id = order_error.error_id
        error_msg = order_error.error_msg
        order_id = order_error.order_id

        self._log.error(
            f"下单失败: order_id={order_id}, error_id={error_id}, error_msg={error_msg}"
        )

        # 触发订单拒绝事件
        if handler := self._event_handlers.get("order_rejected"):
            self._loop.call_soon_threadsafe(
                handler,
                order_id,
                error_msg,
            )

    def _handle_cancel_error(self, cancel_error: Any) -> None:
        """处理撤单错误"""
        error_id = cancel_error.error_id
        error_msg = cancel_error.error_msg
        order_id = cancel_error.order_id

        self._log.warning(
            f"撤单失败: order_id={order_id}, error_id={error_id}, error_msg={error_msg}"
        )

    def _handle_connection_error(self, error_code: int) -> None:
        """处理连接错误"""
        if error_code in self.CONNECTION_ERRORS:
            self._log.error(f"连接错误: {error_code}")
            self._is_connected.clear()
