from typing import Any

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin
from nautilus_trader.adapters.thinktrader.client.common import TTPosition


class ThinkTraderClientAccountMixin(BaseMixin):
    """处理账户和持仓查询"""

    def query_asset(self) -> dict[str, Any] | None:
        """查询账户资产"""
        try:
            asset = self._trader.query_stock_asset(self._account)
            if asset:
                return {
                    "account_id": asset.account_id,
                    "cash": asset.cash,
                    "frozen_cash": asset.frozen_cash,
                    "market_value": asset.market_value,
                    "total_asset": asset.total_asset,
                    "login_status": getattr(asset, "login_status", ""),
                    "account_status": getattr(asset, "account_status", ""),
                    "update_time": getattr(asset, "update_time", 0),
                    "profit_loss": getattr(asset, "profit_loss", 0.0),
                    "total_market_value": getattr(asset, "total_market_value", 0.0),
                    "withdrawable_amount": getattr(asset, "withdrawable_amount", 0.0),
                    "stock_market_value": getattr(asset, "stock_market_value", 0.0),
                    "fund_market_value": getattr(asset, "fund_market_value", 0.0),
                    "broker_name": getattr(asset, "broker_name", ""),
                    "account_name": getattr(asset, "account_name", ""),
                }
            return None
        except Exception as e:
            self._log.error(f"查询资产失败：{e}")
            return None

    def query_positions(self) -> list[TTPosition]:
        """查询持仓"""
        try:
            positions = self._trader.query_stock_positions(self._account)
            result: list[TTPosition] = []
            if positions:
                for pos in positions:
                    result.append(
                        TTPosition(
                            account_id=pos.account_id,
                            stock_code=pos.stock_code,
                            stock_name=getattr(pos, "stock_name", ""),
                            volume=pos.volume,
                            available_volume=pos.can_use_volume,
                            pending_volume=getattr(pos, "pending_volume", 0),
                            frozen_volume=getattr(pos, "frozen_volume", 0),
                            market_value=pos.market_value,
                            last_price=getattr(pos, "last_price", 0.0),
                            avg_price=pos.open_price,
                            float_pnl=getattr(pos, "float_pnl", getattr(pos, "floating_pnl", 0.0)),
                            profit_loss_ratio=getattr(pos, "profit_loss_ratio", 0.0),
                            overnight_volume=getattr(pos, "overnight_volume", 0),
                            account_name=getattr(pos, "account_name", ""),
                            broker_name=getattr(pos, "broker_name", ""),
                            expiry_date=getattr(pos, "expiry_date", ""),
                        ),
                    )
            return result
        except Exception as e:
            self._log.error(f"查询持仓失败：{e}")
            return []

    def query_orders(self) -> list[Any]:
        """查询当日委托"""
        try:
            return self._trader.query_stock_orders(self._account)
        except Exception as e:
            self._log.error(f"查询委托失败: {e}")
            return []

    def query_orders_async(self) -> int:
        method = getattr(self._trader, "query_stock_orders_async", None)
        if method is None:
            self._log.warning("TODO: xtquant 暂不支持 query_stock_orders_async")
            return -1
        return method(self._account)

    def query_trades(self) -> list[Any]:
        """查询当日成交"""
        try:
            return self._trader.query_stock_trades(self._account)
        except Exception as e:
            self._log.error(f"查询成交失败: {e}")
            return []

    def query_trades_async(self) -> int:
        method = getattr(self._trader, "query_stock_trades_async", None)
        if method is None:
            self._log.warning("TODO: xtquant 暂不支持 query_stock_trades_async")
            return -1
        return method(self._account)
