from typing import Any


try:
    from xtquant import xtdata
except ModuleNotFoundError:  # pragma: no cover
    class _XtDataStub:
        def get_instrument_detail(self, *args: Any, **kwargs: Any) -> Any:
            raise ModuleNotFoundError("xtquant is required for ThinkTrader contract data")

        def get_instrument_type(self, *args: Any, **kwargs: Any) -> Any:
            raise ModuleNotFoundError("xtquant is required for ThinkTrader contract data")

        def get_stock_list_in_sector(self, *args: Any, **kwargs: Any) -> Any:
            raise ModuleNotFoundError("xtquant is required for ThinkTrader contract data")

        def get_trading_dates(self, *args: Any, **kwargs: Any) -> Any:
            raise ModuleNotFoundError("xtquant is required for ThinkTrader contract data")

    xtdata = _XtDataStub()

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin


class ThinkTraderClientContractMixin(BaseMixin):
    """处理合约查询"""

    def get_instrument_detail(self, stock_code: str) -> dict[str, Any] | None:
        """
        获取合约详情

        Parameters
        ----------
        stock_code : str
            合约代码, 如 '600000.SH'

        Returns
        -------
        dict | None
            合约详情字典, 如果未找到则返回 None
        """
        try:
            detail = xtdata.get_instrument_detail(stock_code)
            if detail:
                self._log.debug(f"获取合约详情: {stock_code}")
                return detail
            else:
                self._log.warning(f"未找到合约: {stock_code}")
                return None
        except Exception as e:
            self._log.error(f"获取合约详情失败: {stock_code}, 错误: {e}")
            return None

    def get_instrument_type(self, stock_code: str) -> dict[str, Any]:
        """获取合约类型"""
        try:
            return xtdata.get_instrument_type(stock_code)
        except Exception as e:
            self._log.error(f"获取合约类型失败: {stock_code}, 错误: {e}")
            return {}

    def get_stock_list(self, sector: str = "沪深A股") -> list[str]:
        """
        获取板块内的合约列表

        Parameters
        ----------
        sector : str
            板块名称, 如 '沪深A股', '上证50' 等

        Returns
        -------
        list[str]
            合约代码列表
        """
        try:
            stocks = xtdata.get_stock_list_in_sector(sector)
            self._log.debug(f"获取 {sector} 合约列表, 共 {len(stocks)} 个")
            return stocks
        except Exception as e:
            self._log.error(f"获取合约列表失败: {sector}, 错误: {e}")
            return []

    def get_trading_dates(
        self,
        market: str = "SH",
        start_date: str = "",
        end_date: str = "",
    ) -> list[str]:
        """获取交易日历"""
        try:
            dates = xtdata.get_trading_dates(market, start_date, end_date)
            return dates
        except Exception as e:
            self._log.error(f"获取交易日历失败: {e}")
            return []
