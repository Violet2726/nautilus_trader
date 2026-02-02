from typing import List, Dict
from xtquant import xtdata

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin


class ThinkTraderClientMarketDataMixin(BaseMixin):
    """处理行情数据订阅"""
    
    def subscribe_quote(
        self,
        stock_code: str,
        period: str = "tick",
        callback = None,
    ) -> int:
        """
        订阅行情
        
        Parameters
        ----------
        stock_code : str
            合约代码
        period : str
            周期: 'tick', '1m', '5m', '1d' 等
        callback : callable
            回调函数
        
        Returns
        -------
        int
            订阅号，大于 0 表示成功
        """
        seq = xtdata.subscribe_quote(
            stock_code=stock_code,
            period=period,
            start_time="",
            end_time="",
            count=0,
            callback=callback or self._on_quote_data,
        )
        if seq > 0:
            self._subscriptions[seq] = {
                "stock_code": stock_code,
                "period": period,
            }
            self._log.debug(f"已订阅 {stock_code} ({period}), seq={seq}")
        else:
            self._log.warning(f"订阅失败: {stock_code} ({period})")
        return seq
    
    def subscribe_whole_quote(
        self,
        code_list: List[str],
        callback = None,
    ) -> int:
        """订阅全推行情"""
        seq = xtdata.subscribe_whole_quote(
            code_list=code_list,
            callback=callback or self._on_whole_quote_data,
        )
        if seq > 0:
            self._log.debug(f"已订阅全推行情，共 {len(code_list)} 个品种")
        return seq
    
    def unsubscribe_quote(self, seq: int) -> None:
        """取消订阅"""
        xtdata.unsubscribe_quote(seq)
        if seq in self._subscriptions:
            info = self._subscriptions.pop(seq)
            self._log.debug(f"已取消订阅: {info}")
    
    def get_market_data(
        self,
        stock_list: List[str],
        period: str = "1d",
        start_time: str = "",
        end_time: str = "",
        count: int = -1,
    ) -> Dict:
        """获取历史行情数据"""
        # xtdata.get_market_data returns dict { field: DataFrame } or { stock: np.ndarray }
        return xtdata.get_market_data(
            field_list=[],
            stock_list=stock_list,
            period=period,
            start_time=start_time,
            end_time=end_time,
            count=count,
            dividend_type='none',
            fill_data=True,
        )
    
    def _on_quote_data(self, datas: dict) -> None:
        """行情回调处理"""
        for stock_code, data_list in datas.items():
            for data in data_list:
                self._loop.call_soon_threadsafe(
                    self._handle_quote_data,
                    stock_code,
                    data,
                )
    
    def _on_whole_quote_data(self, datas: dict) -> None:
        """全推行情回调处理"""
        for stock_code, data in datas.items():
            self._loop.call_soon_threadsafe(
                self._handle_quote_data,
                stock_code,
                data,
            )
