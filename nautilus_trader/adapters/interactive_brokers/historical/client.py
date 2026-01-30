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

import datetime
import re
from typing import Literal

import msgspec
import pandas as pd
from ibapi.common import MarketDataTypeEnum

from nautilus_trader.adapters.interactive_brokers.client import InteractiveBrokersClient
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersDataClientConfig
from nautilus_trader.adapters.interactive_brokers.config import (
    InteractiveBrokersInstrumentProviderConfig,
)
from nautilus_trader.adapters.interactive_brokers.data import InteractiveBrokersDataClient
from nautilus_trader.adapters.interactive_brokers.parsing.instruments import (
    ib_contract_to_instrument_id,
)
from nautilus_trader.adapters.interactive_brokers.providers import (
    InteractiveBrokersInstrumentProvider,
)
from nautilus_trader.cache.cache import Cache
from nautilus_trader.cache.config import CacheConfig
from nautilus_trader.cache.database import CacheDatabaseAdapter
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.component import init_logging
from nautilus_trader.common.component import log_level_from_str
from nautilus_trader.common.functions import get_event_loop
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarSpecification
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import AggregationSource
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.serialization.serializer import MsgSpecSerializer


class HistoricInteractiveBrokersClient:
    """
    提供回测所需的历史行情数据请求方法。
    """

    def __init__(
        self,
        host: str = "127.0.0.1",
        port: int = 7497,
        client_id: int = 1,
        market_data_type: MarketDataTypeEnum = MarketDataTypeEnum.REALTIME,
        log_level: str = "INFO",
        cache_config: CacheConfig | None = None,
        instrument_provider_config: InteractiveBrokersInstrumentProviderConfig | None = None,
    ) -> None:
        loop = get_event_loop()

        loop.set_debug(True)
        self._clock = LiveClock()

        self._log_guard = init_logging(level_stdout=log_level_from_str(log_level))

        self.log = Logger(name="HistoricInteractiveBrokersClient")
        trader_id = TraderId("historic_interactive_brokers_client-001")
        msgbus = MessageBus(
            trader_id,
            self._clock,
        )
        self.market_data_type = market_data_type
        if not cache_config or not cache_config.database:
            cache_db = None
        elif cache_config.database.type == "redis":
            encoding = cache_config.encoding.lower()
            cache_db = CacheDatabaseAdapter(
                trader_id=trader_id,
                instance_id=UUID4(),
                serializer=MsgSpecSerializer(
                    encoding=msgspec.msgpack if encoding == "msgpack" else msgspec.json,
                    timestamps_as_str=True,  # 目前为硬编码
                    timestamps_as_iso8601=cache_config.timestamps_as_iso8601,
                ),
                config=cache_config,
            )
        else:
            raise ValueError(
                f"无法识别的 `cache_config.database.type`：'{cache_config.database.type}'。 "
                "目前仅支持 'redis' 数据库类型。如果您不想使用缓存数据库，"
                "可以为 `cache_config.database` 传递 `None`。",
            )
 
        self._client = InteractiveBrokersClient(
            loop=loop,
            msgbus=msgbus,
            cache=Cache(database=cache_db, config=cache_config) if cache_config else Cache(),
            clock=self._clock,
            host=host,
            port=port,
            client_id=client_id,
        )
        self._client.start()
 
        # 存储工具提供者配置并仅创建一次提供者
        if instrument_provider_config is None:
            instrument_provider_config = InteractiveBrokersInstrumentProviderConfig()

        self._instrument_provider_config = instrument_provider_config
        instrument_provider = InteractiveBrokersInstrumentProvider(
            self._client,
            self._clock,
            instrument_provider_config,
        )

        self._data_client = InteractiveBrokersDataClient(
            loop=loop,
            client=self._client,
            msgbus=msgbus,
            cache=Cache(database=cache_db, config=cache_config) if cache_config else Cache(),
            clock=self._clock,
            instrument_provider=instrument_provider,
            ibg_client_id=client_id,
            config=InteractiveBrokersDataClientConfig(
                market_data_type=market_data_type,
            ),
        )

    async def connect(self) -> None:
        # 连接客户端
        await self._data_client._connect()

    async def request_instruments(
        self,
        instrument_ids: list[str | InstrumentId] | None = None,
        contracts: list[IBContract] | None = None,
    ) -> list[Instrument]:
        """
        根据 IB 合约列表和/或 InstrumentId 字符串列表返回工具 (Instruments)。
 
        Parameters
        ----------
        instrument_ids : list[str | InstrumentId], 默认 'None'
            定义要检索哪些工具的工具 ID（例如 AAPL.NASDAQ）。
            可以是字符串或 InstrumentId 对象。
        contracts : list[IBContract], 默认 'None'
            定义要检索哪些工具的 IB 合约。
 
        Returns
        -------
        list[Instrument]
 
        """
        # 将字符串类型的 instrument_ids 转换为 InstrumentId 对象
        converted_instrument_ids = [
            InstrumentId.from_str(instrument_id)
            if isinstance(instrument_id, str)
            else instrument_id
            for instrument_id in (instrument_ids or [])
        ]
 
        await self._data_client.instrument_provider.load_ids_async(
            converted_instrument_ids + (contracts or []),
        )
 
        return list(self._data_client.instrument_provider._instruments.values())

    async def request_bars(
        self,
        bar_specifications: list[str],
        end_date_time: datetime.datetime,
        tz_name: str,
        start_date_time: datetime.datetime | None = None,
        duration: str | None = None,
        contracts: list[IBContract] | None = None,
        instrument_ids: list[str | InstrumentId] | None = None,
        use_rth: bool = True,
        timeout: int = 120,
    ) -> list[Bar]:
        """
        根据 IB 合约列表和/或 InstrumentId 字符串列表，返回一个或多个 K 线规范 (BarSpecifications) 的 K 线 (Bars)。
 
        Parameters
        ----------
        bar_specifications : list[str]
            表示为字符串的 K 线规范，定义要检索哪些 K 线。
            （例如：'1-HOUR-LAST', '5-MINUTE-MID'）
        start_date_time : datetime.datetime
            K 线的开始日期时间。如果提供，则会自动推导出时长 (duration)。
        end_date_time : datetime.datetime
            K 线的结束日期时间。
            注意：对于连续期货 (CONTFUT)，下载的数据始终截至目前。
        tz_name : str
            要使用的时区。（例如：'America/New_York', 'UTC'）
        duration : str
            从 end_date_time 向前追溯的时间量。
            有效值遵循整数后跟 S|D|W|M|Y 的模式，
            分别代表秒、天、周、月或年。
        contracts : list[IBContract], 默认 'None'
            定义要检索哪些 K 线的 IB 合约。
        instrument_ids : list[str | InstrumentId], 默认 'None'
            定义要检索哪些 K 线的工具 ID（例如 AAPL.NASDAQ）。
            可以是字符串或 InstrumentId 对象。
        use_rth : bool, 默认 'True'
            是否使用常规交易时间 (Regular Trading Hours)。
        timeout : int, 默认 120
            每个请求的超时时间（秒）。
 
        Returns
        -------
        list[Bar]
 
        """
        # 执行所有必要的验证（从 _prepare_request_bars_parameters 合并而来）
        if start_date_time and duration:
            raise ValueError("应提供 start_date_time 或 duration 其中之一，不能两者都提供。")

        # 根据时区调整开始和结束时间
        if start_date_time:
            start_date_time = pd.Timestamp(start_date_time, tz=tz_name).tz_convert("UTC")
 
        end_date_time = pd.Timestamp(end_date_time, tz=tz_name).tz_convert("UTC")

        if start_date_time and start_date_time >= end_date_time:
            raise ValueError("开始日期必须早于结束日期。")

        if duration:
            pattern = r"^\d+\s[SDWMY]$"
 
            if not re.match(pattern, duration):
                raise ValueError("duration 必须符合格式：'int S|D|W|M|Y'")

        # 准备合约和工具 ID
        contracts = contracts or []
        instrument_ids = instrument_ids or []

        if not contracts and not instrument_ids:
            raise ValueError("必须提供 contracts 或 instrument_ids 其中之一")

        # 将 instrument_id 字符串或 InstrumentId 对象转换为 IB 合约
        contracts.extend(
            [
                await self._data_client.instrument_provider.instrument_id_to_ib_contract(
                    InstrumentId.from_str(instrument_id)
                    if isinstance(instrument_id, str)
                    else instrument_id,
                )
                for instrument_id in instrument_ids
            ],
        )

        # 确保工具已被获取并缓存
        await self._fetch_instruments_if_not_cached(contracts)
        data: list[Bar] = []

        for contract in contracts:
            for bar_spec in bar_specifications:
                venue = self._data_client.instrument_provider.determine_venue_from_contract(
                    contract,
                )
                instrument_id = ib_contract_to_instrument_id(
                    contract,
                    venue,
                    self._instrument_provider_config.symbology_method,
                )
                bar_type = BarType(
                    instrument_id,
                    BarSpecification.from_str(bar_spec),
                    AggregationSource.EXTERNAL,
                )

                bars = await self._data_client.get_historical_bars_chunked(
                    bar_type=bar_type,
                    contract=contract,
                    start_date_time=start_date_time,
                    end_date_time=end_date_time,
                    duration=duration,
                    use_rth=use_rth,
                    timeout=timeout,
                )

                if bars:
                    data.extend(bars)

        return sorted(data, key=lambda x: x.ts_init)

    async def request_ticks(
        self,
        tick_type: Literal["TRADES", "BID_ASK"],
        start_date_time: datetime.datetime,
        end_date_time: datetime.datetime,
        tz_name: str,
        contracts: list[IBContract] | None = None,
        instrument_ids: list[str | InstrumentId] | None = None,
        use_rth: bool = True,
        timeout: int = 60,
        limit: int = 0,
    ) -> list[TradeTick | QuoteTick]:
        """
        根据 IB 合约列表和/或 InstrumentId 字符串列表，返回一个或多个工具的成交逐笔数据 (TradeTicks) 或报价逐笔数据 (QuoteTicks)。
 
        Parameters
        ----------
        tick_type : Literal["TRADES", "BID_ASK"]
            要检索的逐笔数据类型。
        start_date_time : datetime.date
            逐笔数据的开始日期。
        end_date_time : datetime.date
            逐笔数据的结束日期。
        tz_name : str
            要使用的时区。（例如：'America/New_York', 'UTC'）
        contracts : list[IBContract], 默认 'None'
            定义要检索哪些逐笔数据的 IB 合约。
        instrument_ids : list[str | InstrumentId], 默认 'None'
            定义要检索哪些逐笔数据的工具 ID（例如 AAPL.NASDAQ）。
            可以是字符串或 InstrumentId 对象。
        use_rth : bool, 默认 'True'
            是否使用常规交易时间。
        timeout : int, 默认 60
            每个请求的超时时间（秒）。
        limit : int, 默认 0
            要检索的最大逐笔数据数量。如果为 0，则不应用限制。
 
        Returns
        -------
        list[TradeTick | QuoteTick]
 
        """
        if tick_type not in ["TRADES", "BID_ASK"]:
            raise ValueError(
                "tick_type 必须是以下之一：'TRADES'（用于 TradeTicks），'BID_ASK'（用于 QuoteTicks）",
            )
 
        if start_date_time >= end_date_time:
            raise ValueError("开始日期必须早于结束日期。")

        start_date_time = pd.Timestamp(start_date_time, tz=tz_name).tz_convert("UTC")
        end_date_time = pd.Timestamp(end_date_time, tz=tz_name).tz_convert("UTC")

        if (end_date_time - start_date_time) > pd.Timedelta(days=1):
            self.log.warning(
                "请求超过 1 天的逐笔数据可能需要很长时间，特别是对于流动性好的工具。 "
                "您可能需要考虑从其他地方获取逐笔数据",
            )

        contracts = contracts or []
        instrument_ids = instrument_ids or []

        if not contracts and not instrument_ids:
            raise ValueError("必须提供 contracts 或 instrument_ids 其中之一")

        # 将 instrument_id 字符串或 InstrumentId 对象转换为 IB 合约
        contracts.extend(
            [
                await self._data_client.instrument_provider.instrument_id_to_ib_contract(
                    InstrumentId.from_str(instrument_id)
                    if isinstance(instrument_id, str)
                    else instrument_id,
                )
                for instrument_id in instrument_ids
            ],
        )

        # 确保工具已被获取并缓存
        await self._fetch_instruments_if_not_cached(contracts)
        data: list[TradeTick | QuoteTick] = []

        for contract in contracts:
            venue = self._data_client.instrument_provider.determine_venue_from_contract(contract)
            instrument_id = ib_contract_to_instrument_id(
                contract,
                venue,
                self._instrument_provider_config.symbology_method,
            )

            ticks = await self._data_client.get_historical_ticks_paged(
                instrument_id=instrument_id,
                contract=contract,
                tick_type=tick_type,
                start_date_time=start_date_time,
                end_date_time=end_date_time,
                limit=limit,
                use_rth=use_rth,
                timeout=timeout,
            )

            if ticks:
                data.extend(ticks)

        return sorted(data, key=lambda x: x.ts_init)

    async def _fetch_instruments_if_not_cached(
        self,
        contracts: list[IBContract],
    ) -> None:
        """
        如果给定的 IB 合约尚未缓存，则为其获取并缓存工具 (Instruments)。
 
        Parameters
        ----------
        contracts : list[IBContract]
            要获取工具的 IB 合约列表。
 
        Returns
        -------
        None
 
        """
        for contract in contracts:
            venue = self._data_client.instrument_provider.determine_venue_from_contract(contract)
            instrument_id = ib_contract_to_instrument_id(
                contract,
                venue,
                self._instrument_provider_config.symbology_method,
            )

            if not self._client._cache.instrument(instrument_id):
                self.log.info(f"正在获取工具：{instrument_id}")
                await self.request_instruments(
                    contracts=[contract],
                )
