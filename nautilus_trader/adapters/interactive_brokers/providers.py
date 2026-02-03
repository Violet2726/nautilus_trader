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

import copy

import pandas as pd
from ibapi.contract import ContractDetails

from nautilus_trader.adapters.interactive_brokers.client import InteractiveBrokersClient
from nautilus_trader.adapters.interactive_brokers.common import ComboLeg
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.common import IBContractDetails
from nautilus_trader.adapters.interactive_brokers.common import dict_to_contract_details
from nautilus_trader.adapters.interactive_brokers.config import (
    InteractiveBrokersInstrumentProviderConfig,
)
from nautilus_trader.adapters.interactive_brokers.parsing.instruments import VENUE_MEMBERS
from nautilus_trader.adapters.interactive_brokers.parsing.instruments import (
    instrument_id_to_ib_contract,
)
from nautilus_trader.adapters.interactive_brokers.parsing.instruments import (
    parse_futures_spread_instrument_id,
)
from nautilus_trader.adapters.interactive_brokers.parsing.instruments import parse_instrument
from nautilus_trader.adapters.interactive_brokers.parsing.instruments import (
    parse_option_spread_instrument_id,
)
from nautilus_trader.common.component import Clock
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.config import resolve_path
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import generic_spread_id_to_list
from nautilus_trader.model.identifiers import is_generic_spread_id
from nautilus_trader.model.identifiers import new_generic_spread_id
from nautilus_trader.model.instruments import Instrument


class InteractiveBrokersInstrumentProvider(InstrumentProvider):
    """
    提供通过 Interactive Brokers 加载 `Instrument` 对象的方法。
    """

    def __init__(
        self,
        client: InteractiveBrokersClient,
        clock: Clock,
        config: InteractiveBrokersInstrumentProviderConfig,
    ) -> None:
        """
        初始化 ``InteractiveBrokersInstrumentProvider`` 类的新实例。
 
        Parameters
        ----------
        client : InteractiveBrokersClient
            Interactive Brokers 客户端。
        clock : Clock
            提供者的时钟。
        config : InteractiveBrokersInstrumentProviderConfig
            工具提供者配置。
 
        """
        super().__init__(config=config)

        # 配置
        self._load_contracts_on_start = (
            set(config.load_contracts) if config.load_contracts is not None else None
        )
        self._min_expiry_days = config.min_expiry_days
        self._max_expiry_days = config.max_expiry_days
        self._build_options_chain = config.build_options_chain
        self._build_futures_chain = config.build_futures_chain
        self._cache_validity_days = config.cache_validity_days
        self._convert_exchange_to_mic_venue = config.convert_exchange_to_mic_venue
        self._symbol_to_mic_venue = config.symbol_to_mic_venue
        self._filter_sec_types = set(config.filter_sec_types)
        # 待办: 如果 cache_validity_days > 0 且提供了 Catalog

        self._client = client
        self._clock = clock
        self.config = config
        self.contract_details: dict[InstrumentId, IBContractDetails] = {}
        self.contract_id_to_instrument_id: dict[int, InstrumentId] = {}
        self.contract: dict[InstrumentId, IBContract] = {}

    async def initialize(self, reload: bool = False) -> None:
        await super().initialize(reload)

        # 仅当 `load_ids_on_start` 为 False 且 `load_contracts_on_start` 为 True 时触发合约加载
        if not self._load_ids_on_start and self._load_contracts_on_start:
            self._loaded = False
            self._loading = True
            await self.load_all_async()  # 加载启动时作为配置传递的所有工具
            self._loading = False
            self._loaded = True

    def _is_filtered_sec_type(self, sec_type: str | None) -> bool:
        return bool(sec_type and sec_type in self._filter_sec_types)

    @property
    def filter_sec_types(self) -> set[str]:
        """
        返回已过滤的证券类型集合。
        """
        return self._filter_sec_types

    async def get_instrument(self, contract: IBContract) -> Instrument | None:
        if self._is_filtered_sec_type(contract.secType):
            self._log.warning(
                f"跳过合约 {contract} 已过滤的 {contract.secType=}",
            )
            return None

        contract_id = contract.conId
        instrument_id = self.contract_id_to_instrument_id.get(contract_id)

        # 检查是否已有该工具
        if instrument_id:
            instrument = self.find(instrument_id)
            if instrument is not None:
                return instrument

        # 对 BAG 合约的特殊处理
        if contract.secType == "BAG":
            return await self._load_bag_contract(contract)

        # 对于非 BAG 合约，使用常规加载
        instrument_ids = await self.load_with_return_async(contract)
        if instrument_ids is None:
            self._log.error(f"无法为合约 {contract} 加载工具")
            raise ValueError(f"未找到合约 {contract} 的工具")

        instrument = self.find(instrument_ids[0])
        if instrument is None:
            self._log.error(f"无法为合约 {contract} 加载工具")
            raise ValueError(f"未找到合约 {contract} 的工具")

        return instrument

    async def _load_bag_contract(self, bag_contract: IBContract) -> Instrument:
        # 从现有的 IB BAG 合约（例如，来自订单信息）加载 BAG 合约工具。
        # 加载每个腿（leg）工具，创建组合 ID，查询 BAG 详情以获取最小报价单位，并创建组合工具。
        if bag_contract.secType != "BAG" or not bag_contract.comboLegs:
            raise ValueError(f"无效的 BAG 合约: {bag_contract}")

        try:
            self._log.info(f"正在加载 BAG 合约: {bag_contract}")

            # 首先，加载所有单个腿工具并收集其详细信息
            leg_contract_details = []
            leg_tuples = []
            for combo_leg in bag_contract.comboLegs:
                # 使用来自组合腿的信息创建一个更完整的腿合约
                leg_contract = IBContract(
                    conId=combo_leg.conId,
                    exchange=combo_leg.exchange,
                    # 使用来自 BAG 合约的基础证券代码和货币
                    symbol=bag_contract.symbol,
                    currency=bag_contract.currency,
                )
                leg_instrument = await self.get_instrument(leg_contract)
                leg_instrument_id = leg_instrument.id

                # 获取此腿的合约详情
                if leg_instrument_id not in self.contract_details:
                    raise ValueError(f"未找到腿 {leg_instrument_id} 的合约详情")

                leg_details = self.contract_details[leg_instrument_id]

                # 确定比例（BUY 为正，SELL 为负）
                ratio = combo_leg.ratio if combo_leg.action == "BUY" else -combo_leg.ratio
                leg_contract_details.append((leg_details, ratio))
                leg_tuples.append((leg_instrument_id, ratio))

            # 直接从加载的腿工具 ID 创建工具 ID
            instrument_id = new_generic_spread_id(leg_tuples)

            # 创建 BAG 合约（IB 不支持 BAG 合约的合约详情）
            bag_contract = await self._create_bag_contract(
                leg_contract_details,
                instrument_id,
                bag_contract,
                bag_contract.exchange,
            )

            # 使用通用的价差创建逻辑
            spread_instrument = self._create_spread_instrument(
                instrument_id,
                leg_contract_details,
                bag_contract,
            )

            return spread_instrument

        except Exception as e:
            self._log.error(f"加载 BAG 合约失败: {e}")
            raise ValueError(f"加载 BAG 合约失败: {e}") from e

    async def instrument_id_to_ib_contract(
        self,
        instrument_id: InstrumentId,
    ) -> IBContract | None:
        venue = instrument_id.venue.value

        possible_exchanges = VENUE_MEMBERS.get(venue, [venue])
        if len(possible_exchanges) == 1:
            return instrument_id_to_ib_contract(
                instrument_id,
                possible_exchanges[0],
                self.config.symbology_method,
                self.contract_details,
            )
        elif await self.fetch_instrument_id(instrument_id):
            return self.contract[instrument_id]
        else:
            return None

    async def instrument_id_to_ib_contract_details(
        self,
        instrument_id: InstrumentId,
    ) -> IBContractDetails | None:
        if await self.fetch_instrument_id(instrument_id):
            return self.contract_details[instrument_id]

        return None

    def get_price_magnifier(self, instrument_id: InstrumentId) -> int:
        contract_details = self.contract_details.get(instrument_id)
        if contract_details:
            return contract_details.priceMagnifier

        return 1

    async def load_all_async(self, filters: dict | None = None) -> None:
        start_instrument_ids = [
            (InstrumentId.from_str(i) if isinstance(i, str) else i)
            for i in (self._load_ids_on_start or [])
        ]
        start_ib_contracts = [
            (IBContract(**c) if isinstance(c, dict) else c)
            for c in (self._load_contracts_on_start or [])
        ]

        await self.load_ids_with_return_async(start_instrument_ids + start_ib_contracts)

    async def load_ids_async(
        self,
        instrument_ids: list[InstrumentId],
        filters: dict | None = None,
    ) -> None:
        await self.load_ids_with_return_async(
            instrument_ids,
            filters,
        )

    async def load_ids_with_return_async(
        self,
        instrument_ids: list[InstrumentId],
        filters: dict | None = None,
    ) -> list[InstrumentId]:
        """
        加载给定 ID 的工具，并返回成功加载工具的工具 ID。
        """
        loaded_instrument_ids = []
        for instrument_id in instrument_ids:
            loaded_ids = await self.load_with_return_async(
                instrument_id,
                filters,
            )
            if loaded_ids:
                loaded_instrument_ids.extend(loaded_ids)

        return loaded_instrument_ids

    async def load_async(
        self,
        instrument_id: InstrumentId,
        filters: dict | None = None,
    ) -> None:
        await self.load_with_return_async(instrument_id, filters)

    async def load_with_return_async(
        self,
        instrument_id: InstrumentId | IBContract,
        filters: dict | None = None,
    ) -> list[InstrumentId] | None:
        """
        搜索并加载给定 IBContract 的工具。
 
        这是返回值的原始实现。
 
        """
        contract_details: list | None = None
        if isinstance(instrument_id, InstrumentId):
            venue = instrument_id.venue.value

            if await self.fetch_instrument_id(instrument_id, filters):
                return [instrument_id]  # 如果成功获取，则返回工具 ID
            else:
                return None
        elif isinstance(instrument_id, IBContract):
            contract = instrument_id

            contract_details = await self.get_contract_details(contract)
            if contract_details:
                full_contract = contract_details[0].contract
                venue = self.determine_venue_from_contract(full_contract)
        else:
            self._log.error(f"预期为 InstrumentId 或 IBContract，收到了 {instrument_id}")
            return None

        force_instrument_update = (
            filters.get("force_instrument_update", False) if filters else False
        )
        if contract_details:
            return self._process_contract_details(contract_details, venue, force_instrument_update)
        else:
            self._log.error(
                f"无法解析 {instrument_id!r} 的合约详情。"
                f"如果您认为 InstrumentId 或 IBContract 是正确的，请验证其在 "
                f"Interactive Brokers 的 TWS (Trader Workstation) 中的可交易性。",
            )
            return None

    async def fetch_instrument_id(
        self,
        instrument_id: InstrumentId,
        filters: dict | None = None,
    ) -> bool:
        if instrument_id in self.contract:
            return True

        # 对价差工具进行特殊处理
        if is_generic_spread_id(instrument_id):
            return await self._fetch_spread_instrument(instrument_id, filters)

        venue = instrument_id.venue.value
        force_instrument_update = (
            filters.get("force_instrument_update", False) if filters else False
        )

        # 尝试快速构建合约详情（如果它们已存在于工具中）
        if (
            (instrument := self._client._cache.instrument(instrument_id))
            and not force_instrument_update
            and instrument.info
            and instrument.info.get("contract")
        ):
            converted_contract_details = dict_to_contract_details(instrument.info)
            processed_ids = self._process_contract_details([converted_contract_details], venue)

            return bool(processed_ids)  # 如果处理了任何工具，则返回 True

        # VENUE_MEMBERS 将一个 MIC 交易场所关联到多个可能的 IB 交易所
        possible_exchanges = VENUE_MEMBERS.get(venue, [venue])
        try:
            for exchange in possible_exchanges:
                contract = instrument_id_to_ib_contract(
                    instrument_id=instrument_id,
                    exchange=exchange,
                    symbology_method=self.config.symbology_method,
                    contract_details_map=self.contract_details,
                )

                self._log.info(f"正在尝试查找工具 {contract=}")

                contract_details: list = await self.get_contract_details(contract)
                if contract_details:
                    processed_ids = self._process_contract_details(
                        contract_details,
                        venue,
                        force_instrument_update,
                    )
                    return bool(processed_ids)  # 如果处理了任何工具，则返回 True
        except ValueError as e:
            self._log.error(str(e))

        return False

    async def _fetch_spread_instrument(
        self,
        spread_instrument_id: InstrumentId,
        filters: dict | None = None,
    ) -> bool:
        # 通过解析其 ID、加载单个腿、创建 BAG 合约、查询 BAG 详情以获取报价单位，并创建价差工具来获取价差工具。
        try:
            # 解析价差 ID 以获取单个腿
            leg_tuples = generic_spread_id_to_list(spread_instrument_id)
            if not leg_tuples:
                self._log.error(f"价差工具 {spread_instrument_id} 没有腿")
                return False

            self._log.info(
                f"正在加载具有 {len(leg_tuples)} 个腿的价差工具 {spread_instrument_id}",
            )

            # 首先，加载所有单个腿工具以获取其合约详情
            leg_contract_details = []
            for leg_instrument_id, ratio in leg_tuples:
                self._log.info(f"正在加载腿工具：{leg_instrument_id} (比例: {ratio})")

                # 加载单个腿工具
                leg_loaded = await self.fetch_instrument_id(leg_instrument_id, filters)
                if not leg_loaded:
                    self._log.error(f"加载腿工具失败：{leg_instrument_id}")
                    return False

                # 获取此腿的合约详情
                if leg_instrument_id not in self.contract_details:
                    self._log.error(
                        f"在合约详情中未找到腿工具 {leg_instrument_id}",
                    )
                    return False

                leg_details = self.contract_details[leg_instrument_id]
                leg_contract_details.append((leg_details, ratio))

            exchange = filters.get("exchange", "") if filters else ""
            bag_contract = await self._create_bag_contract(
                leg_contract_details,
                spread_instrument_id,
                exchange=exchange,
            )

            # 使用通用的价差创建逻辑
            self._create_spread_instrument(
                spread_instrument_id,
                leg_contract_details,
                bag_contract,
            )

            return True
        except Exception as e:
            self._log.error(f"获取价差工具 {spread_instrument_id} 失败: {e}")
            return False

    async def _create_bag_contract(
        self,
        leg_contract_details: list[tuple[IBContractDetails, int]],
        instrument_id: InstrumentId | None = None,
        bag_contract: IBContract | None = None,
        exchange: str = "",
    ) -> IBContract:
        # 从腿详情创建 BAG 合约
        if bag_contract is None:
            combo_legs = []
            for leg_details, ratio in leg_contract_details:
                action = "BUY" if ratio > 0 else "SELL"
                abs_ratio = abs(ratio)
                combo_leg = ComboLeg(
                    conId=leg_details.contract.conId,
                    ratio=abs_ratio,
                    action=action,
                    exchange=leg_details.contract.exchange,
                )
                combo_legs.append(combo_leg)

            # 使用第一个腿的基础证券代码
            first_contract = leg_contract_details[0][0].contract
            underlying_symbol = getattr(first_contract, "symbol", "ES")

            # 除非明确提供交易所，否则使用 SMART
            if not exchange:
                exchange = "SMART"

            bag_contract = IBContract(
                secType="BAG",
                symbol=underlying_symbol,
                exchange=exchange,
                currency=first_contract.currency,
                comboLegs=combo_legs,
                comboLegsDescrip=(
                    f"价差: {instrument_id.symbol.value}" if instrument_id else "价差"
                ),
            )

        return bag_contract

    def _create_spread_instrument(
        self,
        instrument_id: InstrumentId,
        leg_contract_details: list[tuple[IBContractDetails, int]],
        bag_contract: IBContract,
    ) -> Instrument:
        # 从腿详情创建价差工具（OptionSpread 或 FuturesSpread）。
        # 根据腿的证券类型确定类型，使用第一个腿的 minTick 作为报价单位
        # (IB 不支持 BAG 合约的合约详情)。
        # 检查是否有任何腿是期货
        has_future = any(
            leg_details.contract.secType in ("FUT", "CONTFUT")
            for leg_details, _ in leg_contract_details
        )

        # 创建价差工具
        if has_future:
            spread_instrument = parse_futures_spread_instrument_id(
                instrument_id,
                leg_contract_details,
                self._clock.timestamp_ns(),
            )
        else:
            spread_instrument = parse_option_spread_instrument_id(
                instrument_id,
                leg_contract_details,
                self._clock.timestamp_ns(),
            )

        # 添加到提供程序
        self.add(spread_instrument)

        # 同时添加到客户端缓存
        if not self._client._cache.instrument(spread_instrument.id):
            self._client._cache.add_instrument(spread_instrument)

        # 存储合约映射
        self.contract[instrument_id] = bag_contract
        self.contract_id_to_instrument_id[bag_contract.conId] = instrument_id

        self._log.info(f"成功创建价差工具：{spread_instrument}")

        return spread_instrument

    async def get_contract_details(
        self,
        contract: IBContract,
    ) -> list[ContractDetails]:
        try:
            details = await self._client.get_contract_details(contract=contract)
            if not details:
                self._log.debug(f"未返回 {contract} 的合约详情")
                return []

            [qualified] = details
            self._log.info(
                f"合约已限定为 {qualified.contract.localSymbol}。"
                f"{qualified.contract.primaryExchange or qualified.contract.exchange} "
                f"ConId={qualified.contract.conId}",
            )
            self._log.debug(f"获取到 {details=}")
        except ValueError as e:
            self._log.debug(f"在给定的参数 {contract} 中未找到合约详情, {e}")
            return []

        min_expiry_days = contract.min_expiry_days or self._min_expiry_days or 0
        max_expiry_days = contract.max_expiry_days or self._max_expiry_days or 90

        utc_now = self._clock.utc_now()
        min_expiry = utc_now + pd.Timedelta(days=min_expiry_days)
        max_expiry = utc_now + pd.Timedelta(days=max_expiry_days)

        if (
            contract.secType == "CONTFUT"
            and (contract.build_futures_chain or contract.build_options_chain)
        ) or (self._build_futures_chain or self._build_options_chain):
            # 返回具有期货链的底层合约详情
            future_chain_details = await self.get_future_chain_details(qualified.contract)
            details.extend(future_chain_details)

        if (
            contract.secType in ["STK", "CONTFUT", "FUT", "IND"] and contract.build_options_chain
        ) or self._build_options_chain:
            # 返回具有期权链的底层合约详情，如果适用，也包括期货链
            for detail in set(details):
                if detail.contract.secType == "CONTFUT":
                    continue

                if contract.lastTradeDateOrContractMonth:
                    option_contracts_detail = await self.get_option_chain_details_by_expiry(
                        underlying=detail.contract,
                        last_trading_date=contract.lastTradeDateOrContractMonth,
                        exchange=contract.options_chain_exchange or contract.exchange,
                    )
                else:
                    option_contracts_detail = await self.get_option_chain_details_by_range(
                        underlying=detail.contract,
                        min_expiry=min_expiry,
                        max_expiry=max_expiry,
                        exchange=contract.options_chain_exchange or contract.exchange,
                    )

                details.extend(option_contracts_detail)

        return details

    async def get_future_chain_details(self, underlying: IBContract) -> list[ContractDetails]:
        self._log.info(f"正在为 {underlying.symbol}.{underlying.exchange} 构建期货链")
        details = await self._client.get_contract_details(
            IBContract(
                secType="FUT",
                symbol=underlying.symbol,
                exchange=underlying.exchange,
                tradingClass=underlying.tradingClass,
                includeExpired=True,
            ),
        )
        self._log.debug(f"获取到 {details=}")

        return details

    async def get_option_chain_details_by_range(
        self,
        underlying: IBContract,
        min_expiry: pd.Timestamp,
        max_expiry: pd.Timestamp,
        exchange: str | None = None,
    ) -> list[ContractDetails]:
        chains = await self._client.get_option_chains(underlying)
        if not chains:
            self._log.warning(
                f"对于 {underlying.symbol}.{underlying.exchange} 且到期日为 {underlying.lastTradeDateOrContractMonth} 的合约，没有可用的期权链",
            )
            return []

        details = []
        filtered_chains = [chain for chain in chains if chain[0] == (exchange or "SMART")]
        for chain in filtered_chains:
            expirations = sorted(
                exp for exp in chain[1] if (min_expiry <= pd.Timestamp(exp, tz="UTC") <= max_expiry)
            )
            for expiration in expirations:
                option_contracts_detail = await self.get_option_chain_details_by_expiry(
                    underlying=underlying,
                    last_trading_date=expiration,
                    exchange=exchange,
                )
                details.extend(option_contracts_detail)

        return details

    async def get_option_chain_details_by_expiry(
        self,
        underlying: IBContract,
        last_trading_date: str,
        exchange: str,
    ) -> list[ContractDetails]:
        option_details_result = await self._client.get_contract_details(
            IBContract(
                secType=("FOP" if underlying.secType == "FUT" else "OPT"),
                symbol=underlying.symbol,
                lastTradeDateOrContractMonth=last_trading_date,
                exchange=exchange,
            ),
        )

        if not option_details_result:
            self._log.warning(
                f"未找到 {underlying.symbol} 在 {last_trading_date} 到期的期权合约",
            )
            return []

        # 处理单列表和嵌套列表的情况
        if len(option_details_result) == 1 and isinstance(option_details_result[0], list):
            option_details = option_details_result[0]
        else:
            option_details = option_details_result  # type: ignore[assignment]

        if option_details is None:
            self._log.warning(
                f"对于 {underlying.symbol} 在 {last_trading_date} 到期的期权详情为 None",
            )
            return []

        option_details = [d for d in option_details if d.underConId == underlying.conId]  # type: ignore[assignment]
        self._log.info(
            f"收到了 {len(option_details)} 个 {underlying.symbol}.{underlying.primaryExchange or underlying.exchange} "
            f"在 {last_trading_date} 到期的期权合约",
        )
        self._log.debug(f"获取到 {option_details=}")

        return option_details

    def determine_venue_from_contract(self, contract: IBContract) -> str:  # noqa: C901
        """
        根据工具提供者配置逻辑确定合约的交易场所。
 
        Parameters
        ----------
        contract : IBContract
            要确定交易场所的合约。
 
        Returns
        -------
        str
            确定的交易场所。
 
        """
        if contract.secType == "CFD":
            return "IBCFD"

        if contract.secType == "CMDTY":
            return "IBCMDTY"

        # 使用合约中的交易所
        if contract.exchange == "SMART" and contract.primaryExchange:
            exchange = contract.primaryExchange
        else:
            exchange = contract.exchange
        venue = None

        if self._convert_exchange_to_mic_venue:
            # 首先检查特定证券代码的交易场所映射
            if self._symbol_to_mic_venue:
                for symbol_prefix, symbol_venue in self._symbol_to_mic_venue.items():
                    if contract.symbol.startswith(symbol_prefix):
                        venue = symbol_venue
                        break

            # 如果未找到特定证券代码的映射，使用 VENUE_MEMBERS 映射
            if not venue:
                for venue_member, exchanges in VENUE_MEMBERS.items():
                    if exchange in exchanges:
                        venue = venue_member
                        break

        # 回退到使用交易所作为交易场所
        if not venue:
            venue = exchange

        return venue

    def _process_contract_details(
        self,
        contract_details: list[ContractDetails],
        venue: str,
        force_instrument_update: bool = False,
    ) -> list[InstrumentId]:
        """
        处理合约详情，并返回成功处理的合约的工具 ID。
 
        Parameters
        ----------
        contract_details : list[ContractDetails]
            要处理的合约详情。
        venue : str
            合约的交易场所。
        force_instrument_update : bool, optional
            是否强制更新现有工具。
 
        Returns
        -------
        list[InstrumentId]
            成功处理的合约的工具 ID。
 
        """
        processed_instrument_ids = []
        for details in copy.deepcopy(contract_details):
            if not isinstance(details.contract, IBContract):
                details.contract = IBContract(**details.contract.__dict__)

            if not isinstance(details, IBContractDetails):
                details = IBContractDetails(**details.__dict__)

            sec_type = details.contract.secType
            if self._is_filtered_sec_type(sec_type):
                self._log.warning(
                    f"正在跳过合约 {details.contract} 的已过滤 {sec_type=}",
                )
                continue

            self._log.debug(f"正在尝试从 {details} 创建工具")

            try:
                instrument: Instrument = parse_instrument(
                    details,
                    venue,
                    self.config.symbology_method,
                )
            except ValueError as e:
                self._log.error(f"{self.config.symbology_method=} 解析 {details=} 失败, {e}")
                continue

            if self.config.filter_callable is not None:
                filter_callable = resolve_path(self.config.filter_callable)
                if not filter_callable(instrument):
                    continue

            self._log.info(f"正在从 InteractiveBrokersInstrumentProvider 添加 {instrument=}")

            self.add(instrument)

            if not self._client._cache.instrument(instrument.id) or force_instrument_update:
                self._client._cache.add_instrument(instrument)

            self.contract[instrument.id] = details.contract
            self.contract_details[instrument.id] = details
            self.contract_id_to_instrument_id[details.contract.conId] = instrument.id

            # 添加到成功处理的工具 ID 列表
            processed_instrument_ids.append(instrument.id)

        return processed_instrument_ids
