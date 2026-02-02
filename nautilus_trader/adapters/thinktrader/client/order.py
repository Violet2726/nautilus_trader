from xtquant import xtconstant

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin


class ThinkTraderClientOrderMixin(BaseMixin):
    """处理订单操作"""
    
    def place_order(
        self,
        stock_code: str,
        order_type: int,
        volume: int,
        price: float,
        price_type: int = xtconstant.FIX_PRICE,
        strategy_name: str = "",
        order_remark: str = "",
    ) -> int:
        """
        下单
        
        Parameters
        ----------
        stock_code : str
            合约代码
        order_type : int
            委托类型: STOCK_BUY, STOCK_SELL 等
        volume : int
            委托数量
        price : float
            委托价格
        price_type : int
            报价类型: FIX_PRICE, LATEST_PRICE 等
        strategy_name : str
            策略名称
        order_remark : str
            委托备注 (用于存储 ClientOrderId)
        
        Returns
        -------
        int
            订单 ID，大于 0 表示成功
        """
        order_id = self._trader.order_stock(
            account=self._account,
            stock_code=stock_code,
            order_type=order_type,
            order_volume=volume,
            price_type=price_type,
            price=price,
            strategy_name=strategy_name,
            order_remark=order_remark,
        )
        
        if order_id > 0:
            self._log.debug(
                f"下单成功: {stock_code}, order_id={order_id}, "
                f"type={order_type}, volume={volume}, price={price}"
            )
        else:
            self._log.warning(f"下单失败: {stock_code}, 返回值={order_id}")
        
        return order_id
    
    def cancel_order(self, order_id: int) -> int:
        """撤单 (通过订单 ID)"""
        result = self._trader.cancel_order_stock(
            account=self._account,
            order_id=order_id,
        )
        self._log.debug(f"撤单请求: order_id={order_id}, result={result}")
        return result
    
    def cancel_order_by_sysid(self, market: int, order_sysid: str) -> int:
        """
        撤单 (通过柜台编号)
        
        注意: 根据文档，此接口需要传入 market 参数
        - market: 交易市场，xtconstant.SH_MARKET 或 xtconstant.SZ_MARKET
        - order_sysid: 券商柜台的合同编号
        """
        result = self._trader.cancel_order_stock_sysid(
            account=self._account,
            market=market,
            order_sysid=order_sysid,
        )
        self._log.debug(f"撤单请求: market={market}, order_sysid={order_sysid}, result={result}")
        return result
    
    def place_order_async(
        self,
        stock_code: str,
        order_type: int,
        volume: int,
        price: float,
        price_type: int = xtconstant.FIX_PRICE,
        strategy_name: str = "",
        order_remark: str = "",
    ) -> int:
        """
        异步下单
        
        返回下单请求序号 seq，成功时 seq > 0，失败返回 -1。
        异步下单后会收到 on_order_stock_async_response 回调。
        """
        seq = self._trader.order_stock_async(
            account=self._account,
            stock_code=stock_code,
            order_type=order_type,
            order_volume=volume,
            price_type=price_type,
            price=price,
            strategy_name=strategy_name,
            order_remark=order_remark,
        )
        
        if seq > 0:
            self._log.debug(
                f"异步下单: {stock_code}, seq={seq}, "
                f"type={order_type}, volume={volume}, price={price}"
            )
        else:
            self._log.warning(f"异步下单失败: {stock_code}, 返回值={seq}")
        
        return seq
    
    def cancel_order_async(self, order_id: int) -> int:
        """异步撤单 (通过订单 ID)"""
        seq = self._trader.cancel_order_stock_async(
            account=self._account,
            order_id=order_id,
        )
        self._log.debug(f"异步撤单请求: order_id={order_id}, seq={seq}")
        return seq
