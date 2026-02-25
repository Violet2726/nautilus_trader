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

from __future__ import annotations

import asyncio

import pandas as pd

from nautilus_trader.adapters.interactive_brokers.client import InteractiveBrokersClient
from nautilus_trader.adapters.interactive_brokers.common import IB_VENUE
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersDataClientConfig
from nautilus_trader.adapters.interactive_brokers.parsing.data import timedelta_to_duration_str
from nautilus_trader.adapters.interactive_brokers.providers import (
    InteractiveBrokersInstrumentProvider,
)
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.core.datetime import dt_to_unix_nanos
from nautilus_trader.core.datetime import time_object_to_dt
from nautilus_trader.core.datetime import unix_nanos_to_dt
from nautilus_trader.data.messages import RequestBars
from nautilus_trader.data.messages import RequestData
from nautilus_trader.data.messages import RequestInstrument
from nautilus_trader.data.messages import RequestInstruments
from nautilus_trader.data.messages import RequestQuoteTicks
from nautilus_trader.data.messages import RequestTradeTicks
from nautilus_trader.data.messages import SubscribeBars
from nautilus_trader.data.messages import SubscribeData
from nautilus_trader.data.messages import SubscribeIndexPrices
from nautilus_trader.data.messages import SubscribeInstrument
from nautilus_trader.data.messages import SubscribeInstrumentClose
from nautilus_trader.data.messages import SubscribeInstruments
from nautilus_trader.data.messages import SubscribeInstrumentStatus
from nautilus_trader.data.messages import SubscribeOrderBook
from nautilus_trader.data.messages import SubscribeQuoteTicks
from nautilus_trader.data.messages import SubscribeTradeTicks
from nautilus_trader.data.messages import UnsubscribeBars
from nautilus_trader.data.messages import UnsubscribeData
from nautilus_trader.data.messages import UnsubscribeIndexPrices
from nautilus_trader.data.messages import UnsubscribeInstrument
from nautilus_trader.data.messages import UnsubscribeInstrumentClose
from nautilus_trader.data.messages import UnsubscribeInstruments
from nautilus_trader.data.messages import UnsubscribeInstrumentStatus
from nautilus_trader.data.messages import UnsubscribeOrderBook
from nautilus_trader.data.messages import UnsubscribeQuoteTicks
from nautilus_trader.data.messages import UnsubscribeTradeTicks
from nautilus_trader.live.data_client import LiveMarketDataClient
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import BookType
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.instruments.currency_pair import CurrencyPair


class InteractiveBrokersDataClient(LiveMarketDataClient):
    """
    通过使用 `Gateway` 流式传输市场数据，为 InteractiveBrokers 交易所提供数据客户端。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        客户端的事件循环。
    client : InteractiveBrokersClient
        使用 ibapi 的 Nautilus InteractiveBrokersClient 实例。
    msgbus : MessageBus
        客户端的消息总线。
    cache : Cache
        客户端的缓存。
    clock : LiveClock
        客户端的时钟。
    instrument_provider : InteractiveBrokersInstrumentProvider
        工具提供者。
    ibg_client_id : int
        用于连接 TWS/Gateway 的客户端 ID。
    config : InteractiveBrokersDataClientConfig
        客户端的配置。
    name : str, 可选
        自定义客户端 ID。

    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        client: InteractiveBrokersClient,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        instrument_provider: InteractiveBrokersInstrumentProvider,
        ibg_client_id: int,
        config: InteractiveBrokersDataClientConfig,
        name: str | None = None,
        connection_timeout: int = 300,
    ) -> None:
        super().__init__(
            loop=loop,
            client_id=ClientId(name or f"{IB_VENUE.value}-{ibg_client_id:03d}"),
            venue=None,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=instrument_provider,
            config=config,
        )
        self._connection_timeout = connection_timeout
        self._client = client
        self._handle_revised_bars = config.handle_revised_bars
        self._use_regular_trading_hours = config.use_regular_trading_hours
        self._market_data_type = config.market_data_type
        self._ignore_quote_tick_size_updates = config.ignore_quote_tick_size_updates

    @property
    def instrument_provider(self) -> InteractiveBrokersInstrumentProvider:
        return self._instrument_provider  # type: ignore

    async def _connect(self):
        # 连接客户端
        await self._client.wait_until_ready(self._connection_timeout)
        self._client.registered_nautilus_clients.add(self.id)

        # 在客户端上设置工具提供者，以便访问价格放大系数（price magnifier）
        self._client._instrument_provider = self._instrument_provider

        # 设置行情数据类型
        await self._client.set_market_data_type(self._market_data_type)

        # 根据配置加载工具
        await self.instrument_provider.initialize()
        for instrument in self._instrument_provider.list_all():
            self._handle_data(instrument)

    async def _disconnect(self):
        self._client.registered_nautilus_clients.discard(self.id)

        if self._client.is_running and self._client.registered_nautilus_clients == set():
            self._client.stop()

    async def _subscribe(self, command: SubscribeData) -> None:
        raise NotImplementedError(
            "请实现 `_subscribe` 协程",
        )

    async def _subscribe_instruments(self, command: SubscribeInstruments) -> None:
        raise NotImplementedError(
            "请实现 `_subscribe_instruments` 协程",
        )

    async def _subscribe_instrument(self, command: SubscribeInstrument) -> None:
        raise NotImplementedError(
            "请实现 `_subscribe_instrument` 协程",
        )

    async def _subscribe_index_prices(self, command: SubscribeIndexPrices) -> None:
        contract = self.instrument_provider.contract.get(command.instrument_id)
        if not contract:
            self._log.error(
                f"Cannot subscribe to index prices for {command.instrument_id}: instrument not found",
            )
            return

        if contract.secType != "IND":
            self._log.warning(
                f"Index price subscription not supported for security type {contract.secType}",
            )
            return

        await self._client.subscribe_index_market_data(
            instrument_id=command.instrument_id,
            contract=contract,
            generic_tick_list="",  # Empty for basic price updates
        )

    async def _subscribe_order_book_deltas(self, command: SubscribeOrderBook) -> None:
        if command.book_type == BookType.L3_MBO:
            self._log.error(
                "无法订阅订单簿增量： "
                "Interactive Brokers 不发布 L3_MBO 数据。 "
                "有效的订单簿类型为 L1_MBP, L2_MBP",
            )
            return

        if not (instrument := self._cache.instrument(command.instrument_id)):
            self._log.error(
                f"无法为 {command.instrument_id} 订阅订单簿增量：未找到该工具",
            )
            return

        depth = 20 if not command.depth else command.depth
        is_smart_depth = command.params.get("is_smart_depth", True)

        await self._client.subscribe_order_book(
            instrument_id=command.instrument_id,
            contract=IBContract(**instrument.info["contract"]),
            depth=depth,
            is_smart_depth=is_smart_depth,
        )

    async def _subscribe_quote_ticks(self, command: SubscribeQuoteTicks) -> None:
        contract = self.instrument_provider.contract.get(command.instrument_id)
        if not contract:
            self._log.error(
                f"无法为 {command.instrument_id} 订阅报价：未找到该工具",
            )
            return

        # 默认使用 batch_quotes 以避免“已达到逐笔报价请求最大数量”错误
        batch_quotes = command.params.get("batch_quotes", True)
        if contract.secType == "BAG" or batch_quotes:
            # 对于期权组合 (BAG) 工具，始终使用 reqMktData 而不是 reqTickByTickData，
            # 因为 BAG 合约不支持后者
            await self._client.subscribe_market_data(
                instrument_id=command.instrument_id,
                contract=contract,
                generic_tick_list="",  # Empty for basic bid/ask data
            )
        else:
            await self._client.subscribe_ticks(
                instrument_id=command.instrument_id,
                contract=contract,
                tick_type="BidAsk",
                ignore_size=self._ignore_quote_tick_size_updates,
            )

    async def _subscribe_trade_ticks(self, command: SubscribeTradeTicks) -> None:
        if not (instrument := self._cache.instrument(command.instrument_id)):
            self._log.error(
                f"无法为 {command.instrument_id} 订阅成交：未找到该工具",
            )
            return

        if isinstance(instrument, CurrencyPair):
            self._log.error(
                "Interactive Brokers 不支持货币对 (CurrencyPair) 工具的成交数据",
            )
            return

        await self._client.subscribe_ticks(
            instrument_id=command.instrument_id,
            contract=IBContract(**instrument.info["contract"]),
            tick_type="AllLast",
            ignore_size=self._ignore_quote_tick_size_updates,
        )

    async def _subscribe_bars(self, command: SubscribeBars) -> None:
        contract = self.instrument_provider.contract.get(command.bar_type.instrument_id)

        if not contract:
            self._log.error(
                f"无法为 {command.bar_type.instrument_id} 订阅 K 线：未找到该工具",
            )
            return

        if command.bar_type.spec.timedelta.total_seconds() == 5:
            await self._client.subscribe_realtime_bars(
                bar_type=command.bar_type,
                contract=contract,
                use_rth=self._use_regular_trading_hours,
            )
        else:
            await self._client.subscribe_historical_bars(
                bar_type=command.bar_type,
                contract=contract,
                use_rth=self._use_regular_trading_hours,
                handle_revised_bars=self._handle_revised_bars,
                params=command.params.copy(),
            )

    async def _subscribe_instrument_status(self, command: SubscribeInstrumentStatus) -> None:
        pass  # 作为订单簿的一部分订阅

    async def _subscribe_instrument_close(self, command: SubscribeInstrumentClose) -> None:
        pass  # 作为订单簿的一部分订阅

    async def _unsubscribe(self, command: UnsubscribeData) -> None:
        raise NotImplementedError(
            "请实现 `_unsubscribe` 协程",
        )

    async def _unsubscribe_instruments(self, command: UnsubscribeInstruments) -> None:
        raise NotImplementedError(
            "请实现 `_unsubscribe_instruments` 协程",
        )

    async def _unsubscribe_instrument(self, command: UnsubscribeInstrument) -> None:
        raise NotImplementedError(
            "请实现 `_unsubscribe_instrument` 协程",
        )

    async def _unsubscribe_index_prices(self, command: UnsubscribeIndexPrices) -> None:
        await self._client.unsubscribe_index_market_data(command.instrument_id)

    async def _unsubscribe_order_book_deltas(self, command: UnsubscribeOrderBook) -> None:
        is_smart_depth = command.params.get("is_smart_depth", True)
        await self._client.unsubscribe_order_book(
            instrument_id=command.instrument_id,
            is_smart_depth=is_smart_depth,
        )

    async def _unsubscribe_quote_ticks(self, command: UnsubscribeQuoteTicks) -> None:
        await self._client.unsubscribe_ticks(command.instrument_id, "BidAsk")

    async def _unsubscribe_trade_ticks(self, command: UnsubscribeTradeTicks) -> None:
        await self._client.unsubscribe_ticks(command.instrument_id, "AllLast")

    async def _unsubscribe_bars(self, command: UnsubscribeBars) -> None:
        if command.bar_type.spec.timedelta.total_seconds() == 5:
            await self._client.unsubscribe_realtime_bars(command.bar_type)
        else:
            await self._client.unsubscribe_historical_bars(command.bar_type)

    async def _unsubscribe_instrument_status(self, command: UnsubscribeInstrumentStatus) -> None:
        pass  # 作为订单簿的一部分订阅

    async def _unsubscribe_instrument_close(self, command: UnsubscribeInstrumentClose) -> None:
        pass  # 作为订单簿的一部分订阅

    async def _request(self, request: RequestData) -> None:
        raise NotImplementedError(
            "请实现 `_request` 协程",
        )

    async def _request_instrument(self, request: RequestInstrument) -> None:
        if request.start is not None:
            self._log.warning(
                f"请求具有指定 `start` 的工具 {request.instrument_id}，但这没有效果",
            )

        if request.end is not None:
            self._log.warning(
                f"请求具有指定 `end` 的工具 {request.instrument_id}，但这没有效果",
            )

        await self.instrument_provider.load_with_return_async(
            request.instrument_id,
            request.params,
        )

        if instrument := self.instrument_provider.find(request.instrument_id):
            self._handle_data(instrument)
        else:
            self._log.warning(f"{request.instrument_id} 的工具不可用")
            return

        self._handle_instrument(instrument, request.id, request.start, request.end, request.params)

    async def _request_instruments(self, request: RequestInstruments) -> None:
        loaded_instrument_ids: list[InstrumentId] = []

        if "ib_contracts" in request.params:
            # 我们允许传递 IBContract 参数来构建期货或期权链
            ib_contracts = [IBContract(**d) for d in request.params["ib_contracts"]]
            loaded_instrument_ids = await self.instrument_provider.load_ids_with_return_async(
                ib_contracts,
                request.params,
            )
            loaded_instruments: list[Instrument] = []

            if loaded_instrument_ids:
                for instrument_id in loaded_instrument_ids:
                    instrument = self._cache.instrument(instrument_id)

                    if instrument:
                        loaded_instruments.append(instrument)
                    else:
                        self._log.warning(
                            f"加载后在缓存中未找到工具 {instrument_id}",
                        )
            else:
                self._log.warning("load_ids_async 未返回任何工具 ID")

            self._handle_instruments(
                venue=request.venue,
                instruments=loaded_instruments,
                correlation_id=request.id,
                start=request.start,
                end=request.end,
                params=request.params,
            )
            return

        # 我们确保适配器中也加载了缓存中现有工具的 IB 表示
        instruments = self._cache.instruments()
        instrument_ids = [instrument.id for instrument in instruments]
        loaded_instrument_ids = await self.instrument_provider.load_ids_with_return_async(
            instrument_ids,
            request.params,
        )
        self._handle_instruments(
            venue=request.venue,
            instruments=[],
            correlation_id=request.id,
            start=request.start,
            end=request.end,
            params=request.params,
        )

    async def _request_quote_ticks(self, request: RequestQuoteTicks) -> None:
        if not (instrument := self._cache.instrument(request.instrument_id)):
            self._log.error(
                f"无法请求 {request.instrument_id} 的报价，未找到工具",
            )
            return

        end = request.end or pd.Timestamp.utcnow()

        ticks = await self.get_historical_ticks_paged(
            instrument_id=request.instrument_id,
            contract=IBContract(**instrument.info["contract"]),
            tick_type="BID_ASK",
            start_date_time=request.start,
            end_date_time=end,
            limit=request.limit,
            use_rth=self._use_regular_trading_hours,
            timeout=self._client._request_timeout_secs,
        )
        if not ticks:
            self._log.warning(f"未收到 {request.instrument_id} 的报价数据")
            return

        self._handle_quote_ticks(
            request.instrument_id,
            ticks,
            request.id,
            request.start,
            request.end,
            request.params,
        )

    async def _request_trade_ticks(self, request: RequestTradeTicks) -> None:
        if not (instrument := self._cache.instrument(request.instrument_id)):
            self._log.error(
                f"无法请求 {request.instrument_id} 的成交：未找到工具",
            )
            return

        if isinstance(instrument, CurrencyPair):
            self._log.error(
                "Interactive Brokers 不支持货币对 (CurrencyPair) 工具的成交数据",
            )
            return

        end = request.end or pd.Timestamp.utcnow()

        ticks = await self.get_historical_ticks_paged(
            instrument_id=request.instrument_id,
            contract=IBContract(**instrument.info["contract"]),
            tick_type="TRADES",
            start_date_time=request.start,
            end_date_time=end,
            limit=request.limit,
            use_rth=self._use_regular_trading_hours,
            timeout=self._client._request_timeout_secs,
        )
        if not ticks:
            self._log.warning(f"未收到 {request.instrument_id} 的成交数据")
            return

        self._handle_trade_ticks(
            request.instrument_id,
            ticks,
            request.id,
            request.start,
            request.end,
            request.params,
        )

    async def get_historical_ticks_paged(
        self,
        instrument_id: InstrumentId,
        contract: IBContract,
        tick_type: str,
        start_date_time: pd.Timestamp,
        end_date_time: pd.Timestamp,
        use_rth: bool = True,
        timeout: int = 60,
        limit: int = 0,
    ) -> list[TradeTick | QuoteTick]:
        """
        使用分页检索历史逐笔行情（ticks），以处理大时间范围的情况。

        此方法从 end_date_time 开始向后迭代，请求成批的行情，直到达到 
        start_date_time 或满足 limit 要求。

        当同时指定时间范围和限制（limit）时，方法将在达到 start_date_time 或
        满足 limit 时停止，以先到者为准。如果仅指定了 limit 而没有 start_date_time 
        边界，则分页将继续，直到达到 limit 或没有更多数据可用。

        参数
        ----------
        instrument_id : InstrumentId
            要检索行情的工具标识符。
        contract : IBContract
            该工具的 Interactive Brokers 合约详情。
        tick_type : str
            要检索的行情类型（"TRADES" 或 "BID_ASK"）。
        start_date_time : pd.Timestamp
            行情的开始日期时间。
        end_date_time : pd.Timestamp
            行情的结束日期时间。
        limit : int, 默认 0
            要检索的最大行情数量。如果为 0，则不设限制。
        use_rth : bool, 默认 True
            是否使用常规交易时段（Regular Trading Hours）。
        timeout : int, 默认 60
             每个单独请求的超时时间（秒）。

        返回
        -------
        list[TradeTick | QuoteTick]
            按初始化时间戳排序的汇总行情列表，已过滤到请求的时间范围，
            如果提供了 limit，则限制为指定数量。

        """
        data: list[TradeTick | QuoteTick] = []

        # 确保使用 UTC
        start_date_time = time_object_to_dt(start_date_time)
        current_end_date_time = time_object_to_dt(end_date_time)
        start_date_time_nanos = dt_to_unix_nanos(start_date_time)
        end_date_time_nanos = dt_to_unix_nanos(end_date_time)

        # 使用 1 毫秒的递减量，以避免高频数据中出现重复或跳过的逐笔行情
        TIMESTAMP_DECREMENT_NS = 1_000_000

        await self._client.wait_until_ready()

        while current_end_date_time > start_date_time and (limit == 0 or len(data) < limit):
            self._log.info(
                f"{instrument_id}: 正在请求时间截止到 {current_end_date_time} 的 {tick_type} 逐笔行情",
            )

            ticks = await self._client.get_historical_ticks(
                instrument_id=instrument_id,
                contract=contract,
                tick_type=tick_type,
                end_date_time=current_end_date_time,
                use_rth=use_rth,
                timeout=timeout,
            )

            # 如果未返回任何行情，则提前中断（已达到可用数据的起点）
            if not ticks:
                break

            self._log.info(
                f"{instrument_id}: 批次中检索到的 {tick_type} 逐笔行情数量：{len(ticks)}",
            )

            # 过滤行情以确保它们在请求的时间范围内
            # 向后迭代时，过滤掉在 start_date_time 之前的行情
            filtered_ticks = [
                tick
                for tick in ticks
                if start_date_time_nanos <= tick.ts_init <= end_date_time_nanos
            ]

            if not filtered_ticks:
                # 批次中没有行情处于范围内，中断以避免死循环
                break

            # 从过滤后的逐笔行情中查找最小时间戳
            min_timestamp_nanos = min(tick.ts_init for tick in filtered_ticks)

            # 将 end_date_time 更新为最小时间戳之前的 1ms，以避免重复
            current_end_date_time = unix_nanos_to_dt(min_timestamp_nanos - TIMESTAMP_DECREMENT_NS)

            data.extend(filtered_ticks)
            self._log.info(f"数据中的 {tick_type} 逐笔行情总数：{len(data)}")

            # 如果达到限制，则提前中断
            if limit > 0 and len(data) >= limit:
                break

        sorted_data = sorted(data, key=lambda x: x.ts_init)

        # 如果指定了 limit，则应用限制（修剪为最近的行情）
        if limit > 0 and len(sorted_data) > limit:
            sorted_data = sorted_data[-limit:]

        return sorted_data

    async def _request_bars(self, request: RequestBars) -> None:
        contract = self.instrument_provider.contract.get(request.bar_type.instrument_id)
        if not contract:
            self._log.error(f"无法请求 {request.bar_type} K 线：未找到工具")
            return

        if not request.bar_type.spec.is_time_aggregated():
            self._log.error(
                f"无法请求 {request.bar_type} K 线：Interactive Brokers 仅通过时间聚合 K 线",
            )
            return

        duration = request.end - request.start
        duration_str = timedelta_to_duration_str(duration)
        bars = await self.get_historical_bars_chunked(
            bar_type=request.bar_type,
            contract=contract,
            start_date_time=request.start,
            end_date_time=request.end,
            duration=duration_str,
            use_rth=self._use_regular_trading_hours,
            timeout=self._client._request_timeout_secs,
        )

        if bars:
            bars = list(set(bars))
            bars.sort(key=lambda x: x.ts_init)

            # 如果指定了 limit，则应用限制
            limit = request.limit
            if limit > 0 and len(bars) > limit:
                bars = bars[-limit:]

            self._handle_bars(
                request.bar_type,
                bars,
                request.id,
                request.start,
                request.end,
                request.params,
            )
            status_msg = {"id": request.id, "status": "Success"}
        else:
            self._log.warning(f"未收到 {request.bar_type} 的 K 线数据")
            status_msg = {"id": request.id, "status": "Failed"}

        # 发布状态事件
        self._msgbus.publish(
            topic=f"requests.{request.id}",
            msg=status_msg,
        )

    async def get_historical_bars_chunked(
        self,
        bar_type: BarType,
        contract: IBContract,
        start_date_time: pd.Timestamp | None = None,
        end_date_time: pd.Timestamp | None = None,
        duration: str | None = None,
        use_rth: bool = True,
        timeout: int = 60,
    ) -> list[Bar]:
        """
        分块检索历史 K 线，以处理大时长请求。

        此方法将大型历史数据请求分解为较小的时段（年、天、秒），以符合 IB API 
        的限制并避免超时。它遍历这些时段并汇总结果。

        参数
        ----------
        bar_type : BarType
            要检索的 K 线类型。
        contract : IBContract
             该工具的 Interactive Brokers 合约详情。
        start_date_time : datetime.datetime
             K 线的开始日期时间。如果提供，则推导时长（duration）。
        end_date_time : datetime.datetime
             K 线的结束日期时间。
        duration : str
             从 end_date_time 向回溯的时间量。
        use_rth : bool, 默认 True
             是否使用常规交易时段。
        timeout : int, 默认 60
             每个单独请求时段的超时时间（秒）。

        返回
        -------
        list[Bar]
             按初始化时间戳排序的汇总 Bar 对象列表。

        """
        # 根据时区调整开始和结束时间
        if start_date_time:
            start_date_time = time_object_to_dt(start_date_time)

        if end_date_time:
            end_date_time = time_object_to_dt(end_date_time)

        data: list[Bar] = []

        # 我们需要根据开始/结束时间或时长来计算时长时段（duration segments）
        segments = self._calculate_duration_segments(
            start_date_time,
            end_date_time,
            duration,
        )

        for segment_end_date_time, segment_duration in segments:
            self._log.info(
                f"{bar_type.instrument_id}: 正在请求历史 K 线：{bar_type}，截止日期为 '{segment_end_date_time}'，"
                f"时长为 '{segment_duration}'",
            )

            bars = await self._client.get_historical_bars(
                bar_type,
                contract,
                use_rth,
                segment_end_date_time,
                segment_duration,
                timeout=timeout,
            )
            if bars:
                self._log.info(
                    f"{bar_type.instrument_id}: 批次中检索到的 K 线数量：{len(bars)}",
                )
                data.extend(bars)
                self._log.info(f"数据中的 K 线总数：{len(data)}")
            else:
                self._log.info(f"{bar_type.instrument_id}: 未检索到 {bar_type} 的 K 线数据")

        return sorted(data, key=lambda x: x.ts_init)

    def _calculate_duration_segments(
        self,
        start_date: pd.Timestamp | None,
        end_date: pd.Timestamp,
        duration: str | None,
    ) -> list[tuple[pd.Timestamp, str]]:
        # 计算两个日期之间在年、天和秒方面的差异，以便为历史 K 线请求特定的日期范围。
        #
        # 此函数将两个提供的日期（start_date 和 end_date）之间的时间差分解为不同的组成部分：年、天和秒。
        # 在计算年时，它考虑了闰年，并考虑了详细的时间组成部分（小时、分钟、秒）以精确计算秒。
        #
        # 时间差的每个组成部分（年、天、秒）在返回的列表中表示为一个元组。
        # 第一个元素是从 start_date 移动到 end_date 时，指示该时间段结束点的日期。
        # 例如，如果函数计算出 1 年，则年份条目的日期将是 start_date 经过 1 年后的结束日期。
        # 这有助于理解从 start_date 到 end_date 在分段间隔内的时间进展。

        if duration:
            return [(end_date, duration)]

        total_delta = end_date - start_date

        # 计算时间间隔中的整年数
        years = total_delta.days // 365
        minus_years_date = end_date - pd.Timedelta(days=365 * years)

        # 计算减去整年后的剩余天数
        days = (minus_years_date - start_date).days
        minus_days_date = minus_years_date - pd.Timedelta(days=days)

        # 计算以秒为单位的剩余时间
        delta = minus_days_date - start_date
        subsecond = (
            1
            if delta.components.milliseconds > 0
            or delta.components.microseconds > 0
            or delta.components.nanoseconds > 0
            else 0
        )
        seconds = (
            delta.components.hours * 3600
            + delta.components.minutes * 60
            + delta.components.seconds
            + subsecond
        )

        results = []

        if years:
            results.append((end_date, f"{years} Y"))

        if days:
            results.append((minus_years_date, f"{days} D"))

        if seconds:
            results.append((minus_days_date, f"{seconds} S"))

        return results
