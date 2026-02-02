from nautilus_trader.live.execution_client import LiveExecutionClient
from nautilus_trader.model.identifiers import AccountId, ClientOrderId, VenueOrderId
from nautilus_trader.model.enums import OrderSide, OrderStatus, LiquiditySide
from nautilus_trader.model.objects import Currency

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.adapters.thinktrader.parsing.execution import (
    ORDER_STATUS_MAP,
    NAUTILUS_SIDE_TO_XT,
)
from nautilus_trader.adapters.thinktrader.parsing.instruments import (
    stock_code_to_instrument_id,
    instrument_id_to_stock_code,
)


class ThinkTraderExecutionClient(LiveExecutionClient):
    """ThinkTrader 执行客户端"""
    
    def __init__(
        self,
        loop,
        client: ThinkTraderClient,
        msgbus,
        cache,
        clock,
        instrument_provider,
        config,
    ):
        super().__init__(
            loop=loop,
            client_id=ClientId("THINKTRADER"),
            venue=TT_VENUE,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
        )
        self._client = client
        self._instrument_provider = instrument_provider
        self._config = config
        
        # 订单 ID 映射
        self._client_order_id_to_order_id: dict[ClientOrderId, int] = {}
        self._order_id_to_client_order_id: dict[int, ClientOrderId] = {}
        
        # 注册回调
        self._client.register_event_handler("order_update", self._on_order_update)
        self._client.register_event_handler("trade", self._on_trade)
        self._client.register_event_handler("order_rejected", self._on_order_rejected)
    
    async def _connect(self) -> None:
        await self._client._connect()
    
    async def _disconnect(self) -> None:
        await self._client._disconnect()
    
    async def _submit_order(self, command) -> None:
        """提交订单"""
        order = command.order
        stock_code = instrument_id_to_stock_code(order.instrument_id)
        
        # 目前仅支持股票买卖映射
        # 如需支持期货，需扩展逻辑处理 open/close 标志
        order_type = NAUTILUS_SIDE_TO_XT.get(order.side)
        if order_type is None:
             self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason=f"不支持的订单方向: {order.side}",
                ts_event=self._clock.timestamp_ns(),
            )
             return

        price = float(order.price) if order.price else 0
        
        order_id = self._client.place_order(
            stock_code=stock_code,
            order_type=order_type,
            volume=int(order.quantity),
            price=price,
            order_remark=str(order.client_order_id),
        )
        
        if order_id > 0:
            self._client_order_id_to_order_id[order.client_order_id] = order_id
            self._order_id_to_client_order_id[order_id] = order.client_order_id
            self.generate_order_submitted(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                ts_event=self._clock.timestamp_ns(),
            )
        else:
            self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason=f"下单失败，返回值: {order_id}",
                ts_event=self._clock.timestamp_ns(),
            )
    
    async def _cancel_order(self, command) -> None:
        """取消订单"""
        order_id = self._client_order_id_to_order_id.get(command.client_order_id)
        if order_id:
            self._client.cancel_order(order_id)
        else:
            self._log.warning(f"未找到订单映射: {command.client_order_id}")
    
    def _on_order_update(self, order) -> None:
        """处理委托更新"""
        # order 是 XtOrder 对象
        order_id = order.order_id
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        
        if not client_order_id:
            # 可能是外部订单，尝试从 order_remark 恢复
            if order.order_remark:
                client_order_id = ClientOrderId(order.order_remark)
        
        if not client_order_id:
            self._log.warning(f"未知订单: order_id={order_id}")
            return
        
        status = ORDER_STATUS_MAP.get(order.order_status)
        cached_order = self._cache.order(client_order_id)
        
        if cached_order is None:
            self._log.warning(f"缓存中未找到订单: {client_order_id}")
            return
        
        ts_event = self._clock.timestamp_ns()
        
        # 根据状态生成相应事件
        if status == OrderStatus.ACCEPTED:
            if cached_order.status == OrderStatus.SUBMITTED:
                self.generate_order_accepted(
                    strategy_id=cached_order.strategy_id,
                    instrument_id=cached_order.instrument_id,
                    client_order_id=client_order_id,
                    venue_order_id=VenueOrderId(order.order_sysid),
                    ts_event=ts_event,
                )
        elif status == OrderStatus.CANCELED:
            self.generate_order_canceled(
                strategy_id=cached_order.strategy_id,
                instrument_id=cached_order.instrument_id,
                client_order_id=client_order_id,
                venue_order_id=VenueOrderId(order.order_sysid),
                ts_event=ts_event,
            )
        elif status == OrderStatus.REJECTED:
            # Avoid duplicate rejection events if already in terminal state
            if cached_order.status not in (OrderStatus.REJECTED, OrderStatus.CANCELED, OrderStatus.FILLED):
                self.generate_order_rejected(
                    strategy_id=cached_order.strategy_id,
                    instrument_id=cached_order.instrument_id,
                    client_order_id=client_order_id,
                    reason=order.status_msg or "废单",
                    ts_event=ts_event,
                )
    
    def _on_trade(self, trade) -> None:
        """处理成交回报"""
        from nautilus_trader.model.identifiers import TradeId
        from nautilus_trader.model.objects import Price, Quantity, Money
        
        order_id = trade.order_id
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        
        if not client_order_id:
            # 尝试从 order_remark 恢复
            if trade.order_remark:
                client_order_id = ClientOrderId(trade.order_remark)
            else:
                self._log.warning(f"未知成交: order_id={order_id}")
                return
        
        cached_order = self._cache.order(client_order_id)
        if cached_order is None:
            self._log.warning(f"缓存中未找到订单: {client_order_id}")
            return
        
        instrument_id = stock_code_to_instrument_id(trade.stock_code)
        instrument = self._cache.instrument(instrument_id)
        
        ts_event = int(trade.traded_time) * 1_000_000_000 if trade.traded_time else self._clock.timestamp_ns()
        
        self.generate_order_filled(
            strategy_id=cached_order.strategy_id,
            instrument_id=cached_order.instrument_id,
            client_order_id=client_order_id,
            venue_order_id=VenueOrderId(trade.order_sysid),
            trade_id=TradeId(trade.traded_id),
            order_side=cached_order.side,
            order_type=cached_order.type, # Fix: use cached_order.type not order_type property if ambiguous
            last_qty=Quantity.from_int(trade.traded_volume),
            last_px=Price.from_str(f"{trade.traded_price:.4f}"),
            quote_currency=instrument.quote_currency if instrument else Currency.from_str("CNY"),
            commission=Money(0, Currency.from_str("CNY")),  # 需要额外查询
            liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
            ts_event=ts_event,
        )
    
    def _on_order_rejected(self, order_id: int, reason: str) -> None:
        """处理订单拒绝"""
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        if not client_order_id:
            self._log.warning(f"未知的拒绝订单: order_id={order_id}")
            return
        
        cached_order = self._cache.order(client_order_id)
        if cached_order:
             # Avoid duplicate rejection events
            if cached_order.status not in (OrderStatus.REJECTED, OrderStatus.CANCELED, OrderStatus.FILLED):
                self.generate_order_rejected(
                    strategy_id=cached_order.strategy_id,
                    instrument_id=cached_order.instrument_id,
                    client_order_id=client_order_id,
                    reason=reason,
                    ts_event=self._clock.timestamp_ns(),
                )
