from typing import List, Dict, Optional
from nautilus_trader.adapters.thinktrader.client.common import BaseMixin, TTPosition


class ThinkTraderClientAccountMixin(BaseMixin):
    """处理账户和持仓查询"""
    
    def query_asset(self) -> Optional[Dict]:
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
                }
            return None
        except Exception as e:
            self._log.error(f"查询资产失败: {e}")
            return None
    
    def query_positions(self) -> List[TTPosition]:
        """查询持仓"""
        try:
            positions = self._trader.query_stock_positions(self._account)
            result = []
            if positions:
                for pos in positions:
                    result.append(TTPosition(
                        account_id=pos.account_id,
                        stock_code=pos.stock_code,
                        volume=pos.volume,
                        available_volume=pos.can_use_volume,
                        avg_price=pos.open_price,
                        market_value=pos.market_value,
                    ))
            return result
        except Exception as e:
            self._log.error(f"查询持仓失败: {e}")
            return []
    
    def query_orders(self) -> List:
        """查询当日委托"""
        try:
            return self._trader.query_stock_orders(self._account)
        except Exception as e:
            self._log.error(f"查询委托失败: {e}")
            return []
    
    def query_trades(self) -> List:
        """查询当日成交"""
        try:
            return self._trader.query_stock_trades(self._account)
        except Exception as e:
            self._log.error(f"查询成交失败: {e}")
            return []
