import asyncio
import functools
from typing import Any

import pandas as pd
from xtquant import xtdata

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin
from nautilus_trader.adapters.thinktrader.client.common import Subscription
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.identifiers import InstrumentId


class ThinkTraderClientMarketDataMixin(BaseMixin):
    """
    为 ThinkTrader (XtQuant) 处理市场数据请求、订阅和数据处理。
    """

    async def set_market_data_type(self, market_data_type: Any) -> None:
        """
        设置市场数据类型。
        
        TODO: XtQuant 主要提供实时和本地数据，尚无直接对应的 MarketDataTypeEnum。
        """
        pass

    async def _subscribe(
        self,
        name: str | tuple,
        subscription_method: functools.partial | Any,
        cancellation_method: Any,
        *args: Any,
        **kwargs: Any,
    ) -> Subscription:
        """
        管理市场数据的订阅过程。
        """
        if not (subscription := self._subscriptions.get(name=name)):
            req_id = self._next_req_id()
            
            # XtQuant subscribe_quote returns a sequence number (seq) which is our req_id
            # However, Nautilus expects us to manage req_id, but XtQuant generates its own.
            # We will use xtdata's seq as req_id.
            
            handle_func = functools.partial(subscription_method, *args, **kwargs)
            
            # Actually call xtdata subscription
            seq = handle_func()
            if seq <= 0:
                raise RuntimeError(f"XtQuant 订阅失败: {name}")
                
            subscription = self._subscriptions.add(
                req_id=seq,
                name=name,
                handle=handle_func,
                cancel=functools.partial(cancellation_method, seq),
            )
            self._log.info(f"已创建并注册新的 Subscription: {subscription}")
        else:
            self._log.info(f"复用现有的 Subscription: {subscription}")
            
        return subscription

    async def _unsubscribe(
        self,
        name: str | tuple,
        cancellation_method: Any,
    ) -> None:
        """
        管理市场数据的取消订阅过程。
        """
        if subscription := self._subscriptions.get(name=name):
            req_id = subscription.req_id
            self._subscriptions.remove(req_id)
            cancellation_method(req_id)
            self._log.debug(f"已取消订阅 {subscription}")
        else:
            self._log.debug(f"订阅 {name} 不存在")

    async def subscribe_ticks(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
    ) -> None:
        """
        订阅逐笔行情数据。
        """
        name = (str(instrument_id), "tick")
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period="tick",
            count=0,
            callback=self._on_quote_data,
        )

    async def unsubscribe_ticks(self, instrument_id: InstrumentId) -> None:
        """
        取消订阅逐笔行情数据。
        """
        name = (str(instrument_id), "tick")
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def subscribe_market_data(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
    ) -> None:
        """
        使用普通行情请求订阅数据（在 ThinkTrader 中主要也是 subscribe_quote）。
        """
        name = (str(instrument_id), "market_data")
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period="tick",
            count=0,
            callback=self._on_quote_data,
        )

    async def unsubscribe_market_data(self, instrument_id: InstrumentId) -> None:
        """
        取消订阅市场数据。
        """
        name = (str(instrument_id), "market_data")
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def subscribe_order_book(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
    ) -> None:
        """
        订阅订单簿（Level 2）数据。
        """
        name = (str(instrument_id), "order_book")
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period="l2quote", # Level 2 快照
            count=0,
            callback=self._on_quote_data,
        )

    async def unsubscribe_order_book(self, instrument_id: InstrumentId) -> None:
        """
        取消订阅订单簿数据。
        """
        name = (str(instrument_id), "order_book")
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def subscribe_realtime_bars(
        self,
        bar_type: BarType,
        stock_code: str,
    ) -> None:
        """
        订阅实时 K 线数据。
        """
        from nautilus_trader.adapters.thinktrader.parsing.data import bar_spec_to_period
        period = bar_spec_to_period(bar_type.spec)
        name = str(bar_type)
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period=period,
            count=0,
            callback=self._on_quote_data,
        )

    async def unsubscribe_realtime_bars(self, bar_type: BarType) -> None:
        """
        取消订阅实时 K 线数据。
        """
        name = str(bar_type)
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def subscribe_historical_bars(
        self,
        bar_type: BarType,
        stock_code: str,
        start_ns: int,
    ) -> None:
        """
        订阅包含历史数据的 K 线。
        """
        from nautilus_trader.adapters.thinktrader.parsing.data import bar_spec_to_period
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
        period = bar_spec_to_period(bar_type.spec)
        start_time = ns_to_xt_time(start_ns)
        name = str(bar_type)
        
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period=period,
            start_time=start_time,
            count=-1,
            callback=self._on_quote_data,
        )

    async def unsubscribe_historical_bars(self, bar_type: BarType) -> None:
        """
        取消订阅历史 K 线。
        """
        name = str(bar_type)
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def get_historical_bars(
        self,
        bar_type: BarType,
        stock_code: str,
        start_ns: int,
        end_ns: int,
        timeout: int = 60,
    ) -> list[Bar]:
        """
        请求并检索指定 K 线类型的历史 K 线数据。
        """
        from nautilus_trader.adapters.thinktrader.parsing.data import bar_spec_to_period
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
        
        period = bar_spec_to_period(bar_type.spec)
        start_time = ns_to_xt_time(start_ns)
        end_time = ns_to_xt_time(end_ns)
        name = (bar_type, start_time, end_time)

        if not (request := self._requests.get(name=name)):
            req_id = self._next_req_id()
            
            # 由于 get_market_data 是同步的，我们在这里简单封装为非阻塞
            # 或者将其视为一个立即完成的请求
            # 在实际生产中，可能需要先调用 download_history_data2 并等待其回调
            
            def handle():
                data = xtdata.get_market_data(
                    field_list=[],
                    stock_list=[stock_code],
                    period=period,
                    start_time=start_time,
                    end_time=end_time,
                    count=-1,
                    dividend_type='none',
                    fill_data=True,
                )
                request.future.set_result(data)

            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=handle,
                cancel=lambda: None,
            )
            
            self._log.debug(f"get_historical_bars: {request.req_id=}, {stock_code=}")
            request.handle()

            return await self._await_request(request, timeout, default_value=[])
        else:
            self._log.info(f"请求已存在于 {request}")
            return []

    async def get_historical_ticks(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
        start_ns: int,
        end_ns: int,
        timeout: int = 60,
    ) -> list[QuoteTick | TradeTick]:
        """
        请求并检索历史逐笔行情数据。
        """
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
        start_time = ns_to_xt_time(start_ns)
        end_time = ns_to_xt_time(end_ns)
        name = (instrument_id, "tick", start_time, end_time)

        if not (request := self._requests.get(name=name)):
            req_id = self._next_req_id()
            
            def handle():
                data = xtdata.get_market_data(
                    field_list=[],
                    stock_list=[stock_code],
                    period="tick",
                    start_time=start_time,
                    end_time=end_time,
                    count=-1,
                )
                request.future.set_result(data)

            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=handle,
                cancel=lambda: None,
            )
            
            request.handle()
            return await self._await_request(request, timeout, default_value=[])
        else:
            self._log.info(f"请求 {request} 已存在")
            return []

    # =========================================================================
    # 以下为同步 IB 实现的功能函数占位或 TODO
    # =========================================================================

    async def process_market_data_type(self, *, req_id: int, market_data_type: int) -> None:
        """TODO: 处理市场数据类型变更"""
        pass

    async def process_tick_by_tick_bid_ask(self, **kwargs: Any) -> None:
        """TODO: 处理逐笔买卖报价 (已集成在 _on_quote_data)"""
        pass

    async def process_tick_by_tick_all_last(self, **kwargs: Any) -> None:
        """TODO: 处理逐笔成交 (已集成在 _on_quote_data)"""
        pass

    async def process_tick_price(self, **kwargs: Any) -> None:
        """TODO: 处理行情价格更新"""
        pass

    async def process_tick_size(self, **kwargs: Any) -> None:
        """TODO: 处理行情大小更新"""
        pass

    async def process_order_book_update(self, **kwargs: Any) -> None:
        """TODO: 处理订单簿更新 (已集成在 _on_quote_data)"""
        pass

    async def process_realtime_bar(self, **kwargs: Any) -> None:
        """TODO: 处理实时 K 线更新 (已集成在 _on_quote_data)"""
        pass

    async def process_historical_bars(self, **kwargs: Any) -> None:
        """TODO: 处理历史 K 线响应"""
        pass

    async def process_historical_ticks(self, **kwargs: Any) -> None:
        """TODO: 处理历史逐笔行情响应"""
        pass

    def _on_quote_data(self, datas: dict) -> None:
        # (已由上一步实现)
        ...
        """
        处理单股行情回调。
        callback datas 格式: { stock_code : [data1, data2, ...] }
        """
        for stock_code, data_list in datas.items():
            for data in data_list:
                self._loop.call_soon_threadsafe(
                    self._handle_quote_data,
                    stock_code,
                    data,
                )

    def _on_whole_quote_data(self, datas: dict) -> None:
        """
        处理全推行情回调。
        callback datas 格式: { stock1 : data1, stock2 : data2, ... }
        """
        for stock_code, data in datas.items():
            self._loop.call_soon_threadsafe(
                self._handle_quote_data,
                stock_code,
                data,
            )

    def subscribe_whole_quote(
        self,
        code_list: list[str],
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
