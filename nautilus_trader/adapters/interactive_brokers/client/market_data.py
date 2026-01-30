# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------

import asyncio
import functools
from collections import defaultdict
from collections.abc import Callable
from decimal import Decimal
from inspect import iscoroutinefunction
from typing import Any
from typing import ClassVar
from zoneinfo import ZoneInfo

import pandas as pd
import pytz
from ibapi.common import BarData
from ibapi.common import HistoricalTickLast
from ibapi.common import MarketDataTypeEnum
from ibapi.common import TickAttribBidAsk
from ibapi.common import TickAttribLast

from nautilus_trader.adapters.interactive_brokers.client.common import BaseMixin
from nautilus_trader.adapters.interactive_brokers.client.common import IBKRBookLevel
from nautilus_trader.adapters.interactive_brokers.client.common import Subscription
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.parsing.data import IB_SIDE
from nautilus_trader.adapters.interactive_brokers.parsing.data import MKT_DEPTH_OPERATIONS
from nautilus_trader.adapters.interactive_brokers.parsing.data import bar_spec_to_bar_size
from nautilus_trader.adapters.interactive_brokers.parsing.data import generate_trade_id
from nautilus_trader.adapters.interactive_brokers.parsing.data import timedelta_to_duration_str
from nautilus_trader.adapters.interactive_brokers.parsing.data import what_to_show
from nautilus_trader.adapters.interactive_brokers.parsing.price_conversion import (
    ib_price_to_nautilus_price,
)
from nautilus_trader.core.data import Data
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import BookOrder
from nautilus_trader.model.data import OrderBookDelta
from nautilus_trader.model.data import OrderBookDeltas
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import AggressorSide
from nautilus_trader.model.enums import BookAction
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.identifiers import InstrumentId


# 用于使可能暗示数据问题的异常行情大小无效
MAX_VALID_TICK_SIZE = Decimal("1e12")


class InteractiveBrokersClientMarketDataMixin(BaseMixin):
    """
    为 InteractiveBrokersClient 处理市场数据请求、订阅和数据处理。

    此类处理实时和历史市场数据订阅管理，包括对行情（ticks）、K 线（bars）以及其他
    市场数据类型的订阅和取消订阅。它对接收到的数据进行处理和格式化，以使其与 
    Nautilus Trader 兼容。

    """

    _order_book_depth: ClassVar[dict[int, int]] = {}  # reqId -> 深度
    _order_books_initialized: ClassVar[dict[int, bool]] = {}  # reqId -> 是否已初始化

    # 混合到 InteractiveBrokersClient 时可用的实例变量
    _subscription_tick_data: dict[int, dict[int, Any]]
    _subscription_start_times: dict[int, int]  # reqId -> start_ns (用于 K 线过滤)

    _order_books: ClassVar[dict[int, dict[str, dict[int, IBKRBookLevel]]]] = {}
    """
    Example:
    self._order_books: dict[int, dict[str, dict[int, IBKRBookLevel]]] = {
        100: {
            "bids": {
                0: IBKRBookLevel(price=0, size=Decimal(0), market_maker="NSDQ"),
            },
            "asks": {
                0: IBKRBookLevel(price=0, size=Decimal(0), market_maker="NSDQ"),
            },
        }
    }
    """

    async def set_market_data_type(self, market_data_type: MarketDataTypeEnum) -> None:
        """
        设置数据订阅的市场数据类型。此方法配置用于后续数据请求的市场数据类型（实时、延时等）。

        参数
        ----------
        market_data_type : MarketDataTypeEnum
            要设置的市场数据类型。

        """
        self._log.info(f"将市场数据类型设置为 {MarketDataTypeEnum.toStr(market_data_type)}")
        self._eclient.reqMarketDataType(market_data_type)

    async def _subscribe(
        self,
        name: str | tuple,
        subscription_method: Callable | functools.partial,
        cancellation_method: Callable,
        *args: Any,
        **kwargs: Any,
    ) -> Subscription:
        """
        管理市场数据的订阅和取消订阅过程。此内部方法负责处理对不同市场数据类型（行情、K 线等）
        进行订阅或取消订阅的逻辑。它使用提供的订阅和取消订阅方法来控制数据流。

        参数
        ----------
        name : Any
            订阅的唯一标识符。
        subscription_method : Callable
            订阅市场数据时调用的方法。
        cancellation_method : Callable
            取消订阅市场数据时调用的方法。
        *args
            传递给订阅方法的变长参数列表。
        **kwargs
            传递给订阅方法的变长关键字参数。

        返回
        -------
        Subscription

        """
        if not (subscription := self._subscriptions.get(name=name)):
            self._log.info(
                f"正在为 {name} 创建并注册新的 Subscription 实例",
            )
            req_id = self._next_req_id()

            if subscription_method == self.subscribe_historical_bars:
                handle_func = functools.partial(
                    subscription_method,
                    *args,
                    **kwargs,
                )
            else:
                handle_func = functools.partial(subscription_method, req_id, *args, **kwargs)

                if subscription_method == self._eclient.reqMktDepth:
                    self._order_book_depth[req_id] = args[1]
                    self._order_books_initialized[req_id] = False

            # 添加订阅
            subscription = self._subscriptions.add(
                req_id=req_id,
                name=name,
                handle=handle_func,
                cancel=functools.partial(cancellation_method, req_id),
            )

            # 故意跳过历史请求处理程序的调用
            if subscription_method != self.subscribe_historical_bars:
                if iscoroutinefunction(subscription.handle):
                    await subscription.handle()
                else:
                    subscription.handle()
        else:
            self._log.info(f"复用 {subscription} 的现有 Subscription 实例")

        return subscription

    async def _unsubscribe(
        self,
        name: str | tuple,
        cancellation_method: Callable,
        *args: Any,
        **kwargs: Any,
    ) -> None:
        """
        管理市场数据的取消订阅过程。此内部方法负责处理对不同市场数据类型（行情、K 线等）
        取消订阅的逻辑。它使用提供的取消订阅方法来控制数据流。

        参数
        ----------
        cancellation_method : Callable
            取消订阅市场数据时调用的方法。
        name : Any
            订阅的唯一标识符。
        *args
            传递给订阅方法的变长参数列表。
        **kwargs
            传递给订阅方法的变长关键字参数。

        """
        if subscription := self._subscriptions.get(name=name):
            req_id = subscription.req_id
            self._subscriptions.remove(req_id)
            self._subscription_tick_data.pop(req_id, None)
            cancellation_method(req_id, *args, **kwargs)
            self._log.debug(f"已取消订阅 {subscription}")
        else:
            self._log.debug(f"订阅 {name} 不存在")

    async def subscribe_ticks(
        self,
        instrument_id: InstrumentId,
        contract: IBContract,
        tick_type: str,
        ignore_size: bool,
    ) -> None:
        """
        订阅指定工具的逐笔行情（tick）数据。

        参数
        ----------
        instrument_id : InstrumentId
            要订阅的工具标识符。
        contract : IBContract
            该工具的合约详情。
        tick_type : str
            要订阅的行情数据类型。
        ignore_size : bool
            是否省略仅反映大小变化而不反映价格变化的更新。适用于 Bid_Ask 数据请求。

        """
        name = (str(instrument_id), tick_type)
        await self._subscribe(
            name,
            self._eclient.reqTickByTickData,
            self._eclient.cancelTickByTickData,
            contract,
            tick_type,
            0,
            ignore_size,
        )

    async def unsubscribe_ticks(self, instrument_id: InstrumentId, tick_type: str) -> None:
        """
        取消订阅指定工具的行情数据。

        参数
        ----------
        instrument_id : InstrumentId
            要取消订阅的工具标识符。
        tick_type : str
            要取消订阅的行情数据类型。

        """
        name = (str(instrument_id), tick_type)
        await self._unsubscribe(name, self._eclient.cancelTickByTickData)

    async def subscribe_market_data(
        self,
        instrument_id: InstrumentId,
        contract: IBContract,
        generic_tick_list: str = "",
    ) -> None:
        """
        使用 reqMktData 订阅指定工具的市场数据。此方法用于不支持 reqTickByTickData 
        的 BAG（组合价差）合约。

        参数
        ----------
        instrument_id : InstrumentId
            要订阅的工具标识符。
        contract : IBContract
            该工具的合约详情。
        generic_tick_list : str
            以逗号分隔的通用行情类型请求列表。
            空字符串表示基本买入/卖出（bid/ask）数据。

        """
        name = (str(instrument_id), "market_data")
        await self._subscribe(
            name,
            self._eclient.reqMktData,
            self._eclient.cancelMktData,
            contract,
            generic_tick_list,
            False,  # snapshot
            False,  # regulatory_snapshot
            [],  # mktDataOptions
        )

    async def unsubscribe_market_data(self, instrument_id: InstrumentId) -> None:
        """
        取消订阅指定工具的市场数据。

        参数
        ----------
        instrument_id : InstrumentId
            要取消订阅的工具标识符。

        """
        name = (str(instrument_id), "market_data")
        await self._unsubscribe(name, self._eclient.cancelMktData)

    async def subscribe_order_book(
        self,
        instrument_id: InstrumentId,
        contract: IBContract,
        depth: int,
        is_smart_depth: bool = True,
    ) -> None:
        """
        订阅指定工具的订单簿数据。

        参数
        ----------
        instrument_id : InstrumentId
            要订阅的工具标识符。
        contract : IBContract
            该工具的合约详情。
        depth : int
            订单簿每侧的行数。
        is_smart_depth : bool
            指示这是否为 SMART 深度请求。
            如果 isSmartDepth 为 True（在 API v974+ 中可用），则 marketMaker 
            字段将指明报价来自的交易所；否则指明做市商的 MPID。

        """
        name = (str(instrument_id), "order_book")
        await self._subscribe(
            name,
            self._eclient.reqMktDepth,
            self._eclient.cancelMktDepth,
            contract,
            depth,
            is_smart_depth,
            [],  # IBKR: Internal use only. Leave an empty array.
        )

    async def unsubscribe_order_book(
        self,
        instrument_id: InstrumentId,
        is_smart_depth: bool = True,
    ) -> None:
        """
        取消订阅指定工具的订单簿数据。

        参数
        ----------
        instrument_id : InstrumentId
            要取消订阅的工具标识符。
        is_smart_depth : bool
            指示这是否为 SMART 深度请求。
            如果 isSmartDepth 为 True（在 API v974+ 中可用），则 marketMaker 
            字段将指明报价来自的交易所；否则指明做市商的 MPID。

        """
        name = (str(instrument_id), "order_book")
        await self._unsubscribe(
            name,
            self._eclient.cancelMktDepth,
            is_smart_depth,
        )

    async def subscribe_realtime_bars(
        self,
        bar_type: BarType,
        contract: IBContract,
        use_rth: bool,
    ) -> None:
        """
        订阅指定 K 线类型的实时 K 线数据。

        参数
        ----------
        bar_type : BarType
            要订阅的 K 线类型。
        contract : IBContract
            Interactive Brokers 的该工具合约详情。
        use_rth : bool
            是否仅使用常规交易时段 (RTH)。

        """
        name = str(bar_type)
        await self._subscribe(
            name,
            self._eclient.reqRealTimeBars,
            self._eclient.cancelRealTimeBars,
            contract,
            bar_type.spec.step,
            what_to_show(bar_type),
            use_rth,
            [],
        )

    async def unsubscribe_realtime_bars(self, bar_type: BarType) -> None:
        """
        取消订阅指定 K 线类型的实时 K 线数据。

        参数
        ----------
        bar_type : BarType
            要取消订阅的 K 线类型。

        """
        name = str(bar_type)
        await self._unsubscribe(name, self._eclient.cancelRealTimeBars)

    async def subscribe_historical_bars(
        self,
        bar_type: BarType,
        contract: IBContract,
        use_rth: bool,
        handle_revised_bars: bool,
        params: dict,
    ) -> None:
        """
        订阅指定 K 线类型和合约的历史 K 线数据。允许配置常规交易时段和已修订 K 线（revised bars）的处理。

        参数
        ----------
        bar_type : BarType
            要订阅的 K 线类型。
        contract : IBContract
            Interactive Brokers 的该工具合约详情。
        use_rth : bool
            是否仅使用常规交易时段 (RTH)。
        handle_revised_bars : bool
            是否处理已修订的 K 线。
        params : dict
            可选参数字典。

        """
        name = str(bar_type)
        now = self._clock.timestamp_ns()
        start = params.pop("start_ns", None)

        # 需要请求最少数量的 K 线，以便开始接收 K 线数据
        # 然后我们仅考虑初始化时间戳（ts_init）晚于 start 的 K 线
        if start is not None:
            duration_str = timedelta_to_duration_str(
                max(
                    pd.Timedelta(now - start, "ns"),
                    pd.Timedelta(bar_type.spec.timedelta.total_seconds() * 300, "sec"),
                ),  # 至少下载约 300 根 K 线
            )
        else:
            start = now
            duration_str = timedelta_to_duration_str(
                pd.Timedelta(bar_type.spec.timedelta.total_seconds() * 300, "sec"),
            )  # 下载约 300 根 K 线

        if "first_start_ns" not in params:
            params["first_start_ns"] = start

        subscription = await self._subscribe(
            name,
            self.subscribe_historical_bars,
            self._eclient.cancelHistoricalData,
            bar_type=bar_type,
            contract=contract,
            use_rth=use_rth,
            handle_revised_bars=handle_revised_bars,
            params=params,
        )

        # 为了在断开连接后获取缺失的 K 线
        if (
            self._last_disconnection_ns is not None
            and self._last_disconnection_ns > params["first_start_ns"]
        ):
            start = self._last_disconnection_ns

        # 将开始时间单独存储用于 K 线过滤（不属于重新订阅的处理范围）
        self._subscription_start_times[subscription.req_id] = start

        bar_size_setting: str = bar_spec_to_bar_size(bar_type.spec)
        self._eclient.reqHistoricalData(
            reqId=subscription.req_id,
            contract=contract,
            endDateTime="",
            durationStr=duration_str,
            barSizeSetting=bar_size_setting,
            whatToShow=what_to_show(bar_type),
            useRTH=use_rth,
            formatDate=2,
            keepUpToDate=True,
            chartOptions=[],
        )

    async def unsubscribe_historical_bars(self, bar_type: BarType) -> None:
        """
        取消订阅指定 K 线类型的历史 K 线数据。

        参数
        ----------
        bar_type : BarType
            要取消订阅的 K 线类型。

        """
        name = str(bar_type)

        # 在取消订阅前清理存储的开始时间
        subscription = self._subscriptions.get(name=name)
        if subscription:
            self._subscription_start_times.pop(subscription.req_id, None)

        await self._unsubscribe(name, self._eclient.cancelHistoricalData)

    async def get_historical_bars(
        self,
        bar_type: BarType,
        contract: IBContract,
        use_rth: bool,
        end_date_time: pd.Timestamp,
        duration: str,
        timeout: int = 60,
    ) -> list[Bar]:
        """
        请求并检索指定 K 线类型的历史 K 线数据。

        参数
        ----------
        bar_type : BarType
            请求历史数据的 K 线类型。
        contract : IBContract
            Interactive Brokers 的该工具合约详情。
        use_rth : bool
            是否仅在数据中使用常规交易时段 (RTH)。
        end_date_time : pd.Timestamp
            历史数据请求的结束时间，格式为 pandas Timestamp。
        duration : str
            请求历史数据的持续时间，格式为字符串。
        timeout : int, optional
            等待历史数据响应的最大时间（秒）。

        返回
        -------
        list[Bar]

        """
        # 确保请求的 `end_date_time` 为 UTC，并设置 formatDate=2 以确保返回的日期也是 UTC。
        if end_date_time.tzinfo is None:
            end_date_time = end_date_time.replace(tzinfo=ZoneInfo("UTC"))
        else:
            end_date_time = end_date_time.astimezone(ZoneInfo("UTC"))

        end_date_time_str = (
            end_date_time.strftime("%Y%m%d %H:%M:%S %Z") if contract.secType != "CONTFUT" else ""
        )
        name = (bar_type, end_date_time_str)

        if not (request := self._requests.get(name=name)):
            req_id = self._next_req_id()
            bar_size_setting = bar_spec_to_bar_size(bar_type.spec)
            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=functools.partial(
                    self._eclient.reqHistoricalData,
                    reqId=req_id,
                    contract=contract,
                    endDateTime=end_date_time_str,
                    durationStr=duration,
                    barSizeSetting=bar_size_setting,
                    whatToShow=what_to_show(bar_type),
                    useRTH=use_rth,
                    formatDate=2,
                    keepUpToDate=False,
                    chartOptions=[],
                ),
                cancel=functools.partial(self._eclient.cancelHistoricalData, reqId=req_id),
            )

            if not request:
                return []

            self._log.debug(f"reqHistoricalData: {request.req_id=}, {contract=}")
            request.handle()

            return await self._await_request(request, timeout, default_value=[])
        else:
            self._log.info(f"请求已存在于 {request}")
            return []

    async def get_historical_ticks(
        self,
        instrument_id: InstrumentId,
        contract: IBContract,
        tick_type: str,
        start_date_time: pd.Timestamp | str = "",
        end_date_time: pd.Timestamp | str = "",
        use_rth: bool = True,
        timeout: int = 60,
    ) -> list[QuoteTick | TradeTick] | None:
        """
        请求并检索指定合约和行情类型的历史逐笔行情数据。

        参数
        ----------
        instrument_id : InstrumentId
            请求历史行情的工具标识符。
        contract : IBContract
            Interactive Brokers 的该工具合约详情。
        tick_type : str
            请求的行情数据类型（例如 'BID_ASK', 'TRADES'）。
        start_date_time : pd.Timestamp | str, optional
            历史数据请求的开始时间。可以是 pandas Timestamp 或格式为 'YYYYMMDD HH:MM:SS [TZ]' 的字符串。
        end_date_time : pd.Timestamp | str, optional
            历史数据请求的结束时间。格式与 start_date_time 相同。
        use_rth : bool, optional
            是否仅在数据中使用常规交易时段 (RTH)。
        timeout : int, optional
            等待历史数据响应的最大时间（秒）。

        返回
        -------
        list[QuoteTick | TradeTick] | ``None``

        """
        if isinstance(start_date_time, pd.Timestamp):
            start_date_time = start_date_time.strftime("%Y%m%d %H:%M:%S %Z")

        if isinstance(end_date_time, pd.Timestamp):
            end_date_time = end_date_time.strftime("%Y%m%d %H:%M:%S %Z")

        name = (str(instrument_id), tick_type)

        if not (request := self._requests.get(name=name)):
            req_id = self._next_req_id()
            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=functools.partial(
                    self._eclient.reqHistoricalTicks,
                    reqId=req_id,
                    contract=contract,
                    startDateTime=start_date_time,
                    endDateTime=end_date_time,
                    numberOfTicks=1000,
                    whatToShow=tick_type,
                    useRth=use_rth,
                    ignoreSize=False,
                    miscOptions=[],
                ),
                cancel=functools.partial(self._eclient.cancelHistoricalData, reqId=req_id),
            )

            if not request:
                return None

            request.handle()

            return await self._await_request(request, timeout)
        else:
            self._log.info(f"请求 {request} 已存在")

            return None

    async def process_market_data_type(self, *, req_id: int, market_data_type: int) -> None:
        """
        当 TWS 从实时切换到冻结再切换回实时，以及从延时切换到延时冻结再切换回延时时，
        返回由 EClientSocket::reqMktData 发送的代码的市场数据类型（实时、冻结、延时、延时冻结）。
        """
        if market_data_type == MarketDataTypeEnum.REALTIME:
            self._log.debug(f"市场数据类型为 {MarketDataTypeEnum.toStr(market_data_type)}")
        else:
            self._log.warning(f"市场数据类型为 {MarketDataTypeEnum.toStr(market_data_type)}")

    async def process_tick_by_tick_bid_ask(
        self,
        *,
        req_id: int,
        time: int,
        bid_price: float,
        ask_price: float,
        bid_size: Decimal,
        ask_size: Decimal,
        tick_attrib_bid_ask: TickAttribBidAsk,
    ) -> None:
        """
        返回 "BidAsk" 逐笔实时行情数据。
        """
        if not (subscription := self._subscriptions.get(req_id=req_id)):
            return

        instrument_id = InstrumentId.from_str(subscription.name[0])
        instrument = self._cache.instrument(instrument_id)
        ts_event = pd.Timestamp.fromtimestamp(time, tz=pytz.utc).value

        price_magnifier = (
            self._instrument_provider.get_price_magnifier(instrument_id)
            if self._instrument_provider
            else 1
        )
        converted_bid_price = ib_price_to_nautilus_price(bid_price, price_magnifier)
        converted_ask_price = ib_price_to_nautilus_price(ask_price, price_magnifier)

        quote_tick = QuoteTick(
            instrument_id=instrument_id,
            bid_price=instrument.make_price(converted_bid_price),
            ask_price=instrument.make_price(converted_ask_price),
            bid_size=instrument.make_qty(bid_size),
            ask_size=instrument.make_qty(ask_size),
            ts_event=ts_event,
            ts_init=max(self._clock.timestamp_ns(), ts_event),  # `ts_event` <= `ts_init`
        )

        await self._handle_data(quote_tick)

    async def process_tick_by_tick_all_last(
        self,
        *,
        req_id: int,
        tick_type: int,
        time: int,
        price: float,
        size: Decimal,
        tick_attrib_last: TickAttribLast,
        exchange: str,
        special_conditions: str,
    ) -> None:
        """
        返回 "Last" 或 "AllLast"（成交）逐笔实时行情。
        """
        if not (subscription := self._subscriptions.get(req_id=req_id)):
            return

        # 停牌行情
        if price == 0 and size == 0 and tick_attrib_last.pastLimit:
            return

        instrument_id = InstrumentId.from_str(subscription.name[0])
        instrument = self._cache.instrument(instrument_id)
        ts_event = pd.Timestamp.fromtimestamp(time, tz=pytz.utc).value

        price_magnifier = (
            self._instrument_provider.get_price_magnifier(instrument_id)
            if self._instrument_provider
            else 1
        )
        converted_price = ib_price_to_nautilus_price(price, price_magnifier)

        trade_tick = TradeTick(
            instrument_id=instrument_id,
            price=instrument.make_price(converted_price),
            size=instrument.make_qty(size),
            aggressor_side=AggressorSide.NO_AGGRESSOR,
            trade_id=generate_trade_id(ts_event=ts_event, price=converted_price, size=size),
            ts_event=ts_event,
            ts_init=max(self._clock.timestamp_ns(), ts_event),  # `ts_event` <= `ts_init`
        )

        await self._handle_data(trade_tick)

    async def process_tick_price(
        self,
        *,
        req_id: int,
        tick_type: int,
        price: float,
        attrib: Any,
    ) -> None:
        """
        处理来自 reqMktData 的价差（spread）工具的行情价格数据。
        """
        if not (subscription := self._subscriptions.get(req_id=req_id)):
            return

        # 存储此订阅的价格数据
        if req_id not in self._subscription_tick_data:
            self._subscription_tick_data[req_id] = {}

        # 大多数情况下忽略无效价格（IB 使用 -1.0 表示不可用/无效价格）
        # 但期权价差可能具有负价格，在这种情况下，报价的大小将使该报价无效
        if price == -1.0 and self._subscription_tick_data[req_id].get(tick_type, 0.0) > 0.0:
            self._log.warning(
                f"忽略无效的行情价格：{price}，针对 req_id={req_id}，tick_type={tick_type}",
            )
            return

        # IB 行情类型：0=BID_SIZE, 1=BID_PRICE, 2=ASK_PRICE, 3=ASK_SIZE
        self._subscription_tick_data[req_id][tick_type] = price

        # 检查是否同时拥有买入和卖出价格以创建报价行情
        await self._try_create_quote_tick_from_market_data(subscription, req_id)

    async def process_tick_size(
        self,
        *,
        req_id: int,
        tick_type: int,
        size: Decimal,
    ) -> None:
        """
        处理来自 reqMktData 的价差工具的行情大小数据。
        """
        if not (subscription := self._subscriptions.get(req_id=req_id)):
            return

        # 跳过无效的大小（负值或极大值）
        # IB 可能会在价格无效时发送无效的大小
        if size < 0 or size > MAX_VALID_TICK_SIZE:
            self._log.warning(
                f"忽略无效的行情大小：{size}，针对 req_id={req_id}, tick_type={tick_type}",
            )
            return

        # 存储此订阅的大小数据
        if req_id not in self._subscription_tick_data:
            self._subscription_tick_data[req_id] = {}

        # IB 行情类型：0=BID_SIZE, 1=BID_PRICE, 2=ASK_PRICE, 3=ASK_SIZE
        self._subscription_tick_data[req_id][tick_type] = int(size)

        # 检查是否同时拥有买入和卖出数据以创建报价行情
        await self._try_create_quote_tick_from_market_data(subscription, req_id)

    async def _try_create_quote_tick_from_market_data(
        self,
        subscription: Subscription,
        req_id: int,
    ) -> None:
        """
        尝试从累积的市场数据中创建 QuoteTick（报价行情）。
        """
        if req_id not in self._subscription_tick_data:
            return

        tick_data = self._subscription_tick_data[req_id]

        # IB 行情类型：0=BID_SIZE, 1=BID_PRICE, 2=ASK_PRICE, 3=ASK_SIZE
        bid_size = tick_data.get(0)
        bid_price = tick_data.get(1)
        ask_price = tick_data.get(2)
        ask_size = tick_data.get(3)

        # 验证价格是否都存在且有效（正值）
        if (
            bid_price is not None
            and ask_price is not None
            and bid_size is not None
            and ask_size is not None
        ):
            # 创建报价行情
            instrument_id = InstrumentId.from_str(subscription.name[0])
            instrument = self._cache.instrument(instrument_id)
            ts_event = self._clock.timestamp_ns()
            price_magnifier = (
                self._instrument_provider.get_price_magnifier(instrument_id)
                if self._instrument_provider
                else 1
            )
            converted_bid_price = ib_price_to_nautilus_price(bid_price, price_magnifier)
            converted_ask_price = ib_price_to_nautilus_price(ask_price, price_magnifier)

            quote_tick = QuoteTick(
                instrument_id=instrument_id,
                bid_price=instrument.make_price(converted_bid_price),
                ask_price=instrument.make_price(converted_ask_price),
                bid_size=instrument.make_qty(bid_size),
                ask_size=instrument.make_qty(ask_size),
                ts_event=ts_event,
                ts_init=ts_event,
            )

            await self._handle_data(quote_tick)

    async def process_realtime_bar(
        self,
        *,
        req_id: int,
        time: int,
        open_: float,
        high: float,
        low: float,
        close: float,
        volume: Decimal,
        wap: Decimal,
        count: int,
    ) -> None:
        """
        更新实时 5 秒 K 线。
        """
        if not (subscription := self._subscriptions.get(req_id=req_id)):
            return

        bar_type = BarType.from_str(subscription.name)
        instrument = self._cache.instrument(bar_type.instrument_id)

        price_magnifier = (
            self._instrument_provider.get_price_magnifier(bar_type.instrument_id)
            if self._instrument_provider
            else 1
        )
        converted_open = ib_price_to_nautilus_price(open_, price_magnifier)
        converted_high = ib_price_to_nautilus_price(high, price_magnifier)
        converted_low = ib_price_to_nautilus_price(low, price_magnifier)
        converted_close = ib_price_to_nautilus_price(close, price_magnifier)

        # 在创建 Bar 对象之前验证 K 线数据的完整性
        # IB 有时会在盘后交易期间发送损坏的数据
        if not self._validate_bar_prices(
            bar_type=bar_type,
            open_price=converted_open,
            high_price=converted_high,
            low_price=converted_low,
            close_price=converted_close,
            bar_identifier=f"time={time}",
        ):
            return

        bar = Bar(
            bar_type=bar_type,
            open=instrument.make_price(converted_open),
            high=instrument.make_price(converted_high),
            low=instrument.make_price(converted_low),
            close=instrument.make_price(converted_close),
            volume=instrument.make_qty(0 if volume == -1 else volume),
            ts_event=pd.Timestamp.fromtimestamp(time, tz=pytz.utc).value,
            ts_init=self._clock.timestamp_ns(),
            is_revision=False,  # 是否为修订 K 线
        )

        await self._handle_data(bar)

    async def process_historical_data(self, *, req_id: int, bar: BarData) -> None:
        """
        返回请求的历史数据 K 线。
        """
        if request := self._requests.get(req_id=req_id):
            bar_type = request.name[0]  # K 线类型
            bar = await self._ib_bar_to_nautilus_bar(
                bar_type=bar_type,
                bar=bar,
                ts_init=await self._ib_bar_to_ts_init(bar, bar_type),
            )

            if bar:
                request.result.append(bar)
        elif subscription := self._subscriptions.get(req_id=req_id):
            # 从存储的订阅开始时间中获取开始时间
            start = self._subscription_start_times.get(req_id)

            bar = await self._process_bar_data(
                bar_type_str=str(subscription.name),
                bar=bar,
                handle_revised_bars=False,
                historical=True,
                start=start,
            )

            if bar:
                await self._handle_data(bar)
        else:
            self._log.debug(f"在 {req_id=} 上收到 {bar=}")
            return

    async def process_historical_data_end(self, *, req_id: int, start: str, end: str) -> None:
        """
        标记历史 K 线接收结束。
        """
        self._end_request(req_id)

    async def process_historical_data_update(self, *, req_id: int, bar: BarData) -> None:
        """
        如果在 reqHistoricalData 中将 keepUpToDate 设置为 True，则实时接收 K 线。

        类似于 realTimeBars 函数，但返回的数据是历史数据和实时数据的组合，相当于 
        TWS 用于保持图表更新的功能。返回的 K 线是使用实时数据成功更新的。

        """
        if not (subscription := self._subscriptions.get(req_id=req_id)):
            return

        if not isinstance(subscription.handle, functools.partial):
            raise TypeError(f"Expecting partial type subscription method: {subscription=}")

        if bar := await self._process_bar_data(
            bar_type_str=str(subscription.name),
            bar=bar,
            handle_revised_bars=subscription.handle.keywords.get("handle_revised_bars", False),
        ):
            if bar.is_single_price() and bar.open.as_double() == 0:
                self._log.debug(f"忽略价格为 0 的 {bar=}")
            else:
                await self._handle_data(bar)

    async def process_historical_ticks_bid_ask(
        self,
        *,
        req_id: int,
        ticks: list,
        done: bool,
    ) -> None:
        """
        返回请求的历史买入/卖出行情。
        """
        if not done:
            return

        if request := self._requests.get(req_id=req_id):
            instrument_id = InstrumentId.from_str(request.name[0])
            instrument = self._cache.instrument(instrument_id)
            price_magnifier = (
                self._instrument_provider.get_price_magnifier(instrument_id)
                if self._instrument_provider
                else 1
            )

            for tick in ticks:
                ts_event = pd.Timestamp.fromtimestamp(tick.time, tz=pytz.utc).value
                converted_bid_price = ib_price_to_nautilus_price(tick.priceBid, price_magnifier)
                converted_ask_price = ib_price_to_nautilus_price(tick.priceAsk, price_magnifier)

                quote_tick = QuoteTick(
                    instrument_id=instrument_id,
                    bid_price=instrument.make_price(converted_bid_price),
                    ask_price=instrument.make_price(converted_ask_price),
                    bid_size=instrument.make_qty(tick.sizeBid),
                    ask_size=instrument.make_qty(tick.sizeAsk),
                    ts_event=ts_event,
                    ts_init=ts_event,
                )
                request.result.append(quote_tick)

            self._end_request(req_id)

    async def process_historical_ticks_last(self, *, req_id: int, ticks: list, done: bool) -> None:
        """
        返回请求的历史成交记录。
        """
        if not done:
            return

        await self._process_trade_ticks(req_id, ticks)

    async def process_historical_ticks(self, *, req_id: int, ticks: list, done: bool) -> None:
        """
        返回请求的历史行情。
        """
        if not done:
            return

        await self._process_trade_ticks(req_id, ticks)

    async def get_price(self, contract, tick_type="MidPoint"):
        """
        请求特定合约和行情类型的市场数据。

        此方法向 Interactive Brokers 请求给定合约和行情类型的市场数据，等待响应并返回结果。

        参数
        ----------
        contract : IBContract
            请求市场数据的合约详情。
        tick_type : str, optional
            要请求的行情数据类型（默认为 "MidPoint"）。

        返回
        -------
        Any
            市场数据结果。

        异常
        ------
        asyncio.TimeoutError
            如果请求超时。

        """
        req_id = self._next_req_id()
        request = self._requests.add(
            req_id=req_id,
            name=f"{contract.symbol}-{tick_type}",
            handle=functools.partial(
                self._eclient.reqMktData,
                req_id,
                contract,
                tick_type,
                False,
                False,
                [],
            ),
            cancel=functools.partial(self._eclient.cancelMktData, req_id),
        )
        request.handle()

        return await self._await_request(request, timeout=60)

    async def _schedule_bar_completion_timeout(self, bar_type_str: str, bar: BarData) -> None:
        """
        在 K 线周期结束后调度一个超时任务以发布该 K 线。

        这确保了 K 线在其时间周期完成后立即发布，而不是等待下一个 K 线到来。
        这对于收盘（EOD）K 线尤为重要，并能提供更及时的 K 线交付。

        参数
        ----------
        bar_type_str : str
            K 线类型的字符串表示。
        bar : BarData
            超时后可能发布的 K 线数据。

        """
        # 为此 K 线类型取消任何现有的超时任务
        if bar_type_str in self._bar_timeout_tasks:
            self._bar_timeout_tasks[bar_type_str].cancel()

        # 计算该 K 线周期应在何时结束
        bar_type = BarType.from_str(bar_type_str)
        bar_duration_seconds = bar_type.spec.timedelta.total_seconds()

        # 在 K 线周期结束后增加一个小的缓冲（1 秒）以确保其已完成
        timeout_seconds = bar_duration_seconds + 1.0

        async def completion_handler():
            try:
                await asyncio.sleep(timeout_seconds)

                # 检查此 K 线是否仍为当前 K 线（未被取代）
                current_bar = self._bar_type_to_last_bar.get(bar_type_str)  # 当前 K 线

                if current_bar and int(current_bar.date) == int(bar.date):
                    self._log.debug(f"K 线周期完成后发布 K 线：{bar_type_str}")
                    ts_init = self._clock.timestamp_ns()

                    # 将 K 线转换为 Nautilus 格式
                    nautilus_bar = await self._ib_bar_to_nautilus_bar(
                        bar_type=bar_type,
                        bar=current_bar,
                        ts_init=ts_init,
                        is_revision=False,
                    )

                    # 处理 K 线数据
                    if nautilus_bar and not (
                        nautilus_bar.is_single_price() and nautilus_bar.open.as_double() == 0
                    ):
                        await self._handle_data(nautilus_bar)

            except asyncio.CancelledError:
                # 任务被取消，这在有新 K 线到来时是预期的
                pass
            finally:
                # 清理任务引用
                self._bar_timeout_tasks.pop(bar_type_str, None)

        # 创建并存储超时任务
        task = asyncio.create_task(completion_handler())
        self._bar_timeout_tasks[bar_type_str] = task

    async def _process_bar_data(
        self,
        bar_type_str: str,
        bar: BarData,
        handle_revised_bars: bool,
        historical: bool | None = False,
        start: int | None = None,
    ) -> Bar | None:
        """
        处理接收到的 K 线数据并将其转换为 NautilusTrader 的 Bar 格式。
        此方法确定该 K 线是新 K 线还是对现有 K 线进行的修订，并将 K 线数据转换为 
        NautilusTrader 的格式。

        参数
        ----------
        bar_type_str : str
            K 线类型的字符串表示。
        bar : BarData
            从 Interactive Brokers 接收到的 K 线数据。
        handle_revised_bars : bool
            指示是否应处理修订后的 K 线。
        historical : bool | None, optional
            指示 K 线数据是否为历史数据。默认为 False。
        start: int, optional
            订阅的开始时间（纳秒）。

        返回
        -------
        Bar | ``None``

        """
        previous_bar = self._bar_type_to_last_bar.get(bar_type_str)  # 上一根 K 线
        previous_ts = 0 if not previous_bar else int(previous_bar.date)
        current_ts = int(bar.date)

        if current_ts > previous_ts:
            is_new_bar = True
        elif current_ts == previous_ts:
            is_new_bar = False
        else:
            return None  # 同步冲突

        self._bar_type_to_last_bar[bar_type_str] = bar
        bar_type: BarType = BarType.from_str(bar_type_str)
        bar_ts_init = await self._ib_bar_to_ts_init(bar, bar_type)

        if start and bar_ts_init < start:
            # 过滤掉不需要的历史数据，参见 subscribe_historical_bars
            return None

        ts_init = self._clock.timestamp_ns()

        if not handle_revised_bars:
            if previous_bar and is_new_bar:
                # 新 K 线到达 - 立即发布上一个（已完成的）K 线，并为当前 K 线调度完成超时
                await self._schedule_bar_completion_timeout(bar_type_str, bar)
                bar = previous_bar
            else:
                # 第一个 K 线或相同的时间戳 - 调度完成超时，但暂不发布（等待 K 线周期完成）
                await self._schedule_bar_completion_timeout(bar_type_str, bar)
                return None  # 等待 K 线周期完成

            if historical:
                ts_init = await self._ib_bar_to_ts_init(bar, bar_type)

                if ts_init >= self._clock.timestamp_ns():
                    return None  # K 线不完整

        # 处理 K 线数据
        return await self._ib_bar_to_nautilus_bar(
            bar_type=bar_type,
            bar=bar,
            ts_init=ts_init,
            is_revision=not is_new_bar,
        )

    async def _process_trade_ticks(self, req_id: int, ticks: list[HistoricalTickLast]) -> None:
        """
        处理接收到的成交行情数据，将其转换为 NautilusTrader 的 TradeTick 类型，
        并添加到相应请求的结果中。

        参数
        ----------
        req_id : int
            正在处理成交记录的请求标识符。
        ticks : list
            从 Interactive Brokers 接收到的成交行情数据列表。

        """
        if request := self._requests.get(req_id=req_id):
            instrument_id = InstrumentId.from_str(request.name[0])
            instrument = self._cache.instrument(instrument_id)

            price_magnifier = (
                self._instrument_provider.get_price_magnifier(instrument_id)
                if self._instrument_provider
                else 1
            )

            for tick in ticks:
                ts_event = pd.Timestamp.fromtimestamp(tick.time, tz=pytz.utc).value
                converted_price = ib_price_to_nautilus_price(tick.price, price_magnifier)

                trade_tick = TradeTick(
                    instrument_id=instrument_id,
                    price=instrument.make_price(converted_price),
                    size=instrument.make_qty(tick.size),
                    aggressor_side=AggressorSide.NO_AGGRESSOR,
                    trade_id=generate_trade_id(
                        ts_event=ts_event,
                        price=converted_price,
                        size=tick.size,
                    ),
                    ts_event=ts_event,
                    ts_init=ts_event,
                )
                request.result.append(trade_tick)

            self._end_request(req_id)

    async def _handle_data(self, data: Data) -> None:
        """
        处理并向适当的目的地转发已处理的数据。此方法是一个通用的数据处理器，
        负责将处理过的市场数据（如 K 线或行情）转发到 DataEngine.process 消息总线端点。

        参数
        ----------
        data : Data
            准备转发的处理后市场数据。

        """
        self._msgbus.send(endpoint="DataEngine.process", msg=data)

    async def _ib_bar_to_nautilus_bar(
        self,
        bar_type: BarType,
        bar: BarData,
        ts_init: int,
        is_revision: bool = False,
    ) -> Bar | None:
        """
        将 Interactive Brokers 的 K 线数据转换为 NautilusTrader 的 K 线类型。

        参数
        ----------
        bar_type : BarType
            K 线的类型。
        bar : BarData
            从 Interactive Brokers 接收到的 K 线数据。
        ts_init : int
            表示 K 线初始化时间的 unix 纳秒时间戳。
        is_revision : bool, optional
            指示该 K 线是否为对现有 K 线的修订。默认为 False。

        返回
        -------
        Bar | None
            转换后的 K 线；如果 K 线数据无效（例如盘后交易中 low > open），则返回 None。

        """
        instrument = self._cache.instrument(bar_type.instrument_id)

        if not instrument:
            raise ValueError(f"No cached instrument for {bar_type.instrument_id}")

        ts_event = await self._ib_bar_to_ts_event(bar, bar_type)  # 曾为 _convert_ib_bar_date_to_unix_nanos

        # 应用价格乘数转换
        price_magnifier = (
            self._instrument_provider.get_price_magnifier(bar_type.instrument_id)
            if self._instrument_provider
            else 1
        )
        converted_open = ib_price_to_nautilus_price(bar.open, price_magnifier)
        converted_high = ib_price_to_nautilus_price(bar.high, price_magnifier)
        converted_low = ib_price_to_nautilus_price(bar.low, price_magnifier)
        converted_close = ib_price_to_nautilus_price(bar.close, price_magnifier)

        # 在创建 Bar 对象之前验证 K 线数据的完整性
        # IB 有时会在盘后交易期间发送损坏的数据
        if not self._validate_bar_prices(
            bar_type=bar_type,
            open_price=converted_open,
            high_price=converted_high,
            low_price=converted_low,
            close_price=converted_close,
            bar_identifier=f"bar.date={bar.date}",
        ):
            return None

        return Bar(
            bar_type=bar_type,
            open=instrument.make_price(converted_open),
            high=instrument.make_price(converted_high),
            low=instrument.make_price(converted_low),
            close=instrument.make_price(converted_close),
            volume=instrument.make_qty(0 if bar.volume == -1 else bar.volume),
            ts_event=ts_event,
            ts_init=ts_init,
            is_revision=is_revision,
        )

    async def _ib_bar_to_ts_event(self, bar: BarData, bar_type: BarType) -> int:
        """
        计算 K 线的 ts_event 时间戳。

        此方法通过根据 K 线类型的持续时间调整提供的 K 线时间戳，来计算数据事件发生的时间戳。
        ts_event 被设置为 K 线周期的开始时间。

        周/月 K 线从 IB 返回的日期代表结束日期，K 线周期的开始应当分别是周初和月初。

        参数
        ----------
        bar : BarData
            用于计算的 K 线数据。
        bar_type : BarType
            K 线的类型，包含有关 K 线持续时间的信息。

        返回
        -------
        int

        """
        ts_event = 0

        if bar_type.spec.aggregation in [15, 16]:
            date_obj = pd.to_datetime(bar.date, format="%Y%m%d", utc=True)

            if bar_type.spec.aggregation == 15:
                first_day_of_week = date_obj - pd.Timedelta(days=date_obj.weekday())
                ts_event = first_day_of_week.value
            else:
                first_day_of_month = date_obj.replace(day=1)
                ts_event = first_day_of_month.value
        else:
            ts_event = await self._convert_ib_bar_date_to_unix_nanos(bar, bar_type)

        return ts_event

    async def _ib_bar_to_ts_init(self, bar: BarData, bar_type: BarType) -> int:
        """
        计算 K 线的初始化时间戳（ts_init）。

        此方法通过根据 K 线类型的持续时间调整提供的 K 线时间戳，来计算 K 线初始化的时间戳。
        ts_init 被设置为 K 线周期的结束时间，而不是开始时间。

        参数
        ----------
        bar : BarData
            用于计算的 K 线数据。
        bar_type : BarType
            K 线的类型，包含有关 K 线持续时间的信息。

        返回
        -------
        int

        """
        ts = await self._convert_ib_bar_date_to_unix_nanos(bar, bar_type)

        if bar_type.spec.aggregation in [15, 16]:
            # 周/月 K 线的日期代表结束日期
            return ts
        elif bar_type.spec.aggregation == 14:
            # -1 使当日 K 线的 ts_event 和 ts_init 在同一天
            return ts + pd.Timedelta(bar_type.spec.timedelta).value - 1
        else:
            return ts + pd.Timedelta(bar_type.spec.timedelta).value

    async def _convert_ib_bar_date_to_unix_nanos(self, bar: BarData, bar_type: BarType) -> int:
        """
        将 BarData 中的日期转换为 unix 纳秒。

        如果 K 线类型的聚合方式为 14 - 16，从 IB 返回的 K 线日期始终采用 YYYYMMDD 
        格式。对于所有其他聚合方式，返回的 K 线日期为系统时间。

        参数
        ----------
        bar : BarData
            包含要转换日期的 K 线数据。
        bar_type : BarType
            指定聚合级别的 K 线类型。

        返回
        -------
        int

        """
        if bar_type.spec.aggregation in [14, 15, 16]:
            # 日/周/月 K 线返回的日期始终为 YYYYMMDD 格式
            ts = pd.to_datetime(bar.date, format="%Y%m%d", utc=True)
        else:
            ts = pd.Timestamp.fromtimestamp(int(bar.date), tz=pytz.utc)

        return ts.value

    def _validate_bar_prices(
        self,
        bar_type: BarType,
        open_price: float,
        high_price: float,
        low_price: float,
        close_price: float,
        bar_identifier: str,
    ) -> bool:
        if high_price < open_price:
            self._log.warning(
                f"来自 IB 的 {bar_type.instrument_id} K 线无效： "
                f"最高价 ({high_price}) < 开盘价 ({open_price})， "
                f"{bar_identifier}，跳过该 K 线",
            )
            return False

        if high_price < low_price:
            self._log.warning(
                f"来自 IB 的 {bar_type.instrument_id} K 线无效： "
                f"最高价 ({high_price}) < 最低价 ({low_price})， "
                f"{bar_identifier}，跳过该 K 线",
            )
            return False

        if high_price < close_price:
            self._log.warning(
                f"来自 IB 的 {bar_type.instrument_id} K 线无效： "
                f"最高价 ({high_price}) < 收盘价 ({close_price})， "
                f"{bar_identifier}，跳过该 K 线",
            )
            return False

        if low_price > close_price:
            self._log.warning(
                f"来自 IB 的 {bar_type.instrument_id} K 线无效： "
                f"最低价 ({low_price}) > 收盘价 ({close_price})， "
                f"{bar_identifier}，跳过该 K 线",
            )
            return False

        if low_price > open_price:
            self._log.warning(
                f"来自 IB 的 {bar_type.instrument_id} K 线无效： "
                f"最低价 ({low_price}) > 开盘价 ({open_price})， "
                f"{bar_identifier}，跳过该 K 线",
            )
            return False

        return True

    async def process_update_mkt_depth_l2(
        self,
        *,
        req_id: int,
        position: int,
        market_maker: str,
        operation: int,
        side: int,
        price: float,
        size: Decimal,
        is_smart_depth: bool,
    ) -> None:
        """
        返回市场深度 (L2) 实时数据。

        注意
        ----
        IBKR 的 L2 深度数据是按位置（position）更新的，因此我们需要维护一个按位置
        索引的本地订单簿，然后再按价格对该订单簿进行汇总（aggregate）。

        参数
        ----------
        req_id : TickerId
            请求的标识符。
        position : int
            正在更新的订单簿行。
        market_maker : str
            如果 is_smart_depth 为 True，则为持有订单的交易所；否则为做市商的 MPID。
        operation : int
            如何刷新该行：
            - 0: insert（在 'position' 标识的行中插入此新订单）
            - 1: update（更新 'position' 标识的行中的现有订单）
            - 2: delete（删除 'position' 标识的行中的现有订单）
        side : int
            0 表示 ask（卖出），1 表示 bid（买入）。
        price : float
            订单价格。
        size : Decimal
            订单大小。
        is_smart_depth : bool
            是否为 SMART 深度请求。

        """
        if not (subscription := self._subscriptions.get(req_id=req_id)):
            return

        instrument_id = InstrumentId.from_str(subscription.name[0])
        instrument = self._cache.instrument(instrument_id)
        ts_init = self._clock.timestamp_ns()

        # 如果订单簿不存在，则为该证券创建一个新订单簿
        if req_id not in self._order_books:
            self._order_books[req_id] = {"bids": {}, "asks": {}}

        book: dict[str, dict[int, IBKRBookLevel]] = self._order_books[req_id]

        # 选择要更新的出价或要价侧
        order_side = IB_SIDE[side]
        levels: dict[int, IBKRBookLevel] = (
            book["bids"] if order_side == OrderSide.BUY else book["asks"]
        )

        # 基于操作类型更新订单簿
        action = MKT_DEPTH_OPERATIONS[operation]

        if action in (BookAction.ADD, BookAction.UPDATE):
            levels[position] = IBKRBookLevel(
                price=price,
                size=size,
                side=order_side,
                market_maker=market_maker,
            )
        elif action == BookAction.DELETE:
            levels.pop(position, None)

        # 检查订单簿是否已初始化
        # 对于低流动性股票，设定的深度要求可能无法满足，因此暂时禁用初始化检查处理
        # if not self._order_books_initialized.get(req_id, False):
        #     depth = self._order_book_depth[req_id]
        #     if len(book["bids"]) == depth and len(book["asks"]) == depth:
        #         self._order_books_initialized[req_id] = True
        #     else:
        #         return

        # 转换为 OrderBookDeltas
        aggregated_book = self._aggregate_order_book_by_price(book)

        price_magnifier = (
            self._instrument_provider.get_price_magnifier(instrument_id)
            if self._instrument_provider
            else 1
        )

        deltas: list[OrderBookDelta] = [
            OrderBookDelta.clear(
                instrument_id,
                sequence=0,
                ts_event=ts_init,  # 无事件时间戳
                ts_init=ts_init,
            ),
        ]

        bids = [
            BookOrder(
                side=level.side,
                price=instrument.make_price(
                    ib_price_to_nautilus_price(
                        level.price,
                        price_magnifier,
                    ),
                ),
                size=instrument.make_qty(level.size),
                order_id=0,  # 不适用于 L2 数据
            )
            for level in aggregated_book["bids"].values()
        ]

        asks = [
            BookOrder(
                side=level.side,
                price=instrument.make_price(
                    ib_price_to_nautilus_price(
                        level.price,
                        price_magnifier,
                    ),
                ),
                size=instrument.make_qty(level.size),
                order_id=0,  # 不适用于 L2 数据
            )
            for level in aggregated_book["asks"].values()
        ]

        deltas += [
            OrderBookDelta(
                instrument_id,
                BookAction.ADD,
                o,
                flags=0,
                sequence=0,
                ts_event=ts_init,  # 无事件时间戳
                ts_init=ts_init,
            )
            for o in bids + asks
        ]

        await self._handle_data(OrderBookDeltas(instrument_id=instrument_id, deltas=deltas))

    def _aggregate_order_book_by_price(
        self,
        book: dict[str, dict[int, IBKRBookLevel]],
    ) -> dict[str, dict[float, IBKRBookLevel]]:
        """
        按价格对订单簿进行汇总。

        参数
        ----------
        book : dict[str, dict[int, IBKRBookLevel]]
            要汇总的订单簿。

        返回
        -------
        dict[str, dict[float, IBKRBookLevel]]
            汇总后的订单簿。

        """
        aggregated_book: dict[str, dict[float, IBKRBookLevel]] = {}

        for side, order_side in [("bids", OrderSide.BUY), ("asks", OrderSide.SELL)]:
            price_aggregates: dict[float, Decimal] = defaultdict(Decimal)

            for level in book[side].values():
                price_aggregates[level.price] += level.size

            aggregated_book[side] = {
                price: IBKRBookLevel(price=price, size=size, side=order_side, market_maker="")
                for price, size in price_aggregates.items()
            }

        return aggregated_book
