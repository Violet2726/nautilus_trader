import asyncio
import functools
from typing import Any

import pandas as pd
from xtquant import xtdata

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin
from nautilus_trader.adapters.thinktrader.client.common import Subscription
from nautilus_trader.adapters.thinktrader.parsing.data import bar_spec_to_period
from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
from nautilus_trader.adapters.thinktrader.parsing.data import parse_kline_to_bar
from nautilus_trader.adapters.thinktrader.parsing.data import parse_l2_order_to_delta
from nautilus_trader.adapters.thinktrader.parsing.data import parse_l2_quote_to_order_book_deltas
from nautilus_trader.adapters.thinktrader.parsing.data import parse_l2_transaction_to_trade_tick
from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_quote_tick
from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_trade_tick
from nautilus_trader.core.data import Data
from nautilus_trader.model.data import BarType
from nautilus_trader.model.identifiers import InstrumentId


class ThinkTraderClientMarketDataMixin(BaseMixin):
    """
    为 ThinkTrader (XtQuant) 处理市场数据请求、订阅和数据处理。

    此 Mixin 旨在与系统的标准市场数据接口保持功能对齐。
    """

    async def set_market_data_type(self, market_data_type: Any) -> None:
        """
        设置数据订阅的市场数据类型。

        TODO: XtQuant 主要提供实时和本地数据, 尚无直接对应的 MarketDataTypeEnum。
        """

    def configure_xtdata_data_dir(self, data_dir: str) -> None:
        xtdata.data_dir = data_dir

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
        subscription = self._subscriptions.get(name=name)
        if subscription is None:
            handle_func = functools.partial(subscription_method, *args, **kwargs)
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
        订阅指定工具的逐笔行情 (tick) 数据。
        """
        name = (str(instrument_id), "tick")
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period="tick",
            count=0,
            callback=functools.partial(self._on_quote_data, name=name),
        )

    async def unsubscribe_ticks(self, instrument_id: InstrumentId) -> None:
        """
        取消订阅指定工具的逐笔行情数据。
        """
        name = (str(instrument_id), "tick")
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def subscribe_market_data(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
    ) -> None:
        """
        订阅指定工具的市场数据。
        """
        name = (str(instrument_id), "market_data")
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period="tick",
            count=0,
            callback=functools.partial(self._on_quote_data, name=name),
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
        订阅指定工具的订单簿数据。
        """
        name = (str(instrument_id), "order_book")
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period="l2quote",  # Level 2 快照
            count=0,
            callback=functools.partial(self._on_quote_data, name=name),
        )

    async def unsubscribe_order_book(self, instrument_id: InstrumentId) -> None:
        """
        取消订阅指定工具的订单簿数据。
        """
        name = (str(instrument_id), "order_book")
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def subscribe_realtime_bars(
        self,
        bar_type: BarType,
        stock_code: str,
    ) -> None:
        """
        订阅指定 K 线类型的实时 K 线数据。
        """
        period = bar_spec_to_period(bar_type.spec)
        name = str(bar_type)
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period=period,
            count=0,
            callback=functools.partial(self._on_quote_data, name=name),
        )

    async def unsubscribe_realtime_bars(self, bar_type: BarType) -> None:
        """
        取消订阅指定 K 线类型的实时 K 线数据。
        """
        name = str(bar_type)
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    async def subscribe_realtime_bars_with_dividend(
        self,
        bar_type: BarType,
        stock_code: str,
        dividend_type: str | None = None,
    ) -> None:
        """
        使用带除权参数的接口订阅指定 K 线类型的实时 K 线数据。
        """
        period = bar_spec_to_period(bar_type.spec)
        name = str(bar_type)
        await self._subscribe(
            name,
            xtdata.subscribe_quote2,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period=period,
            count=0,
            dividend_type=dividend_type,
            callback=functools.partial(self._on_quote_data, name=name),
        )

    async def subscribe_historical_bars(
        self,
        bar_type: BarType,
        stock_code: str,
        start_ns: int,
    ) -> None:
        """
        订阅指定 K 线类型的历史 K 线数据。
        """
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
            callback=functools.partial(self._on_quote_data, name=name),
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
    ) -> Any:
        """
        请求并检索指定 K 线类型的历史 K 线数据。
        """
        period = bar_spec_to_period(bar_type.spec)
        start_time = ns_to_xt_time(start_ns)
        end_time = ns_to_xt_time(end_ns)
        name = (bar_type, start_time, end_time)

        if self._requests.get(name=name) is None:
            req_id = self._next_req_id()

            def handle():
                data = xtdata.get_market_data(
                    field_list=[],
                    stock_list=[stock_code],
                    period=period,
                    start_time=start_time,
                    end_time=end_time,
                    count=-1,
                    dividend_type="none",
                    fill_data=True,
                )
                req = self._requests.get(req_id=req_id)
                assert req is not None
                req.future.set_result(data)

            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=handle,
                cancel=lambda: None,
            )

            self._log.debug(f"get_historical_bars: {request.req_id=}, {stock_code=}")
            request.handle()

            return await self._await_request(request, timeout, default_value={})
        else:
            existing = self._requests.get(name=name)
            if existing is None:
                return {}
            self._log.info(f"请求已存在于 {existing}")
            return await self._await_request(existing, timeout, default_value={})

    async def get_historical_ticks(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
        start_ns: int,
        end_ns: int,
        timeout: int = 60,
    ) -> Any:
        """
        请求并检索历史逐笔行情数据。
        """
        start_time = ns_to_xt_time(start_ns)
        end_time = ns_to_xt_time(end_ns)
        name = (instrument_id, "tick", start_time, end_time)

        if self._requests.get(name=name) is None:
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
                req = self._requests.get(req_id=req_id)
                assert req is not None
                req.future.set_result(data)

            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=handle,
                cancel=lambda: None,
            )

            request.handle()
            return await self._await_request(request, timeout, default_value={})
        else:
            existing = self._requests.get(name=name)
            if existing is None:
                return {}
            self._log.info(f"请求 {name} 已存在")
            return await self._await_request(existing, timeout, default_value={})

    async def req_fundamental_data(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
        report_type: str = "report_time",
        timeout: int = 60,
    ) -> dict[str, Any]:
        """
        请求特定合约的基本面/财务数据。
        """
        name = (instrument_id, "fundamental", report_type)
        if self._requests.get(name=name) is None:
            req_id = self._next_req_id()

            def handle():
                # 获取财务数据 (Balance, Income, CashFlow 等)
                financial = xtdata.get_financial_data(
                    stock_list=[stock_code],
                    report_type=report_type,
                )
                # 获取合约详细静态信息
                detail = xtdata.get_instrument_detail(stock_code)

                req = self._requests.get(req_id=req_id)
                assert req is not None
                req.future.set_result({"financial": financial, "detail": detail})

            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=handle,
                cancel=lambda: None,
            )

            request.handle()
            return await self._await_request(request, timeout, default_value={})
        else:
            self._log.info(f"请求 {name} 已存在")
            return {}

    async def subscribe_tick_by_tick(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
        tick_type: str = "AllLast",  # "AllLast" (成交) 或 "BidAsk" (报单/撤单)
    ) -> None:
        """
        订阅逐笔行情数据 (Level 2)。
        """
        if tick_type == "AllLast":
            period = "l2transaction"
        else:
            period = "l2order"

        name = (str(instrument_id), period)
        await self._subscribe(
            name,
            xtdata.subscribe_quote,
            xtdata.unsubscribe_quote,
            stock_code=stock_code,
            period=period,
            count=0,
            callback=functools.partial(self._on_quote_data, name=name),
        )

    async def unsubscribe_tick_by_tick(
        self,
        instrument_id: InstrumentId,
        stock_code: str,
        tick_type: str = "AllLast",
    ) -> None:
        if tick_type == "AllLast":
            period = "l2transaction"
        else:
            period = "l2order"

        name = (str(instrument_id), period)
        await self._unsubscribe(name, xtdata.unsubscribe_quote)

    def _try_get_price_from_full_tick(self, stock_code: str) -> float:
        data = xtdata.get_full_tick([stock_code])
        if not isinstance(data, dict):
            return 0.0

        tick = data.get(stock_code)
        if not isinstance(tick, dict):
            return 0.0

        try:
            price = float(tick.get("lastPrice") or 0.0)
        except Exception:
            return 0.0

        return price if price > 0 else 0.0

    async def _await_price_from_tick_subscription(self, stock_code: str, timeout: float) -> float:
        future: asyncio.Future[float] = self._loop.create_future()

        def callback(datas: Any) -> None:
            if future.done():
                return
            if not isinstance(datas, dict):
                return

            items = datas.get(stock_code)
            if not isinstance(items, list) or not items:
                return

            first = items[0]
            if not isinstance(first, dict):
                return

            try:
                price = float(first.get("lastPrice") or 0.0)
            except Exception:
                return

            if price <= 0:
                return

            self._loop.call_soon_threadsafe(future.set_result, price)

        seq = xtdata.subscribe_quote(
            stock_code=stock_code,
            period="tick",
            count=0,
            callback=callback,
        )
        if not isinstance(seq, int) or seq <= 0:
            return 0.0

        try:
            return await asyncio.wait_for(future, timeout=timeout)
        except TimeoutError:
            return 0.0
        finally:
            xtdata.unsubscribe_quote(seq)

    async def get_price(self, instrument_id: InstrumentId, stock_code: str) -> float:
        """
        请求特定合约的最新价格。
        """
        price = self._try_get_price_from_full_tick(stock_code)
        if price > 0:
            return price

        return await self._await_price_from_tick_subscription(stock_code=stock_code, timeout=3.0)

    def get_market_data(
        self,
        field_list: list[str],
        stock_list: list[str],
        period: str = "1d",
        start_time: str = "",
        end_time: str = "",
        count: int = -1,
        dividend_type: str = "none",
        fill_data: bool = True,
    ) -> dict[str, pd.DataFrame]:
        """
        批量获取市场数据 (直接封装 xtdata.get_market_data)。
        """
        try:
            return xtdata.get_market_data(
                field_list=field_list,
                stock_list=stock_list,
                period=period,
                start_time=start_time,
                end_time=end_time,
                count=count,
                dividend_type=dividend_type,
                fill_data=fill_data,
            )
        except Exception as e:
            self._log.error(f"批量获取市场数据失败: {e}")
            return {}

    def get_full_tick(self, stock_list: list[str]) -> dict[str, dict]:
        """
        获取全推行情快照 (直接封装 xtdata.get_full_tick)。
        """
        try:
            return xtdata.get_full_tick(stock_list)
        except Exception as e:
            self._log.error(f"获取行情快照失败: {e}")
            return {}

    # =========================================================================
    # 内部辅助函数
    # =========================================================================

    async def _handle_data(self, data: Data) -> None:
        """
        处理并向适当的目的地转发已处理的数据。
        """
        # 基类 BaseMixin 中应定义此属性, 通常在 Client 中初始化
        self._msgbus.send(endpoint="DataEngine.process", msg=data)

    # =========================================================================
    # ThinkTrader 特有逻辑 (XtQuant API 特点)
    # =========================================================================

    def _on_quote_data(self, datas: dict, name: str | tuple | None = None) -> None:
        """
        处理单股行情回调。
        callback datas 格式: { stock_code : [data1, data2, ...] }
        """
        for stock_code, data_list in datas.items():
            for data in data_list:
                self._loop.call_soon_threadsafe(
                    self._handle_quote_data,
                    name,
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
                None,
                stock_code,
                data,
            )

    def _forward_data(self, data: Any) -> None:
        """转发解析后的数据到注册的处理器"""
        if handler := self._event_handlers.get("data"):
            handler(data)

    def _handle_tuple_quote_data(
        self,
        instrument_id: InstrumentId,
        data_type: str,
        data: dict,
        ts_init: int,
    ) -> None:
        if data_type == "tick":
            quote = parse_tick_to_quote_tick(instrument_id, data, ts_init)
            self._forward_data(quote)
            trade = parse_tick_to_trade_tick(instrument_id, data, ts_init)
            self._forward_data(trade)
            return

        if data_type == "market_data":
            quote = parse_tick_to_quote_tick(instrument_id, data, ts_init)
            self._forward_data(quote)
            return

        if data_type == "order_book":
            deltas = parse_l2_quote_to_order_book_deltas(instrument_id, data, ts_init)
            for delta in deltas:
                self._forward_data(delta)
            return

        if data_type == "l2order":
            delta = parse_l2_order_to_delta(instrument_id, data, ts_init)
            self._forward_data(delta)
            return

        if data_type == "l2transaction":
            trade = parse_l2_transaction_to_trade_tick(instrument_id, data, ts_init)
            self._forward_data(trade)

    def _handle_quote_data(
        self,
        name: str | tuple | None,
        stock_code: str,
        data: dict,
    ) -> None:
        """
        处理原始 XtQuant 数据报文并转发。
        """
        ts_init = self._clock.timestamp_ns()

        # 尝试从订阅名中推断数据类型
        if isinstance(name, tuple):
            instrument_id = InstrumentId.from_str(name[0])
            data_type = name[1]
            self._handle_tuple_quote_data(instrument_id, data_type, data, ts_init)
            return

        if isinstance(name, str):
            try:
                bar_type = BarType.from_str(name)
            except Exception:
                self._log.error(f"无法解析数据报文, 订阅名为: {name}")
                return
            bar = parse_kline_to_bar(bar_type.instrument_id, bar_type, data, ts_init)
            self._forward_data(bar)
            return

        instrument_id = self._cache.instrument_id_for_symbol(stock_code)
        if instrument_id:
            quote = parse_tick_to_quote_tick(instrument_id, data, ts_init)
            self._forward_data(quote)

    def subscribe_whole_quote(
        self,
        code_list: list[str],
        callback: Any | None = None,
    ) -> int:
        """
        订阅全推行情。
        """
        seq = xtdata.subscribe_whole_quote(
            code_list=code_list,
            callback=callback or self._on_whole_quote_data,
        )
        if seq > 0:
            self._log.debug(f"已订阅全推行情, 共 {len(code_list)} 个品种")
        return seq

    async def download_history_data(
        self,
        stock_list: list[str],
        period: str,
        start_time: str = "",
        end_time: str = "",
        timeout: float = 60.0,
    ) -> None:
        """
        在请求历史数据前, 确保数据已下载到本地。
        """
        try:
            await asyncio.wait_for(
                asyncio.to_thread(
                    self._download_history_data_sync,
                    stock_list=stock_list,
                    period=period,
                    start_time=start_time,
                    end_time=end_time,
                ),
                timeout=timeout,
            )
        except TimeoutError:
            self._log.debug(
                f"download_history_data2 timeout ({timeout}s): {stock_list=}, {period=}, {start_time=}, {end_time=}",
            )
            raise

    def _download_history_data_sync(
        self,
        *,
        stock_list: list[str],
        period: str,
        start_time: str,
        end_time: str,
    ) -> None:
        if len(stock_list) == 1:
            xtdata.download_history_data(
                stock_code=stock_list[0],
                period=period,
                start_time=start_time,
                end_time=end_time,
                incrementally=None,
            )
            return

        xtdata.download_history_data2(
            stock_list=stock_list,
            period=period,
            start_time=start_time,
            end_time=end_time,
            callback=None,
        )
