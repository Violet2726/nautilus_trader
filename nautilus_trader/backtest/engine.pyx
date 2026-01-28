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

import heapq
import pickle
import uuid
from collections import deque
from decimal import Decimal
from heapq import heappush
from typing import Generator

import cython
import pandas as pd

from nautilus_trader.accounting.error import AccountError
from nautilus_trader.backtest.results import BacktestResult
from nautilus_trader.common.component import is_logging_pyo3
from nautilus_trader.common.config import InvalidConfiguration
from nautilus_trader.config import BacktestEngineConfig
from nautilus_trader.core import nautilus_pyo3
from nautilus_trader.core.inspect import is_nautilus_class
from nautilus_trader.core.rust.model import OtoTriggerMode
from nautilus_trader.data.engine import TimeRangeGenerator
from nautilus_trader.data.engine import get_time_range_generator
from nautilus_trader.model import BOOK_DATA_TYPES
from nautilus_trader.model import NAUTILUS_PYO3_DATA_TYPES
from nautilus_trader.system.kernel import NautilusKernel
from nautilus_trader.trading.trader import Trader

from cpython.datetime cimport timedelta
from cpython.object cimport PyObject
from libc.stdint cimport uint8_t
from libc.stdint cimport uint32_t
from libc.stdint cimport uint64_t

from nautilus_trader.accounting.accounts.base cimport Account
from nautilus_trader.accounting.margin_models cimport LeveragedMarginModel
from nautilus_trader.accounting.margin_models cimport MarginModel
from nautilus_trader.backtest.data_client cimport BacktestDataClient
from nautilus_trader.backtest.data_client cimport BacktestMarketDataClient
from nautilus_trader.backtest.execution_client cimport BacktestExecClient
from nautilus_trader.backtest.models cimport FeeModel
from nautilus_trader.backtest.models cimport FillModel
from nautilus_trader.backtest.models cimport LatencyModel
from nautilus_trader.backtest.models cimport MakerTakerFeeModel
from nautilus_trader.backtest.modules cimport SimulationModule
from nautilus_trader.cache.base cimport CacheFacade
from nautilus_trader.common.actor cimport Actor
from nautilus_trader.common.component cimport FORCE_STOP
from nautilus_trader.common.component cimport LOGGING_PYO3
from nautilus_trader.common.component cimport LogColor
from nautilus_trader.common.component cimport Logger
from nautilus_trader.common.component cimport LogGuard
from nautilus_trader.common.component cimport MessageBus
from nautilus_trader.common.component cimport TestClock
from nautilus_trader.common.component cimport TimeEvent
from nautilus_trader.common.component cimport flush_logger
from nautilus_trader.common.component cimport get_component_clocks
from nautilus_trader.common.component cimport is_logging_initialized
from nautilus_trader.common.component cimport log_sysinfo
from nautilus_trader.common.component cimport set_backtest_force_stop
from nautilus_trader.common.component cimport set_logging_clock_realtime_mode
from nautilus_trader.common.component cimport set_logging_clock_static_mode
from nautilus_trader.common.component cimport set_logging_clock_static_time
from nautilus_trader.core.correctness cimport Condition
from nautilus_trader.core.data cimport Data
from nautilus_trader.core.datetime cimport format_iso8601
from nautilus_trader.core.datetime cimport format_optional_iso8601
from nautilus_trader.core.datetime cimport maybe_dt_to_unix_nanos
from nautilus_trader.core.datetime cimport unix_nanos_to_dt
from nautilus_trader.core.rust.backtest cimport TimeEventAccumulator_API
from nautilus_trader.core.rust.backtest cimport time_event_accumulator_advance_clock
from nautilus_trader.core.rust.backtest cimport time_event_accumulator_drop
from nautilus_trader.core.rust.backtest cimport time_event_accumulator_new
from nautilus_trader.core.rust.backtest cimport time_event_accumulator_peek_next_time
from nautilus_trader.core.rust.backtest cimport time_event_accumulator_pop_next_at_or_before
from nautilus_trader.core.rust.common cimport TimeEventHandler_t
from nautilus_trader.core.rust.common cimport logging_is_colored
from nautilus_trader.core.rust.common cimport time_event_handler_drop
from nautilus_trader.core.rust.common cimport vec_time_event_handlers_drop
from nautilus_trader.core.rust.core cimport CVec
from nautilus_trader.core.rust.model cimport FIXED_PRECISION
from nautilus_trader.core.rust.model cimport AccountType
from nautilus_trader.core.rust.model cimport AggregationSource
from nautilus_trader.core.rust.model cimport AggressorSide
from nautilus_trader.core.rust.model cimport BookAction
from nautilus_trader.core.rust.model cimport BookType
from nautilus_trader.core.rust.model cimport ContingencyType
from nautilus_trader.core.rust.model cimport InstrumentCloseType
from nautilus_trader.core.rust.model cimport LiquiditySide
from nautilus_trader.core.rust.model cimport MarketStatus
from nautilus_trader.core.rust.model cimport MarketStatusAction
from nautilus_trader.core.rust.model cimport OmsType
from nautilus_trader.core.rust.model cimport OrderSide
from nautilus_trader.core.rust.model cimport OrderStatus
from nautilus_trader.core.rust.model cimport OrderType
from nautilus_trader.core.rust.model cimport OtoTriggerMode
from nautilus_trader.core.rust.model cimport Price_t
from nautilus_trader.core.rust.model cimport PriceRaw
from nautilus_trader.core.rust.model cimport PriceType
from nautilus_trader.core.rust.model cimport QuantityRaw
from nautilus_trader.core.rust.model cimport TimeInForce
from nautilus_trader.core.rust.model cimport orderbook_best_ask_price
from nautilus_trader.core.rust.model cimport orderbook_best_bid_price
from nautilus_trader.core.rust.model cimport orderbook_has_ask
from nautilus_trader.core.rust.model cimport orderbook_has_bid
from nautilus_trader.core.rust.model cimport trade_id_new
from nautilus_trader.core.string cimport pystr_to_cstr
from nautilus_trader.core.uuid cimport UUID4
from nautilus_trader.data.messages cimport DataCommand
from nautilus_trader.data.messages cimport DataResponse
from nautilus_trader.data.messages cimport SubscribeData
from nautilus_trader.data.messages cimport SubscribeInstruments
from nautilus_trader.data.messages cimport UnsubscribeData
from nautilus_trader.data.messages cimport UnsubscribeInstruments
from nautilus_trader.execution.algorithm cimport ExecAlgorithm
from nautilus_trader.execution.matching_core cimport MatchingCore
from nautilus_trader.execution.messages cimport BatchCancelOrders
from nautilus_trader.execution.messages cimport CancelAllOrders
from nautilus_trader.execution.messages cimport CancelOrder
from nautilus_trader.execution.messages cimport ModifyOrder
from nautilus_trader.execution.messages cimport SubmitOrder
from nautilus_trader.execution.messages cimport SubmitOrderList
from nautilus_trader.execution.messages cimport TradingCommand
from nautilus_trader.execution.trailing cimport TrailingStopCalculator
from nautilus_trader.model.book cimport OrderBook
from nautilus_trader.model.data cimport Bar
from nautilus_trader.model.data cimport BarType
from nautilus_trader.model.data cimport CustomData
from nautilus_trader.model.data cimport InstrumentClose
from nautilus_trader.model.data cimport InstrumentStatus
from nautilus_trader.model.data cimport OrderBookDelta
from nautilus_trader.model.data cimport OrderBookDeltas
from nautilus_trader.model.data cimport OrderBookDepth10
from nautilus_trader.model.data cimport QuoteTick
from nautilus_trader.model.data cimport TradeTick
from nautilus_trader.model.data cimport compute_bar_quarter_sizes
from nautilus_trader.model.events.order cimport OrderAccepted
from nautilus_trader.model.events.order cimport OrderCanceled
from nautilus_trader.model.events.order cimport OrderCancelRejected
from nautilus_trader.model.events.order cimport OrderExpired
from nautilus_trader.model.events.order cimport OrderFilled
from nautilus_trader.model.events.order cimport OrderModifyRejected
from nautilus_trader.model.events.order cimport OrderRejected
from nautilus_trader.model.events.order cimport OrderTriggered
from nautilus_trader.model.events.order cimport OrderUpdated
from nautilus_trader.model.functions cimport account_type_to_str
from nautilus_trader.model.functions cimport aggressor_side_to_str
from nautilus_trader.model.functions cimport book_type_to_str
from nautilus_trader.model.functions cimport oms_type_to_str
from nautilus_trader.model.functions cimport order_type_to_str
from nautilus_trader.model.functions cimport time_in_force_to_str
from nautilus_trader.model.identifiers cimport AccountId
from nautilus_trader.model.identifiers cimport ClientId
from nautilus_trader.model.identifiers cimport ClientOrderId
from nautilus_trader.model.identifiers cimport InstrumentId
from nautilus_trader.model.identifiers cimport PositionId
from nautilus_trader.model.identifiers cimport StrategyId
from nautilus_trader.model.identifiers cimport TradeId
from nautilus_trader.model.identifiers cimport TraderId
from nautilus_trader.model.identifiers cimport Venue
from nautilus_trader.model.identifiers cimport VenueOrderId
from nautilus_trader.model.instruments.base cimport EXPIRING_INSTRUMENT_CLASSES
from nautilus_trader.model.instruments.base cimport Instrument
from nautilus_trader.model.instruments.crypto_future cimport CryptoFuture
from nautilus_trader.model.instruments.crypto_perpetual cimport CryptoPerpetual
from nautilus_trader.model.instruments.currency_pair cimport CurrencyPair
from nautilus_trader.model.instruments.equity cimport Equity
from nautilus_trader.model.objects cimport AccountBalance
from nautilus_trader.model.objects cimport Currency
from nautilus_trader.model.objects cimport Money
from nautilus_trader.model.objects cimport Price
from nautilus_trader.model.objects cimport Quantity
from nautilus_trader.model.orders.base cimport Order
from nautilus_trader.model.orders.limit cimport LimitOrder
from nautilus_trader.model.orders.limit_if_touched cimport LimitIfTouchedOrder
from nautilus_trader.model.orders.market cimport MarketOrder
from nautilus_trader.model.orders.market_if_touched cimport MarketIfTouchedOrder
from nautilus_trader.model.orders.market_to_limit cimport MarketToLimitOrder
from nautilus_trader.model.orders.stop_limit cimport StopLimitOrder
from nautilus_trader.model.orders.stop_market cimport StopMarketOrder
from nautilus_trader.model.position cimport Position
from nautilus_trader.portfolio.base cimport PortfolioFacade
from nautilus_trader.trading.strategy cimport Strategy


cdef class BacktestEngine:
    """
    提供回测引擎，通过历史数据运行策略组合。

    Parameters
    ----------
    config : BacktestEngineConfig, optional
        实例的配置。

    Raises
    ------
    TypeError
        如果 `config` 不是 `BacktestEngineConfig` 类型。
    """

    def __init__(self, config: BacktestEngineConfig | None = None) -> None:
        if config is None:
            config = BacktestEngineConfig()

        Condition.type(config, BacktestEngineConfig, "config")

        self._config: BacktestEngineConfig  = config

        # 设置组件
        self._accumulator = <TimeEventAccumulator_API>time_event_accumulator_new()
 
        # 运行 ID
        self._run_config_id: str | None = None
        self._run_id: UUID4 | None = None
 
        # 场所和数据
        self._venues: dict[Venue, SimulatedExchange] = {}
        self._has_data: set[InstrumentId] = set()
        self._has_book_data: set[InstrumentId] = set()
        self._data: list[Data] = []
        self._data_len: uint64_t = 0
        self._iteration: uint64_t = 0
        self._last_ns : uint64_t = 0
        self._end_ns : uint64_t = 0
        self._sorted: bint = True
 
        # 计时
        self._run_started: pd.Timestamp | None = None
        self._run_finished: pd.Timestamp | None = None
        self._backtest_start: pd.Timestamp | None = None
        self._backtest_end: pd.Timestamp | None = None
 
        # 构建核心系统内核
        self._kernel = NautilusKernel(name=type(self).__name__, config=config)
        self._instance_id = self._kernel.instance_id
        self._log = Logger(type(self).__name__)
 
        self._data_engine: DataEngine = self._kernel.data_engine
 
        # 设置数据迭代器
        self._data_requests: dict[str, RequestData] = {}
        self._last_subscription_ts: dict[str, uint64_t] = {}
        self._backtest_subscription_names = set()
        self._response_data = []
        self._data_iterator = BacktestDataIterator()
        self._kernel.msgbus.register(endpoint="BacktestEngine.execute", handler=self._handle_data_command)

    def __del__(self) -> None:
        if self._accumulator._0 != NULL:
            time_event_accumulator_drop(self._accumulator)

    @property
    def trader_id(self) -> TraderId:
        """
        返回引擎的交易员 ID。

        Returns
        -------
        TraderId

        """
        return self._kernel.trader_id

    @property
    def machine_id(self) -> str:
        """
        返回引擎的机器 ID。

        Returns
        -------
        str

        """
        return self._kernel.machine_id

    @property
    def instance_id(self) -> UUID4:
        """
        返回引擎的实例 ID。

        这是每个初始化引擎的唯一标识符。

        Returns
        -------
        UUID4

        """
        return self._kernel.instance_id

    @property
    def kernel(self) -> NautilusKernel:
        """
        返回引擎的内部内核。

        Returns
        -------
        NautilusKernel

        """
        return self._kernel

    @property
    def logger(self) -> Logger:
        """
        返回引擎的内部日志记录器。

        Returns
        -------
        Logger

        """
        return self._log

    @property
    def run_config_id(self) -> str:
        """
        返回最后一次回测引擎运行配置 ID。

        Returns
        -------
        str or ``None``

        """
        return self._run_config_id

    @property
    def run_id(self) -> UUID4:
        """
        返回最后一次回测引擎运行 ID（如果已运行）。

        Returns
        -------
        UUID4 or ``None``

        """
        return self._run_id

    @property
    def iteration(self) -> int:
        """
        返回回测引擎迭代计数。

        Returns
        -------
        int

        """
        return self._iteration

    @property
    def run_started(self) -> pd.Timestamp | None:
        """
        返回最后一次回测运行开始的时间（如果已运行）。

        Returns
        -------
        pd.Timestamp or ``None``

        """
        return self._run_started

    @property
    def run_finished(self) -> pd.Timestamp | None:
        """
        返回最后一次回测运行结束的时间（如果已运行）。

        Returns
        -------
        pd.Timestamp or ``None``

        """
        return self._run_finished

    @property
    def backtest_start(self) -> pd.Timestamp | None:
        """
        返回最后一次回测运行时间范围的开始（如果已运行）。

        Returns
        -------
        pd.Timestamp or ``None``

        """
        return self._backtest_start

    @property
    def backtest_end(self) -> pd.Timestamp | None:
        """
        返回最后一次回测运行时间范围的结束（如果已运行）。

        Returns
        -------
        pd.Timestamp or ``None``

        """
        return self._backtest_end

    @property
    def trader(self) -> Trader:
        """
        返回引擎的内部交易员。

        Returns
        -------
        Trader

        """
        return self._kernel.trader

    @property
    def cache(self) -> CacheFacade:
        """
        返回引擎的内部只读缓存。

        Returns
        -------
        CacheFacade

        """
        return self._kernel.cache

    @property
    def data(self) -> list[Data]:
        """
        返回引擎的内部数据流。

        Returns
        -------
        list[Data]

        """
        return self._data.copy()

    @property
    def portfolio(self) -> PortfolioFacade:
        """
        返回引擎的内部只读投资组合。

        Returns
        -------
        PortfolioFacade

        """
        return self._kernel.portfolio

    def get_log_guard(self) -> nautilus_pyo3.LogGuard | LogGuard | None:
        """
        返回全局日志子系统的日志守卫。

        如果日志子系统已初始化，可能返回 ``None``。

        Returns
        -------
        nautilus_pyo3.LogGuard | LogGuard | None

        """
        return self._kernel.get_log_guard()

    def list_venues(self) -> list[Venue]:
        """
        返回引擎中包含的场所。

        Returns
        -------
        list[Venue]

        """
        return list(self._venues)

    def add_venue(
        self,
        venue: Venue,
        oms_type: OmsType,
        account_type: AccountType,
        starting_balances: list[Money],
        base_currency: Currency | None = None,
        default_leverage: Decimal | None = None,
        leverages: dict[InstrumentId, Decimal] | None = None,
        margin_model: MarginModel = None,
        modules: list[SimulationModule] | None = None,
        fill_model: FillModel | None = None,
        fee_model: FeeModel | None = None,
        latency_model: LatencyModel | None = None,
        book_type: BookType = BookType.L1_MBP,
        routing: bool = False,
        reject_stop_orders: bool = True,
        support_gtd_orders: bool = True,
        support_contingent_orders: bool = True,
        oto_trigger_mode: OtoTriggerMode = OtoTriggerMode.PARTIAL,
        use_position_ids: bool = True,
        use_random_ids: bool = False,
        use_reduce_only: bool = True,
        use_message_queue: bool = True,
        use_market_order_acks: bool = False,
        bar_execution: bool = True,
        bar_adaptive_high_low_ordering: bool = False,
        trade_execution: bool = False,
        liquidity_consumption: bool = False,
        allow_cash_borrowing: bool = False,
        frozen_account: bool = False,
        price_protection_points=None,
    ) -> None:
        """
        向回测引擎添加具有给定参数的 `SimulatedExchange`。

        Parameters
        ----------
        venue : Venue
            场所 ID。
        oms_type : OmsType {``HEDGING``, ``NETTING``}
            交易所的订单管理系统类型。如果为 ``HEDGING``，将生成新的持仓 ID。
        account_type : AccountType
            交易所的账户类型。
        starting_balances : list[Money]
            账户期初余额（单资产账户指定一个）。
        base_currency : Currency, optional
            客户的账户基础货币。对于多币种账户，使用 ``None``。
        default_leverage : Decimal, optional
            账户默认杠杆（用于保证金账户）。
        leverages : dict[InstrumentId, Decimal], optional
            特定工具的杠杆配置（用于保证金账户）。
        margin_model : MarginModelConfig, optional
            保证金计算模型配置。默认为 'leveraged'。
        modules : list[SimulationModule], optional
            要加载到交易所的模拟模块。
        fill_model : FillModel, optional
            交易所的成交模型。
        fee_model : FeeModel, optional
            场所的费用模型。
        latency_model : LatencyModel, optional
            交易所的延迟模型。
        book_type : BookType, default ``BookType.L1_MBP``
            默认订单簿类型。
        routing : bool, default False
            是否应为执行客户端启用多场所路由。
        reject_stop_orders : bool, default True
            如果提交时触发价格在市场价格范围内，是否拒绝止损单。
        support_gtd_orders : bool, default True
            场所是否支持 GTD（Good Till Date）有效时间的订单。
        support_contingent_orders : bool, default True
            场所是否支持/遵循条件订单。
            如果为 False，则预期策略将管理任何条件订单。
        oto_trigger_mode : OtoTriggerMode, default ``OtoTriggerMode.PARTIAL``
            条件订单的 OTO 触发模式：
            - ``PARTIAL``：根据每次部分成交按比例释放子订单（默认）。
            - ``FULL``：仅在父订单完全成交后释放子订单。
        use_position_ids : bool, default True
            是否在订单成交时生成场所持仓 ID。
        use_random_ids : bool, default False
            是否所有场所生成的标识符都是随机 UUID4。
        use_reduce_only : bool, default True
            是否遵循订单上的 `reduce_only` 执行指令。
        use_message_queue : bool, default True
            是否应使用内部消息队列按顺序处理交易指令。对于实时沙盒环境，
            将其设置为 False 可能更合适，因为我们不想在处理交易指令之前引入
            等待下一个数据事件的额外延迟。
        use_market_order_acks : bool, default False
            是否在成交前为市价单生成 OrderAccepted 事件。
        bar_execution : bool, default True
            是否应由撮合引擎处理 Bar 数据（并推动市场）。
        bar_adaptive_high_low_ordering : bool, default False
            决定是否根据启发式算法自适应处理 Bar 价格顺序。
            此设置仅在 `bar_execution` 为 True 时相关。
            如果为 False，Bar 价格始终按固定顺序处理：Open, High, Low, Close。
            如果为 True，处理顺序随启发式算法调整：
            - 如果 High 比 Low 更接近 Open，则处理顺序为 Open, High, Low, Close。
            - 如果 Low 比 High 更接近 Open，则处理顺序为 Open, Low, High, Close。
        trade_execution : bool, default False
            是否应由撮合引擎处理 Trade 数据（并推动市场）。
        liquidity_consumption : bool, default False
            是否应按价格水平跟踪流动性消耗。启用时，成交会消耗可用流动性，
            当该水平的新数据到达时重置。禁用时，每次迭代都可以独立地根据
            全额订单簿流动性进行成交。
        allow_cash_borrowing : bool, default False
            现金账户是否允许借贷（负余额）。
        frozen_account : bool, default False
            此交易所的账户是否冻结（余额不会改变）。
        price_protection_points : int, optional
            定义交易所计算的价格边界（以点为单位），以防止市价单在过于激进的价格执行。

        Raises
        ------
        ValueError
            如果 `venue` 已经在引擎中注册。

        """
        if modules is None:
            modules = []
 
        if margin_model is None:
            margin_model = LeveragedMarginModel()
 
        if fill_model is None:
            fill_model = FillModel()
 
        if fee_model is None:
            fee_model = MakerTakerFeeModel()

        Condition.not_none(venue, "venue")
        Condition.not_in(venue, self._venues, "venue", "_venues")
        Condition.not_empty(starting_balances, "starting_balances")
        Condition.list_type(modules, SimulationModule, "modules")
        Condition.type(fill_model, FillModel, "fill_model")
        Condition.type(fee_model, FeeModel, "fee_model")

        if default_leverage is None:
            if account_type == AccountType.MARGIN:
                default_leverage = Decimal(10)
            else:
                default_leverage = Decimal(1)

        exchange = SimulatedExchange(
            venue=venue,
            oms_type=oms_type,
            account_type=account_type,
            starting_balances=starting_balances,
            base_currency=base_currency,
            default_leverage=default_leverage,
            leverages=leverages or {},
            margin_model=margin_model,
            modules=modules,
            portfolio=self._kernel.portfolio,
            msgbus=self._kernel.msgbus,
            cache=self._kernel.cache,
            fill_model=fill_model,
            fee_model=fee_model,
            latency_model=latency_model,
            book_type=book_type,
            clock=self._kernel.clock,
            frozen_account=frozen_account,
            reject_stop_orders=reject_stop_orders,
            support_gtd_orders=support_gtd_orders,
            support_contingent_orders=support_contingent_orders,
            oto_trigger_mode=oto_trigger_mode,
            use_position_ids=use_position_ids,
            use_random_ids=use_random_ids,
            use_reduce_only=use_reduce_only,
            use_message_queue=use_message_queue,
            use_market_order_acks=use_market_order_acks,
            bar_execution=bar_execution,
            bar_adaptive_high_low_ordering=bar_adaptive_high_low_ordering,
            trade_execution=trade_execution,
            liquidity_consumption=liquidity_consumption,
            price_protection_points=price_protection_points,
        )

        self._venues[venue] = exchange

        exec_client = BacktestExecClient(
            exchange=exchange,
            msgbus=self._kernel.msgbus,
            cache=self._kernel.cache,
            clock=self._kernel.clock,
            routing=routing,
            frozen_account=frozen_account,
            allow_cash_borrowing=allow_cash_borrowing,
        )

        exchange.register_client(exec_client)
        self._kernel.exec_engine.register_client(exec_client)

        self._add_market_data_client_if_not_exists(venue)

        self._log.info(f"Added {exchange}")

    def change_fill_model(self, Venue venue, FillModel model) -> None:
        """
        更改给定场所交易所的成交模型。

        Parameters
        ----------
        venue : Venue
            模拟交易所的场所。
        model : FillModel
            要更改为的成交模型。

        """
        Condition.not_none(venue, "venue")
        Condition.not_none(model, "model")
        Condition.is_in(venue, self._venues, "venue", "self._venues")

        self._venues[venue].set_fill_model(model)

    def add_instrument(self, Instrument instrument) -> None:
        """
        将工具添加到回测引擎。

        该工具必须对其关联场所有效。例如，不能将以保证金交易的衍生工具
        添加到具有 ``CASH`` 账户的场所。

        Parameters
        ----------
        instrument : Instrument
            要添加的工具。

        Raises
        ------
        InvalidConfiguration
            如果尚未将 `instrument` 的场所添加到引擎中。
        InvalidConfiguration
            如果 `instrument` 对其关联场所无效。

        """
        Condition.not_none(instrument, "instrument")

        if instrument.id.venue not in self._venues:
            raise InvalidConfiguration(
                "Cannot add an `Instrument` object without first adding its associated venue. "
                f"Add the {instrument.id.venue} venue using the `add_venue` method."
            )

        # 验证工具是否适用于该场所
        cdef SimulatedExchange venue = self._venues[instrument.id.venue]
 
        if (
            isinstance(instrument, CurrencyPair)
            and venue.account_type != AccountType.MARGIN
            and venue.base_currency is not None  # 单币种账户
        ):
            raise InvalidConfiguration(
                f"无法为具有单币种现金账户的场所添加 `CurrencyPair` 工具 {instrument}。",
            )
 
        # 检查客户端是否已注册
        self._add_market_data_client_if_not_exists(instrument.id.venue)
 
        # 添加数据
        self._kernel.data_engine.process(instrument)  # 添加到缓存
        self._venues[instrument.id.venue].add_instrument(instrument)
 
        self._log.info(f"已添加 {instrument.id} 工具")

    def add_data(
        self,
        list data,
        ClientId client_id = None,
        bint validate = True,
        bint sort = True,
    ) -> None:
        """
        将给定的 `data` 添加到回测引擎。

        Parameters
        ----------
        data : list[Data]
            要添加的数据。
        client_id : ClientId, optional
            与数据关联的客户端 ID。
        validate : bool, default True
            如果应验证 `data`（直接向引擎添加数据时建议使用）。
        sort : bool, default True
            如果 `data` 在添加后应与流的其余部分按 `ts_init` 排序
            （直接向引擎添加数据时建议使用）。

        Raises
        ------
        ValueError
            如果 `data` 为空。
        ValueError
            如果 `data` 包含非 `Data` 类型的对象。
        ValueError
            如果在缓存中找不到数据的 `instrument_id`。
        ValueError
            如果 `data` 元素没有 `instrument_id` 且 `client_id` 为 ``None``。
        TypeError
            如果 `data` 是 Rust PyO3 数据类型（尚无法直接添加到引擎）。

        Warnings
        --------
        假设所有数据元素都是相同类型。添加不同数据类型的列表可能会导致不正确的回测逻辑。

        如果添加数据时 `sort` 不为 True，请小心，因为这可能导致在时间戳非
        单调递增的流上运行回测。

        Notes
        -----
        为了加载大型数据集时获得最佳性能，请考虑对所有 `add_data()` 调用
        使用 `sort=False`，然后在添加所有数据后调用一次 `sort_data()`：

        .. code-block:: python

            # Add multiple data streams without sorting
            # 添加多个数据流而不排序
            engine.add_data(instrument1_bars, sort=False)
            engine.add_data(instrument2_bars, sort=False)
            engine.add_data(instrument3_bars, sort=False)

            # Sort once at the end
            # 最后排序一次
            engine.sort_data()

        这种方法避免了在每次调用时重复对整个数据流进行排序，从而显着减少了
        大型数据集的加载时间。

        **合约不变量：**

        - 当 `sort=True` 时：数据可通过 `run()` 立即用于回测。
        - 当 `sort=False` 时：在 `run()` 之前，您 **必须** 调用 `sort_data()` 或使用 `sort=True` 添加数据。
        - 提供的 `data` 列表始终在内部复制，以防止外部突变影响引擎状态。

        """
        Condition.not_empty(data, "data")
        Condition.list_type(data, Data, "data")

        if isinstance(data[0], NAUTILUS_PYO3_DATA_TYPES):
            raise TypeError(
                f"Cannot add data of type `{type(data[0]).__name__}` from pyo3 directly to engine. "
                "This will be supported in a future release.",
            )

        cdef str data_added_str = "data"

        if validate:
            first = data[0]

            if hasattr(first, "instrument_id"):
                Condition.is_true(
                    first.instrument_id in self._kernel.cache.instrument_ids(),
                    f"`Instrument` {first.instrument_id} for the given data not found in the cache. "
                    "Add the instrument through `add_instrument()` prior to adding related data.",
                )
                # Check client has been registered
                self._add_market_data_client_if_not_exists(first.instrument_id.venue)
                self._has_data.add(first.instrument_id)
                data_added_str = f"{first.instrument_id} {type(first).__name__}"
            elif isinstance(first, Bar):
                Condition.is_true(
                    first.bar_type.instrument_id in self._kernel.cache.instrument_ids(),
                    f"`Instrument` {first.bar_type.instrument_id} for the given data not found in the cache. "
                    "Add the instrument through `add_instrument()` prior to adding related data.",
                )
                Condition.equal(
                    first.bar_type.aggregation_source,
                    AggregationSource.EXTERNAL,
                    "bar_type.aggregation_source",
                    "required source",
                )
                self._has_data.add(first.bar_type.instrument_id)
                data_added_str = f"{first.bar_type} {type(first).__name__}"
            else:
                Condition.not_none(client_id, "client_id")
                # Check client has been registered
                self._add_data_client_if_not_exists(client_id)

                if isinstance(first, CustomData):
                    data_added_str = f"{type(first.data).__name__} "

            if type(first) in BOOK_DATA_TYPES:
                self._has_book_data.add(first.instrument_id)
 
        # 添加数据
        self._data.extend(data)

        if sort:
            self._data = sorted(self._data, key=lambda x: x.ts_init)
            self._data_iterator.add_data("backtest_data", self._data, append_data=True, presorted=True)
            self._sorted = True
        else:
            self._sorted = False

        for data_point in data:
            data_type = type(data_point)

            if data_type is Bar:
                self._backtest_subscription_names.add(f"{data_point.bar_type}")
            elif data_type in (QuoteTick, TradeTick):
                self._backtest_subscription_names.add(f"{data_type.__name__}.{data_point.instrument_id}")
            elif data_type is CustomData:
                self._backtest_subscription_names.add(f"{type(data_point.data).__name__}.{getattr(data_point.data, 'instrument_id', None)}")

        self._log.info(
            f"Added {len(data):_} {data_added_str} element{'' if len(data) == 1 else 's'}",
        )

    def add_data_iterator(
        self,
        str data_name,
        generator: Generator[list[Data], None, None],
        ClientId client_id = None,
    ) -> None:
        """
        为底层流式回测 API 添加产生 ``list[Data]`` 对象的单流生成器。

        Parameters
        ----------
        data_name : str
            数据流的名称标识符。
        generator : Generator[list[Data], None, None]
            产生 ``Data`` 对象列表的 Python 生成器。
        client_id : ClientId, optional
            与数据关联的客户端 ID。

        Notes
        -----
        该方法通过分块加载数据来实现大型数据集的流式传输。
        生成器应产生按 `ts_init` 时间戳排序的 ``list[Data]`` 对象。

        """
        self._data_iterator.init_data(
            data_name,
            generator,
            append_data=True
        )

        self._log.info(f"Added {data_name} stream generator")

    cpdef void _handle_data_command(self, DataCommand command):
        if not(command.data_type.type in [Bar, QuoteTick, TradeTick, OrderBookDepth10]
               or type(command) in [SubscribeData, UnsubscribeData, SubscribeInstruments, UnsubscribeInstruments]):
            return

        if isinstance(command, SubscribeData):
            self._handle_subscribe(<SubscribeData>command)
        elif isinstance(command, UnsubscribeData):
            self._handle_unsubscribe(<UnsubscribeData>command)

    cdef void _handle_subscribe(self, SubscribeData command):
        cdef RequestData request = command.to_request(unix_nanos_to_dt(self._last_ns), unix_nanos_to_dt(self._end_ns), self._handle_data_response)
        cdef str subscription_name = request.params["subscription_name"]

        if subscription_name in self._data_requests or subscription_name in self._backtest_subscription_names:
            return

        self._log.debug(f"正在订阅 {subscription_name}，{command.params.get('durations_seconds')=}")
 
        time_range_generator = get_time_range_generator(
            request.params.get("time_range_generator", "")
        )(request)
        cdef bint append_data = request.params.get("append_data", True)
        request.params.pop("time_range_generator", None) # 这样子请求就不会也使用长数据范围请求
 
        self._data_requests[subscription_name] = request
        self._data_iterator.init_data(
            subscription_name,
            self._subscription_generator(
                subscription_name,
                time_range_generator,
            ),
            append_data
        )

    def _subscription_generator(
        self,
        str subscription_name,
        time_range_generator: TimeRangeGenerator,
    ):
        """
        使用时间范围生成器产生产生订阅的回测数据范围的生成器。
        """
        def get_next_time_range(data_received):
            # 获取下一个时间范围的辅助函数，具有适当的错误处理，data_received 是发送到 time_range_generator 的信号，
            # 用于指示在上一次调用 _update_subscription_data 时是否收到数据
            try:
                return time_range_generator.send(data_received) if data_received is not None else next(time_range_generator)
            except StopIteration:
                return None, None
 
        # 获取初始时间范围
        request_start_ns, request_end_ns = get_next_time_range(None)

        try:
            while request_start_ns is not None and request_start_ns <= self._end_ns:
                # 清除并更新响应数据
                self._response_data = []
                self._update_subscription_data(subscription_name, request_start_ns, request_end_ns)
 
                # 根据是否获得数据来确定信号
                data_received = len(self._response_data) > 0
 
                # 如果有数据，则产生数据
                if self._response_data:
                    yield self._response_data
 
                # 获取下一个时间范围
                request_start_ns, request_end_ns = get_next_time_range(data_received)
        finally:
            # 确保生成器被正确关闭
            try:
                time_range_generator.close()
            except (StopIteration, GeneratorExit):
                pass

    cpdef void _update_subscription_data(self, str subscription_name, uint64_t request_start_ns, uint64_t request_end_ns):
        cdef RequestData request = self._data_requests[subscription_name]
        cdef RequestData new_request = request.with_dates(
            unix_nanos_to_dt(request_start_ns),
            unix_nanos_to_dt(request_end_ns),
            self._last_ns,
            self._handle_data_response
        )
        self._log.debug(f"Renewing {request.data_type.type.__name__} data from {unix_nanos_to_dt(request_start_ns)} to {unix_nanos_to_dt(request_end_ns)}")
        self._kernel._msgbus.request(endpoint="DataEngine.request", request=new_request)

    cpdef void _handle_data_response(self, DataResponse response):
        cdef list[Data] data = response.data
        cdef str subscription_name = response.params["subscription_name"]
 
        if not data:
            self._log.debug(f"{subscription_name} 数据为空")
        else:
            self._log.debug(f"已收到订阅 {subscription_name} 数据，从 {unix_nanos_to_dt(data[0].ts_init)} 到 {unix_nanos_to_dt(data[-1].ts_init)}")
 
        self._response_data = data

    cpdef void _handle_unsubscribe(self, UnsubscribeData command):
        cdef str subscription_name = ""

        if command.data_type.type is Bar:
            subscription_name = f"{command.bar_type}"
        elif type(command) is UnsubscribeInstruments:
            subscription_name = "subscribe_instruments"
        else:
            subscription_name = f"{command.data_type.type.__name__}.{command.instrument_id}"

        self._log.debug(f"正在取消订阅 {subscription_name}")
        self._data_iterator.remove_data(subscription_name, complete_remove=True)
        self._data_requests.pop(subscription_name, None)

    def dump_pickled_data(self) -> bytes:
        """
        返回序列化（pickle）的内部数据流。

        Returns
        -------
        bytes

        """
        return pickle.dumps(self._data)

    def load_pickled_data(self, bytes data) -> None:
        """
        将给定的序列化数据直接加载到内部数据流中。

        强烈建议仅将通过调用 `.dump_pickled_data()` 获得的数据传递给此方法。

        Warnings
        --------
        此底层直接访问方法做出以下假设：
         - 数据仅包含有效的 Nautilus 对象，且继承自 `Data`。
         - 数据已通过调用 `pickle.dumps()` 成功序列化。
         - 数据在序列化之前已排序。
         - 所有即将需要的工具都已添加到引擎中。

        """
        Condition.not_none(data, "data")
        self._data = pickle.loads(data)
        self._data_iterator.add_data("backtest_data", self._data, append_data=True, presorted=True)
        self._sorted = True

        self._log.info(
            f"Loaded {len(self._data):_} data "
            f"element{'' if len(data) == 1 else 's'} from pickle",
        )

    def add_actor(self, actor: Actor) -> None:
        """
        将给定的 actor 添加到回测引擎。

        Parameters
        ----------
        actor : Actor
            The actor to add.

        """
        # Checked inside trader
        self._kernel.trader.add_actor(actor)

    def add_actors(self, actors: list[Actor]) -> None:
        """
        将给定的 actors 列表添加到回测引擎。

        Parameters
        ----------
        actors : list[Actor]
            要添加的 actor 列表。
 
        """
        # 在交易员内部检查
        self._kernel.trader.add_actors(actors)

    def add_strategy(self, strategy: Strategy) -> None:
        """
        将给定的策略添加到回测引擎。

        Parameters
        ----------
        strategy : Strategy
            The strategy to add.

        """
        # Checked inside trader
        self._kernel.trader.add_strategy(strategy)

    def add_strategies(self, strategies: list[Strategy]) -> None:
        """
        将给定的策略列表添加到回测引擎。

        Parameters
        ----------
        strategies : list[Strategy]
            要添加的策略列表。
 
        """
        # 在交易员内部检查
        self._kernel.trader.add_strategies(strategies)

    def add_exec_algorithm(self, exec_algorithm: ExecAlgorithm) -> None:
        """
        将给定的执行算法添加到回测引擎。

        Parameters
        ----------
        exec_algorithm : ExecAlgorithm
            要添加的执行算法。
 
        """
        # 在交易员内部检查
        self._kernel.trader.add_exec_algorithm(exec_algorithm)

    def add_exec_algorithms(self, exec_algorithms: list[ExecAlgorithm]) -> None:
        """
        将给定的执行算法列表添加到回测引擎。

        Parameters
        ----------
        exec_algorithms : list[ExecAlgorithm]
            要添加的执行算法列表。
 
        """
        # 在交易员内部检查
        self._kernel.trader.add_exec_algorithms(exec_algorithms)

    def reset(self) -> None:
        """
        重置回测引擎。
 
        所有有状态字段都将重置为其初始值，但数据和工具除外（它们会保留）。
 
        Notes
        -----
        默认情况下，数据和工具在重置后会保留，以便能够针对同一数据集使用不同的策略
        或参数进行重复运行。
 
        See Also
        --------
        https://nautilustrader.io/docs/concepts/backtesting#repeated-runs
 
        """
        self._log.debug(f"正在重置")
 
        if self._kernel.trader.is_running:
            # 结束当前回测运行
            self.end()
 
        # 重置数据引擎 (DataEngine)
        if self._kernel.data_engine.is_running:
            self._kernel.data_engine.stop()
 
        self._kernel.data_engine.reset()
 
        # 重置执行引擎 (ExecEngine)
        if self._kernel.exec_engine.is_running:
            self._kernel.exec_engine.stop()
 
        self._kernel.exec_engine.reset()
 
        # 重置风险引擎 (RiskEngine)
        if self._kernel.risk_engine.is_running:
            self._kernel.risk_engine.stop()
 
        self._kernel.risk_engine.reset()
 
        # 重置仿真器 (Emulator)
        if self._kernel.emulator.is_running:
            self._kernel.emulator.stop()
 
        self._kernel.emulator.reset()
 
        self._kernel.trader.reset()
 
        for exchange in self._venues.values():
            exchange.reset()
 
        # 重置运行 ID
        self._run_config_id = None
        self._run_id = None
 
        # 重置计时
        self._iteration = 0
        self._data_iterator = BacktestDataIterator()
 
        if self._sorted:
            self._data_iterator.add_data("backtest_data", self._data, append_data=True, presorted=True)
 
        self._run_started = None
        self._run_finished = None
        self._backtest_start = None
        self._backtest_end = None
 
        self._log.info("已重置")

    def sort_data(self) -> None:
        """
        对引擎的内部数据流进行排序。

        """
        self._data = sorted(self._data, key=lambda x: x.ts_init)
        self._data_iterator.add_data("backtest_data", self._data, append_data=True, presorted=True)
        self._sorted = True

    def clear_data(self) -> None:
        """
        清除引擎的内部数据流。

        不会清除已添加的工具。

        """
        self._has_data.clear()
        self._has_book_data.clear()
        self._data.clear()
        self._data_len = 0
        self._data_iterator = BacktestDataIterator()
        self._sorted = True

    def clear_actors(self) -> None:
        """
        清除引擎内部交易员的所有 actor。

        """
        self._kernel.trader.clear_actors()

    def clear_strategies(self) -> None:
        """
        清除引擎内部交易员的所有交易策略。

        """
        self._kernel.trader.clear_strategies()

    def clear_exec_algorithms(self) -> None:
        """
        清除引擎内部交易员的所有执行算法。

        """
        self._kernel.trader.clear_exec_algorithms()

    def dispose(self) -> None:
        """
        通过释放交易员和系统资源来销毁回测引擎。

        多次调用此方法的效果与调用一次相同（它是幂等的）。
        一旦调用，它就不能被逆转，并且不应在此实例上调用其他方法。

        """
        self.clear_data()
        self._kernel.dispose()

    def run(
        self,
        start: datetime | str | int | None = None,
        end: datetime | str | int | None = None,
        run_config_id: str | None = None,
        streaming: bool = False,
    ) -> None:
        """
        运行回测。

        运行结束时，交易员和策略将停止，然后执行运行后分析。

        对于大于可用内存的数据集，请使用带有以下顺序的 `streaming` 模式：
        - 1. 添加初始数据批次和策略
        - 2. 调用 `run(streaming=True)`
        - 3. 调用 `clear_data()`
        - 4. 添加下一批数据流
        - 5. 处理最后一批时调用 `run(streaming=False)` 或 `end()`

        Parameters
        ----------
        start : datetime or str or int, optional
            回测运行的开始日期时间（UTC）。
            如果为 ``None``，引擎从数据开始处运行。
        end : datetime or str or int, optional
            回测运行的结束日期时间（UTC）。
            如果为 ``None``，引擎运行到数据结束处。
        run_config_id : str, optional
            标记化的 `BacktestRunConfig` ID。
        streaming : bool, default False
            控制数据加载和处理模式：
            - 如果为 False（默认）：一次加载所有数据。
              这是目前自定义数据（例如期权希腊字母）唯一支持的模式。
            - 如果为 True：按块加载数据，以便内存高效地处理大型数据集。

        Raises
        ------
        ValueError
            如果尚未向引擎添加数据。
        ValueError
            如果 `start` >= `end` 日期时间。
        RuntimeError
            如果使用 `sort=False` 添加了数据，但未调用 `sort_data()`。

        Notes
        -----
        **合约不变量：**

        - 通过 `add_data()` 添加的所有数据必须在调用 `run()` 之前进行排序并同步到内部迭代器。
        - 如果使用 `sort=False` 添加了任何数据，则必须在此方法之前调用 `sort_data()` 或使用 `sort=True` 添加数据。
        - 引擎会验证此要求，并在检测到未排序数据时引发 `RuntimeError`。

        """
        self._run(start, end, run_config_id, streaming)

        if not streaming:
            self.end()

    def end(self):
        """
        手动结束回测。

        Notes
        -----
        仅当您之前一直在使用流式传输运行时才需要。

        """
        if self._kernel.trader.is_running:
            self._kernel.trader.stop()

        if self._kernel.data_engine.is_running:
            self._kernel.data_engine.stop()

        if self._kernel.risk_engine.is_running:
            self._kernel.risk_engine.stop()

        if self._kernel.exec_engine.is_running:
            self._kernel.exec_engine.stop()

        if self._kernel.emulator.is_running:
            self._kernel.emulator.stop()

        try:
            # 处理剩余消息
            for exchange in self._venues.values():
                exchange.process(self._kernel.clock.timestamp_ns())
        except AccountError:
            pass

        self._run_finished = pd.Timestamp.utcnow()
        self._backtest_end = self._kernel.clock.utc_now()

        # 将日志时钟改回实时模式，以保持时间戳一致
        set_logging_clock_realtime_mode()
 
        if LOGGING_PYO3:
            nautilus_pyo3.logging_clock_set_realtime_mode()

        self._log_post_run()

        if LOGGING_PYO3:
            nautilus_pyo3.logger_flush()
        else:
            flush_logger()

    def get_result(self):
        """
        返回最后一次运行的回测结果。

        Returns
        -------
        BacktestResult

        """
        stats_pnls: dict[str, dict[str, float]] = {}

        for currency in self._kernel.portfolio.analyzer.currencies:
            stats_pnls[currency.code] = self._kernel.portfolio.analyzer.get_performance_stats_pnls(currency)

        if self._backtest_start is not None and self._backtest_end is not None:
            elapsed_time = (self._backtest_end - self._backtest_start).total_seconds()
        else:
            elapsed_time = 0

        return BacktestResult(
            trader_id=self._kernel.trader_id.value,
            machine_id=self._kernel.machine_id,
            run_config_id=self._run_config_id,
            instance_id=self._kernel.instance_id.value,
            run_id=self._run_id.to_str() if self._run_id is not None else None,
            run_started=maybe_dt_to_unix_nanos(self._run_started),
            run_finished=maybe_dt_to_unix_nanos(self.run_finished),
            backtest_start=maybe_dt_to_unix_nanos(self._backtest_start),
            backtest_end=maybe_dt_to_unix_nanos(self._backtest_end),
            elapsed_time=elapsed_time,
            iterations=self._iteration,
            total_events=self._kernel.exec_engine.event_count,
            total_orders=self._kernel.cache.orders_total_count(),
            total_positions=len(self._kernel.cache.positions()) + len(self._kernel.cache.position_snapshots()),
            stats_pnls=stats_pnls,
            stats_returns=self._kernel.portfolio.analyzer.get_performance_stats_returns(),
        )

    def _run(
        self,
        start: datetime | str | int | None = None,
        end: datetime | str | int | None = None,
        run_config_id: str | None = None,
        bint streaming = False,
    ):
        # 验证数据已排序并同步至迭代器
        if self._data and not self._sorted:
            raise RuntimeError(
                "数据已添加但未排序，"
                "在运行前调用 `engine.sort_data()` 或使用 `engine.add_data(..., sort=True)`"
            )
 
        # 验证数据
        cdef:
            SimulatedExchange exchange
            InstrumentId instrument_id
            bint has_data
            bint missing_book_data
            bint book_type_has_depth
        for exchange in self._venues.values():
            for instrument_id in exchange.instruments:
                has_data = instrument_id in self._has_data
                missing_book_data = instrument_id not in self._has_book_data
                book_type_has_depth = exchange.book_type > BookType.L1_MBP
 
                if book_type_has_depth and has_data and missing_book_data:
                    raise InvalidConfiguration(
                        f"当 `book_type` 为 '{book_type_to_str(exchange.book_type)}' 时，未找到工具 '{instrument_id }' 的订单簿数据。"
                        "请将场所的 `book_type` 设置为 'L1_MBP'（适用于报价、成交和 Bar 等盘口数据）或为该工具提供订单簿数据。"
                    )

        cdef uint64_t start_ns
        cdef uint64_t end_ns

        # 时间范围检查和设置
        if start is None:
            # 将 `start` 设置为数据开始时间
            start_ns = self._data[0].ts_init if self._data else 0
            start = unix_nanos_to_dt(start_ns)
        else:
            start = pd.to_datetime(start, utc=True)
            start_ns = start.value
 
        if end is None:
            # 将 `end` 设置为数据结束时间
            end_ns = self._data[-1].ts_init if self._data else 4102444800000000000  # 2100-01-01 00:00:00 UTC
            end = unix_nanos_to_dt(end_ns)
        else:
            end = pd.to_datetime(end, utc=True)
            end_ns = end.value
 
        Condition.is_true(start_ns <= end_ns, "开始时间大于结束时间")
        self._end_ns = end_ns
 
        # 设置时钟
        self._last_ns = start_ns

        cdef TestClock clock
        for clock in get_component_clocks(self._instance_id):
            clock.set_time(start_ns)

        if self._iteration == 0:
            # 初始化运行
            self._run_config_id = run_config_id  # 可以为 None
            self._run_id = UUID4()
            self._run_started = pd.Timestamp.utcnow()
            self._backtest_start = start
 
            for exchange in self._venues.values():
                exchange.initialize_account()
                open_orders = self._kernel.cache.orders_open(venue=exchange.id)
 
                for order in open_orders:
                    if order.is_emulated:
                        # 订单应该已经在仿真器中加载
                        continue
 
                    matching_engine = exchange.get_matching_engine(order.instrument_id)
 
                    if matching_engine is None:
                        self._log.error(
                            f"没有用于 {order.instrument_id} 的撮合引擎来处理 {order}",
                        )
                        continue
 
                    matching_engine.process_order(order, order.account_id)
 
            # 重置之前设置的任何 FORCE_STOP
            set_backtest_force_stop(False)
 
            # 设置所有组件（包括日志）的开始时间
            for clock in get_component_clocks(self._instance_id):
                clock.set_time(start_ns)
 
            set_logging_clock_static_mode()
            set_logging_clock_static_time(start_ns)
 
            if LOGGING_PYO3:
                nautilus_pyo3.logging_clock_set_static_mode()
                nautilus_pyo3.logging_clock_set_static_time(start_ns)
 
            # 通用内核启动序列
            self._kernel.start()
 
            self._log_pre_run()
 
        self._log_run(start, end)

        # 设置开始索引
        cdef uint64_t i
        self._data_len = len(self._data)
 
        if self._data_len > 0:
            for i in range(self._data_len):
                if start_ns <= self._data[i].ts_init:
                    self._data_iterator.set_index("backtest_data", i)
                    break
 
        # -- 回测主循环 -----------------------------------------------#
        self._last_ns = 0
        cdef uint64_t raw_handlers_count = 0
        cdef Data data = self._data_iterator.next()
        cdef CVec raw_handlers
        try:
            while data is not None:
                if data.ts_init > end_ns:
                    # 回测结束
                    break
 
                if data.ts_init > self._last_ns:
                    # 将时钟推进到下一个数据时间戳
                    self._last_ns = data.ts_init
                    raw_handlers = self._advance_time(data.ts_init)
                    raw_handlers_count = raw_handlers.len
 
                # 通过交易所处理数据
                if isinstance(data, Instrument):
                    exchange = self._venues[data.id.venue]
                    exchange.update_instrument(data)
                elif isinstance(data, OrderBookDelta):
                    exchange = self._venues[data.instrument_id.venue]
                    exchange.process_order_book_delta(data)
                elif isinstance(data, OrderBookDeltas):
                    exchange = self._venues[data.instrument_id.venue]
                    exchange.process_order_book_deltas(data)
                elif isinstance(data, OrderBookDepth10):
                    exchange = self._venues[data.instrument_id.venue]
                    exchange.process_order_book_depth10(data)
                elif isinstance(data, QuoteTick):
                    exchange = self._venues[data.instrument_id.venue]
                    exchange.process_quote_tick(data)
                elif isinstance(data, TradeTick):
                    exchange = self._venues[data.instrument_id.venue]
                    exchange.process_trade_tick(data)
                elif isinstance(data, Bar):
                    exchange = self._venues[data.bar_type.instrument_id.venue]
                    exchange.process_bar(data)
                elif isinstance(data, InstrumentClose):
                    exchange = self._venues[data.instrument_id.venue]
                    exchange.process_instrument_close(data)
                elif isinstance(data, InstrumentStatus):
                    exchange = self._venues[data.instrument_id.venue]
                    exchange.process_instrument_status(data)
 
                self._data_engine.process(data)
 
                # 处理所有交易所消息
                for exchange in self._venues.values():
                    exchange.process(data.ts_init)
 
                data = self._data_iterator.next()
 
                if data is None or data.ts_init > self._last_ns:
                    self._process_raw_time_event_handlers(
                        raw_handlers,
                        self._last_ns,
                        only_now=True,
                    )
                    if raw_handlers.ptr != NULL:
                        vec_time_event_handlers_drop(raw_handlers)
                    raw_handlers_count = 0
 
                self._iteration += 1
        except AccountError as e:
            set_backtest_force_stop(True)
            self._log.error(f"正因 {e} 而停止回测")
            if streaming:
                # 重新引发异常以中断分批流式传输
                raise
 
        # ---------------------------------------------------------------------#
 
        if FORCE_STOP:
            return
 
        # 处理剩余消息
        for exchange in self._venues.values():
            exchange.process(self._kernel.clock.timestamp_ns())
 
        # 在最后一个数据时间戳刷新剩余事件
        if self._last_ns > 0:
            self._flush_accumulator_events(self._last_ns)

    cdef CVec _advance_time(self, uint64_t ts_now):
        # 推进时钟并按时间戳顺序处理 ts_now 之前的所有事件。
        #
        # 此方法使用迭代处理：在每个回调执行后，重新推进时钟以捕获任何新调度的定时器。
        # 这确保了链式警报（一个警报调度另一个警报）能够按正确的时间戳顺序处理，保持时钟单调性。
        cdef list[TestClock] clocks = get_component_clocks(self._instance_id)
        cdef TestClock clock
        cdef TimeEventHandler_t handler
        cdef uint64_t ts_event
        cdef uint64_t ts_last = 0
        cdef TimeEvent event
        cdef PyObject *raw_callback
        cdef object callback
        cdef SimulatedExchange exchange

        for clock in clocks:
            time_event_accumulator_advance_clock(
                &self._accumulator,
                &clock._mem,
                ts_now,
                False,
            )
 
        # 处理 < ts_now 的事件，每次回调后重新检查新调度的定时器
        while ts_now > 0:
            if FORCE_STOP:
                break

            handler = time_event_accumulator_pop_next_at_or_before(
                &self._accumulator,
                ts_now - 1,
            )

            if handler.callback_ptr == NULL:
                break

            ts_event = handler.event.ts_event
            set_logging_clock_static_time(ts_event)

            if LOGGING_PYO3:
                nautilus_pyo3.logging_clock_set_static_time(ts_event)

            for clock in clocks:
                clock.set_time(ts_event)

            event = TimeEvent.from_mem_c(handler.event)
            raw_callback = <PyObject *>handler.callback_ptr
            callback = <object>raw_callback
            callback(event)
            time_event_handler_drop(handler)

            if ts_event != ts_last:
                ts_last = ts_event
                for exchange in self._venues.values():
                    exchange.process(ts_event)
 
            # 重新推进以捕获由回调调度的定时器
            for clock in clocks:
                time_event_accumulator_advance_clock(
                    &self._accumulator,
                    &clock._mem,
                    ts_now,
                    False,
                )

        set_logging_clock_static_time(ts_now)

        if LOGGING_PYO3:
            nautilus_pyo3.logging_clock_set_static_time(ts_now)

        for clock in clocks:
            clock.set_time(ts_now)

        cdef CVec empty_vec
        empty_vec.ptr = NULL
        empty_vec.len = 0
        empty_vec.cap = 0
        return empty_vec

    cdef void _flush_accumulator_events(self, uint64_t ts_now):
        cdef list[TestClock] clocks = get_component_clocks(self._instance_id)
        cdef TestClock clock
        cdef TimeEventHandler_t handler
        cdef uint64_t ts_event
        cdef uint64_t ts_last = 0
        cdef TimeEvent event
        cdef PyObject *raw_callback
        cdef object callback
        cdef SimulatedExchange exchange

        # 先推进时钟以捕获在最后一次回调期间调度的警报
        for clock in clocks:
            time_event_accumulator_advance_clock(
                &self._accumulator,
                &clock._mem,
                ts_now,
                False,
            )

        while True:
            if FORCE_STOP:
                break

            handler = time_event_accumulator_pop_next_at_or_before(
                &self._accumulator,
                ts_now,
            )

            if handler.callback_ptr == NULL:
                break

            ts_event = handler.event.ts_event
            set_logging_clock_static_time(ts_event)

            if LOGGING_PYO3:
                nautilus_pyo3.logging_clock_set_static_time(ts_event)

            for clock in clocks:
                clock.set_time(ts_event)

            event = TimeEvent.from_mem_c(handler.event)
            raw_callback = <PyObject *>handler.callback_ptr
            callback = <object>raw_callback
            callback(event)
            time_event_handler_drop(handler)

            if ts_event != ts_last:
                ts_last = ts_event
                for exchange in self._venues.values():
                    exchange.process(ts_event)
 
            # 重新推进时钟以捕获由回调调度的链式警报
            for clock in clocks:
                time_event_accumulator_advance_clock(
                    &self._accumulator,
                    &clock._mem,
                    ts_now,
                    False,
                )

    cdef void _process_raw_time_event_handlers(
        self,
        CVec raw_handler_vec,
        uint64_t ts_now,
        bint only_now,
        bint as_of_now = False,
    ):
        cdef list[TestClock] clocks = get_component_clocks(self._instance_id)
        cdef TestClock clock
        cdef TimeEventHandler_t handler
        cdef uint64_t ts_event
        cdef uint64_t ts_last = 0
        cdef TimeEvent event
        cdef PyObject *raw_callback
        cdef object callback
        cdef SimulatedExchange exchange

        if not only_now:
            return

        while True:
            if FORCE_STOP:
                break

            handler = time_event_accumulator_pop_next_at_or_before(
                &self._accumulator,
                ts_now,
            )

            if handler.callback_ptr == NULL:
                break

            ts_event = handler.event.ts_event

            if as_of_now and ts_event > ts_now:
                break

            set_logging_clock_static_time(ts_event)

            if LOGGING_PYO3:
                nautilus_pyo3.logging_clock_set_static_time(ts_event)

            for clock in clocks:
                clock.set_time(ts_event)

            event = TimeEvent.from_mem_c(handler.event)
            raw_callback = <PyObject *>handler.callback_ptr
            callback = <object>raw_callback
            callback(event)
            time_event_handler_drop(handler)

            if ts_event != ts_last:
                ts_last = ts_event
                for exchange in self._venues.values():
                    exchange.process(ts_event)
 
            # 重新推进以捕获由回调调度的定时器
            for clock in clocks:
                time_event_accumulator_advance_clock(
                    &self._accumulator,
                    &clock._mem,
                    ts_now,
                    False,
                )

    def _get_log_color_code(self):
        return "\033[36m" if logging_is_colored() else ""

    def _log_pre_run(self):
        if is_logging_pyo3():
            nautilus_pyo3.log_sysinfo(component=type(self).__name__)
        else:
            log_sysinfo(component=type(self).__name__)

        cdef str color = self._get_log_color_code()

        for exchange in self._venues.values():
            account = exchange.exec_client.get_account()
            self._log.info(f"{color}=================================================================")
            self._log.info(f"{color} SimulatedVenue {exchange.id}")
            self._log.info(f"{color}=================================================================")
            self._log.info(f"{repr(account)}")
            self._log.info(f"{color}-----------------------------------------------------------------")
            self._log.info(f"期初余额：")
 
            if exchange.is_frozen_account:
                self._log.warning(f"账户已冻结")
            else:
                for b in account.starting_balances().values():
                    self._log.info(b.to_formatted_str())

    def _log_run(self, start: pd.Timestamp, end: pd.Timestamp):
        cdef str color = self._get_log_color_code()

        self._log.info(f"{color}=================================================================")
        self._log.info(f"{color} 回测运行")
        self._log.info(f"{color}=================================================================")
        self._log.info(f"运行配置 ID:  {self._run_config_id}")
        self._log.info(f"运行 ID:       {self._run_id}")
        self._log.info(f"运行开始时间:  {format_optional_iso8601(self._run_started)}")
        self._log.info(f"回测开始时间:  {format_optional_iso8601(self._backtest_start)}")
        self._log.info(f"批次开始时间:  {format_optional_iso8601(start)}")
        self._log.info(f"批次结束时间:  {format_optional_iso8601(end)}")
        self._log.info(f"{color}-----------------------------------------------------------------")

    def _log_post_run(self):
        if self._run_finished and self._run_started:
            elapsed_time = self._run_finished - self._run_started
        else:
            elapsed_time = None

        if self._backtest_end and self._backtest_start:
            backtest_range = self._backtest_end - self._backtest_start
        else:
            backtest_range = None

        cdef str color = self._get_log_color_code()

        self._log.info(f"{color}=================================================================")
        self._log.info(f"{color} 回测运行后总结")
        self._log.info(f"{color}=================================================================")
        self._log.info(f"运行配置 ID:    {self._run_config_id}")
        self._log.info(f"运行 ID:         {self._run_id}")
        self._log.info(f"运行开始时间:    {format_optional_iso8601(self._run_started)}")
        self._log.info(f"运行结束时间:    {format_optional_iso8601(self._run_finished)}")
        self._log.info(f"耗时:            {elapsed_time}")
        self._log.info(f"回测开始时间:    {format_optional_iso8601(self._backtest_start)}")
        self._log.info(f"回测结束时间:    {format_optional_iso8601(self._backtest_end)}")
        self._log.info(f"回测范围:        {backtest_range}")
        self._log.info(f"迭代次数:        {self._iteration:_}")
        self._log.info(f"总事件数:        {self._kernel.exec_engine.event_count:_}")
        self._log.info(f"总订单数:        {self._kernel.cache.orders_total_count():_}")

        # 获取场地的所有持仓
        cdef list[Position] positions = []
 
        for position in self._kernel.cache.positions() + self._kernel.cache.position_snapshots():
            positions.append(position)
 
        self._log.info(f"总持仓数: {len(positions):_}")

        if not self._config.run_analysis:
            return

        cdef:
            list[Position] venue_positions
            set venue_currencies
        for venue in self._venues.values():
            account = venue.exec_client.get_account()
            self._log.info(f"{color}=================================================================")
            self._log.info(f"{color} SimulatedVenue {venue.id}")
            self._log.info(f"{color}=================================================================")
            self._log.info(f"{repr(account)}")
            self._log.info(f"{color}-----------------------------------------------------------------")
            unrealized_pnls: dict[Currency, Money] | None = None
 
            if venue.is_frozen_account:
                self._log.warning(f"账户已冻结")
            else:
                if account is None:
                    continue
 
                self._log.info(f"期初余额：")

                for b in account.starting_balances().values():
                    self._log.info(b.to_formatted_str())

                self._log.info(f"{color}-----------------------------------------------------------------")
                self._log.info(f"期末余额：")
 
                for b in account.balances_total().values():
                    self._log.info(b.to_formatted_str())
 
                self._log.info(f"{color}-----------------------------------------------------------------")
                self._log.info(f"佣金费用：")
 
                for c in account.commissions().values():
                    self._log.info(Money(-c.as_double(), c.currency).to_formatted_str())  # 将佣金显示为负数
 
                self._log.info(f"{color}-----------------------------------------------------------------")
                self._log.info(f"未实现盈亏（已包含在总额中）：")
                unrealized_pnls = self.portfolio.unrealized_pnls(Venue(venue.id.value))
 
                if not unrealized_pnls:
                    self._log.info("无")
                else:
                    for b in unrealized_pnls.values():
                        self._log.info(b.to_formatted_str())
 
            # 记录所有模拟模块的输出诊断信息
            for module in venue.modules:
                module.log_diagnostics(self._log)
 
            self._log.info(f"{color}=================================================================")
            self._log.info(f"{color} 投资组合表现")
            self._log.info(f"{color}=================================================================")

            # 收集该场地的所有持仓和货币
            venue_positions = []
            venue_currencies = set()
 
            for position in positions:
                if position.instrument_id.venue == venue.id:
                    venue_positions.append(position)
                    venue_currencies.add(position.quote_currency)
 
                    if position.base_currency is not None:
                        venue_currencies.add(position.base_currency)
 
            # 计算统计数据
            self._kernel.portfolio.analyzer.calculate_statistics(account, venue_positions)
 
            # 按资产展示盈亏表现统计数据
            for currency in sorted(list(venue_currencies), key=lambda x: x.code):
                self._log.info(f" 盈亏统计 ({str(currency)})")
                self._log.info(f"{color}-----------------------------------------------------------------")
                unrealized_pnl = unrealized_pnls.get(currency) if unrealized_pnls else None

                for stat in self._kernel.portfolio.analyzer.get_stats_pnls_formatted(currency, unrealized_pnl):
                    self._log.info(stat)

                self._log.info(f"{color}-----------------------------------------------------------------")
 
            self._log.info(" 收益统计")
            self._log.info(f"{color}-----------------------------------------------------------------")
 
            for stat in self._kernel.portfolio.analyzer.get_stats_returns_formatted():
                self._log.info(stat)
 
            self._log.info(f"{color}-----------------------------------------------------------------")
 
            self._log.info(" 通用统计")
            self._log.info(f"{color}-----------------------------------------------------------------")
 
            for stat in self._kernel.portfolio.analyzer.get_stats_general_formatted():
                self._log.info(stat)
 
            self._log.info(f"{color}-----------------------------------------------------------------")

    def _add_data_client_if_not_exists(self, ClientId client_id) -> None:
        if client_id not in self._kernel.data_engine.registered_clients:
            client = BacktestDataClient(
                client_id=client_id,
                msgbus=self._kernel.msgbus,
                cache=self._kernel.cache,
                clock=self._kernel.clock,
            )
            self._kernel.data_engine.register_client(client)

    def _add_market_data_client_if_not_exists(self, Venue venue) -> None:
        cdef ClientId client_id = ClientId(venue.value)

        if client_id not in self._kernel.data_engine.registered_clients:
            client = BacktestMarketDataClient(
                client_id=client_id,
                msgbus=self._kernel.msgbus,
                cache=self._kernel.cache,
                clock=self._kernel.clock,
            )
            self._kernel.data_engine.register_client(client)

    def set_default_market_data_client(self) -> None:
        cdef ClientId client_id = ClientId("BACKTEST")
        client = BacktestMarketDataClient(
            client_id=client_id,
            msgbus=self._kernel.msgbus,
            cache=self._kernel.cache,
            clock=self._kernel.clock,
        )
        self._kernel.data_engine.register_client(client)


cdef class BacktestDataIterator:
    """
    回测中历史 ``Data`` 流的时间顺序多路复用器。

    该迭代器有效地管理多个数据流，并根据其 ``ts_init`` 时间戳以严格的时间顺序产生 ``Data`` 对象。
    它支持用于流式传输大型数据集的静态数据列表和动态数据生成器。

    **架构：**

    - **单流优化**：当只加载一个流时，使用快速数组遍历以获得最佳性能。
    - **多流合并**：对于两个或更多流，使用二进制最小堆执行高效的 k 路归并排序。
    - **动态流式传输**：支持按需产生数据块的 Python 生成器，从而能够处理大于可用内存的数据集。

    **流优先级：**

    不仅可以使用 ``append_data`` 参数为流分配不同的优先级：

    - ``append_data=True``（默认）：较低优先级，在现有流之后处理
    - ``append_data=False``：较高优先级，在现有流之前处理

    当多个数据点具有相同的时间戳时，优先生成较高优先级的流。

    **性能特征：**

    - **内存效率**：动态生成器增量加载数据
    - **时间复杂度**：n 个流的每项 O(log n)（堆操作）
    - **空间复杂度**：O(k)，其中 k 是任何给定时间所有流中活动数据点的总数

    Notes
    -----
    当使用 ``presorted=False``（默认）的 ``add_data()`` 时，数据将在内部排序。
    当使用 ``presorted=True`` 或 ``init_data()`` 时，数据必须按 ``ts_init`` 升序预先排序。

    See Also
    --------
    BacktestEngine.add_data : Add static data to the backtest engine
    BacktestEngine.add_data_iterator : Add streaming data generators

    """
    def __init__(self) -> None:
        self._log = Logger(type(self).__name__)

        self._data = {} # key=data_priority, value=data_list
        self._data_name = {} # key=data_priority, value=data_name
        self._data_priority = {} # key=data_name, value=data_priority
        self._data_len = {} # key=data_priority, value=len(data_list)
        self._data_index = {} # key=data_priority, value=current index of data_list
        self._data_update_function = {} # key=data_priority, value=data_update_function, Callable[[], list] | None

        self._heap = []
        # 用于为数据流分配优先级的计数器。
        # 在使用前递增，以便永远不会分配零优先级。
        self._next_data_priority = 0
        self._reset_single_data()

    cpdef void _reset_single_data(self):
        self._single_data = []
        self._single_data_name = ""
        self._single_data_priority = 0
        self._single_data_len = 0
        self._single_data_index = 0
        self._is_single_data = False

    def add_data(
        self,
        str data_name,
        list data,
        bint append_data = True,
        bint presorted = False,
    ) -> None:
        """
        添加（或替换）用于静态数据加载的命名数据列表。

        如果已存在具有相同 ``data_name`` 的流，它将被新数据替换。

        Parameters
        ----------
        data_name : str
            数据流的唯一标识符。
        data : list[Data]
            要添加的数据实例。如果 ``presorted=True``，必须按 `ts_init` 预先排序。
        append_data : bool, default ``True``
            控制时间戳并列时的流优先级：
            ``True`` – 较低优先级（追加）。
            ``False`` – 较高优先级（前置）。
        presorted : bool, default ``False``
            如果 ``True``，假设数据已按 `ts_init` 排序，并跳过内部排序以获得更好的性能。
            如果 ``False``（默认），数据将在内部排序。

        Raises
        ------
        ValueError
            如果 `data_name` 不是有效的字符串。

        """
        Condition.valid_string(data_name, "data_name")

        if not data:
            return

        self._add_data(data_name, data, append_data, presorted)

    def init_data(
        self,
        str data_name,
        data_generator,
        bint append_data = True,
    ) -> None:
        """
        添加（或替换）用于流式传输大型数据集的命名数据生成器。

        此方法通过使用按需生成数据块的 Python 生成器来实现大型数据集的内存高效处理。
        随着数据的消耗，生成器被增量调用，从而允许处理大于可用内存的数据集。

        生成器应产生 ``Data`` 对象列表，其中这每个列表代表一个数据块。
        当一个块耗尽时，迭代器会自动调用生成器上的 ``next()`` 来获取下一个块。

        Parameters
        ----------
        data_name : str
            数据流的唯一标识符。
        data_generator : Generator[list[Data], None, None]
            产生按 `ts_init` 升序排序的 ``Data`` 实例列表的 Python 生成器。
        append_data : bool, default ``True``
            控制时间戳并列时的流优先级：
            ``True`` – 较低优先级（追加）。
            ``False`` – 较高优先级（前置）。

        Raises
        ------
        ValueError
            如果 `data_name` 不是有效的字符串。

        """
        Condition.valid_string(data_name, "data_name")

        cdef list[Data] data

        try:
            data = next(data_generator)
 
            if data:
                self._data_update_function[data_name] = data_generator
                self._add_data(data_name, data, append_data)
                self._log.debug(f"已从迭代器 '{data_name}' 添加 {len(data):_} 个数据元素")
        except StopIteration:
            # 生成器已耗尽，无内容可添加
            pass

    cdef void _add_data(
        self,
        str data_name,
        list data_list,
        bint append_data = True,
        bint presorted = False,
    ):
        if len(data_list) == 0:
            return

        cdef int data_priority

        if data_name in self._data_priority:
            data_priority = self._data_priority[data_name]
            self.remove_data(data_name)
        else:
            # heapq 是一个最小优先级队列，因此较小的值会先弹出。
            # 在应用符号 *之前* 递增计数器，以便永远不会产生零优先级
            # （零在对流进行排序时会破坏前置/追加语义）。
            self._next_data_priority += 1
            data_priority = (1 if append_data else -1) * self._next_data_priority

        if self._is_single_data:
            self._deactivate_single_data()
 
        # 复制并根据需要选择排序，以避免对调用者的列表起别名
        if presorted:
            self._data[data_priority] = list(data_list)
        else:
            self._data[data_priority] = sorted(data_list, key=lambda data: data.ts_init)

        self._data_name[data_priority] = data_name
        self._data_priority[data_name] = data_priority
        self._data_len[data_priority] = len(data_list)
        self._data_index[data_priority] = 0

        if len(self._data) == 1:
            self._activate_single_data()
            return

        self._push_data(data_priority, 0)

    cpdef void remove_data(self, str data_name, bint complete_remove=False):
        """
        删除由 ``data_name`` 标识的数据流。如果指定的流不存在，则静默忽略该操作。

        Parameters
        ----------
        data_name : str
            要删除的数据流的唯一标识符。
        complete_remove : bool, default False
            控制执行清理的级别：
            - ``False``：删除流数据但保留生成器函数以便潜在的重新初始化（对于临时流删除很有用）
            - ``True``：完全删除，包括任何关联的生成器函数（建议用于永久流删除）

        Raises
        ------
        ValueError
            如果 `data_name` 不是有效的字符串。

        """
        Condition.valid_string(data_name, "data_name")

        if data_name not in self._data_priority:
            return

        cdef int data_priority = self._data_priority[data_name]
        del self._data[data_priority]
        del self._data_name[data_priority]
        del self._data_priority[data_name]
        del self._data_len[data_priority]
        del self._data_index[data_priority]

        if complete_remove:
            del self._data_update_function[data_name]

        if len(self._data) == 1:
            self._activate_single_data()
            return

        if len(self._data) == 0:
            self._reset_single_data()
            return
 
        # 排除 data_priority 后重构堆
        self._heap = [item for item in self._heap if item[1] != data_priority]
        heapq.heapify(self._heap)

    cpdef void _activate_single_data(self):
        assert len(self._data) == 1

        cdef str single_data_name = list(self._data_name.values())[0]
        self._single_data_name = single_data_name
        self._single_data_priority = self._data_priority[self._single_data_name]
        self._single_data = self._data[self._single_data_priority]
        self._single_data_len = self._data_len[self._single_data_priority]
        self._single_data_index = self._data_index[self._single_data_priority]
        self._heap = []
        self._is_single_data = True

    cpdef void _deactivate_single_data(self):
        assert len(self._heap) == 0

        if self._single_data_index < self._single_data_len:
            self._data_index[self._single_data_priority] = self._single_data_index
            self._push_data(self._single_data_priority, self._single_data_index)

        self._reset_single_data()

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cpdef Data next(self):
        """
        按时间顺序返回下一个 ``Data`` 对象。

        此方法实现了核心迭代逻辑，根据 ``ts_init`` 时间戳以严格的时间顺序
        产生来自所有流的数据点。当多个数据点具有相同的时间戳时，流优先级决定顺序。

        该方法自动处理：
        - 性能的单流优化
        - 基于堆的多流合并
        - 来自生成器的动态数据加载
        - 流耗尽和清理

        Returns
        -------
        Data or None
            按时间顺序的下一个 ``Data`` 对象，或者当所有流耗尽时为 ``None``。

        Notes
        -----
        - 当所有流耗尽时返回 ``None``
        - 自动触发流数据的生成器调用
        - 针对单流场景优化了性能
        - 仅当从单个线程调用时才线程安全

        """
        cdef:
            uint64_t ts_init
            int data_priority
            int cursor
            Data object_to_return

        if not self._is_single_data:
            if not self._heap:
                return None

            ts_init, data_priority, cursor = heapq.heappop(self._heap)
            object_to_return = self._data[data_priority][cursor]

            self._data_index[data_priority] += 1
            self._push_data(data_priority, self._data_index[data_priority])

            return object_to_return

        if self._single_data_index >= self._single_data_len:
            return None

        object_to_return = self._single_data[self._single_data_index]
        self._single_data_index += 1

        if self._single_data_index >= self._single_data_len:
            self._update_data(self._single_data_priority)

        return object_to_return

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cpdef void _push_data(self, int data_priority, int data_index):
        cdef uint64_t ts_init

        if data_index < self._data_len[data_priority]:
            ts_init = self._data[data_priority][data_index].ts_init
            heapq.heappush(self._heap, (ts_init, data_priority, data_index))
        else:
            self._update_data(data_priority)

    cpdef void _update_data(self, int data_priority):
        cdef str data_name = self._data_name[data_priority]

        if data_name not in self._data_update_function:
            return

        cdef list[Data] data

        try:
            data = next(self._data_update_function[data_name])
 
            if data:
                # 无需 append_data 布尔值，因为它是一个更新
                self._add_data(data_name, data)
                self._log.debug(f"正在从迭代器 '{data_name}' 添加 {len(data):_} 个数据元素")
            else:
                self.remove_data(data_name, complete_remove=True)
        except StopIteration:
            # 生成器已耗尽，删除流
            self.remove_data(data_name, complete_remove=True)

    cpdef void set_index(self, str data_name, int index):
        """
        将 `data_name` 的游标移动到 `index` 并重新构建排序。
 
        Raises
        ------
        ValueError
            如果 `data_name` 不是有效的字符串。
 
        """
        Condition.valid_string(data_name, "data_name")
 
        if data_name not in self._data_priority:
            return
 
        cdef int data_priority = self._data_priority[data_name]
        self._data_index[data_priority] = index
        self._reset_heap()

    cpdef void _reset_heap(self):
        if len(self._data) == 1:
            self._activate_single_data()
            return

        self._heap = []

        for data_priority, index in self._data_index.items():
            self._push_data(data_priority, index)

    cpdef bint is_done(self):
        """
        当每个流都已完全消耗时返回 ``True``。
        """
        if self._is_single_data:
            return self._single_data_index >= self._single_data_len
        else:
            return not self._heap

    cpdef dict all_data(self):
        """
        返回 ``{stream_name: list[Data]}`` 的 *浅* 映射。
        """
        # 我们假设字典按插入顺序排列
        return {data_name:self._data[data_priority] for data_priority, data_name in self._data_name.items()}

    cpdef list[Data] data(self, str data_name):
        """
        返回 `data_name` 的底层数据列表。

        Returns
        -------
        list[Data]

        Raises
        ------
        ValueError
            如果 `data_name` 不是有效的字符串。
        KeyError
            如果流未知。

        """
        Condition.valid_string(data_name, "data_name")

        return self._data[self._data_priority[data_name]]

    def __iter__(self):
        return self

    def __next__(self):
        cdef Data element
        element = self.next()

        if element is None:
            raise StopIteration

        return element


cdef class SimulatedExchange:
    """
    提供一个模拟交易所场所。

    Parameters
    ----------
    venue : Venue
        要模拟的场所。
    oms_type : OmsType {``HEDGING``, ``NETTING``}
        交易所使用的订单管理系统类型。
    account_type : AccountType
        客户的账户类型。
    starting_balances : list[Money]
        交易所的期初余额。
    base_currency : Currency, optional
        客户的账户基础货币。对于多币种账户，使用 ``None``。
    default_leverage : Decimal
        账户默认杠杆（用于保证金账户）。
    leverages : dict[InstrumentId, Decimal]
        特定工具的杠杆配置（用于保证金账户）。
    modules : list[SimulationModule]
        交易所的模拟模块。
    portfolio : PortfolioFacade
        交易所的只读投资组合。
    msgbus : MessageBus
        交易所的消息总线。
    cache : CacheFacade
        交易所的只读缓存。
    clock : TestClock
        交易所的时钟。
    fill_model : FillModel
        交易所的成交模型。
    fee_model : FeeModel
        交易所的费用模型。
    latency_model : LatencyModel, optional
        交易所的延迟模型。
    book_type : BookType
        交易所的订单簿类型。
    frozen_account : bool, default False
        此交易所的账户是否冻结（余额不会改变）。
    reject_stop_orders : bool, default True
        如果提交时触发价格在市场价格范围内，是否拒绝止损单。
    support_gtd_orders : bool, default True
        交易所是否支持 GTD（Good Till Date）有效时间的订单。
    support_contingent_orders : bool, default True
        交易所是否支持/遵循条件订单。
        如果为 False，则预期策略将管理任何条件订单。
    oto_trigger_mode : OtoTriggerMode, default ``OtoTriggerMode.PARTIAL``
        条件订单的 OTO 触发模式：
        - ``PARTIAL``：根据每次部分成交按比例释放子订单（默认）。
        - ``FULL``：仅在父订单完全成交后释放子订单。
    use_position_ids : bool, default True
        是否在订单成交时生成场所持仓 ID。
    use_random_ids : bool, default False
        是否所有交易所生成的标识符都是随机 UUID4。
    use_reduce_only : bool, default True
        是否遵循订单上的 `reduce_only` 执行指令。
    use_message_queue : bool, default True
        是否应使用内部消息队列按顺序处理交易指令。对于实时沙盒环境，
        将其设置为 False 可能更合适，因为我们不想在处理交易指令之前引入
        等待下一个数据事件的额外延迟。
    use_market_order_acks : bool, default False
        是否在成交前为市价单生成 OrderAccepted 事件。
    bar_execution : bool, default True
        是否应由撮合引擎处理 Bar 数据（并推动市场）。
    bar_adaptive_high_low_ordering : bool, default False
        决定是否根据启发式算法自适应处理 Bar 价格顺序。
        此设置仅在 `bar_execution` 为 True 时相关。
        如果为 False，Bar 价格始终按固定顺序处理：Open, High, Low, Close。
        如果为 True，处理顺序随启发式算法调整：
        - 如果 High 比 Low 更接近 Open，则处理顺序为 Open, High, Low, Close。
        - 如果 Low 比 High 更接近 Open，则处理顺序为 Open, Low, High, Close。
    price_protection_points : int, optional
        定义交易所计算的价格边界（以点为单位），以防止市价单在过于激进的价格执行。
    trade_execution : bool, default False
        是否应由撮合引擎处理 Trade 数据（并推动市场）。
    liquidity_consumption : bool, default False
        是否应按价格水平跟踪流动性消耗。启用时，成交会消耗可用流动性，
        当该水平的新数据到达时重置。禁用时，每次迭代都可以独立地根据
        全额订单簿流动性进行成交。

    Raises
    ------
    ValueError
        如果 `instruments` 为空。
    ValueError
        如果 `instruments` 包含非 `Instrument` 类型。
    ValueError
        如果 `starting_balances` 为空。
    ValueError
        如果 `starting_balances` 包含非 `Money` 类型。
    ValueError
        如果指定了 `base_currency` 且有多个期初余额。
    ValueError
        如果 `modules` 包含非 `SimulationModule` 类型。

    """

    def __init__(
        self,
        Venue venue not None,
        OmsType oms_type,
        AccountType account_type,
        list starting_balances not None,
        Currency base_currency: Currency | None,
        default_leverage not None: Decimal,
        leverages not None: dict[InstrumentId, Decimal],
        list modules not None,
        PortfolioFacade portfolio not None,
        MessageBus msgbus not None,
        CacheFacade cache not None,
        TestClock clock not None,
        FillModel fill_model not None,
        FeeModel fee_model not None,
        LatencyModel latency_model = None,
        MarginModel margin_model = None,
        BookType book_type = BookType.L1_MBP,
        bint frozen_account = False,
        bint reject_stop_orders = True,
        bint support_gtd_orders = True,
        bint support_contingent_orders = True,
        OtoTriggerMode oto_trigger_mode = OtoTriggerMode.PARTIAL,
        bint use_position_ids = True,
        bint use_random_ids = False,
        bint use_reduce_only = True,
        bint use_message_queue = True,
        bint use_market_order_acks = False,
        bint bar_execution = True,
        bint bar_adaptive_high_low_ordering = False,
        bint trade_execution = False,
        bint liquidity_consumption = False,
        price_protection_points=None,
    ) -> None:
        Condition.not_empty(starting_balances, "starting_balances")
        Condition.list_type(starting_balances, Money, "starting_balances")
        Condition.list_type(modules, SimulationModule, "modules", "SimulationModule")
        if base_currency:
            Condition.is_true(len(starting_balances) == 1, "single-currency account has multiple starting currencies")
        if default_leverage and default_leverage > 1 or leverages:
            Condition.is_true(account_type == AccountType.MARGIN, "leverages defined when account type is not `MARGIN`")

        self._clock = clock
        self._log = Logger(name=f"{type(self).__name__}({venue})")

        self.id = venue
        self.oms_type = oms_type
        self._log.info(f"OmsType={oms_type_to_str(oms_type)}")
        self.book_type = book_type

        self.msgbus = msgbus
        self.cache = cache
        self.exec_client = None  # 在注册执行客户端时初始化
 
        # 会计
        self.account_type = account_type
        self.base_currency = base_currency
        self.starting_balances = starting_balances
        self.default_leverage = default_leverage
        self.leverages = leverages
        self.margin_model = margin_model
        self.is_frozen_account = frozen_account
 
        # 执行配置
        self.reject_stop_orders = reject_stop_orders
        self.support_gtd_orders = support_gtd_orders
        self.support_contingent_orders = support_contingent_orders
        self.oto_full_trigger = oto_trigger_mode == OtoTriggerMode.FULL
        self.use_position_ids = use_position_ids
        self.use_random_ids = use_random_ids
        self.use_reduce_only = use_reduce_only
        self.use_message_queue = use_message_queue
        self.use_market_order_acks = use_market_order_acks
        self.bar_execution = bar_execution
        self.bar_adaptive_high_low_ordering = bar_adaptive_high_low_ordering
        self.trade_execution = trade_execution
        self.liquidity_consumption = liquidity_consumption
        self.price_protection_points = price_protection_points if price_protection_points is not None else 0
 
        # 执行模型
        self.fill_model = fill_model
        self.fee_model = fee_model
        self.latency_model = latency_model
 
        # 加载模块
        self.modules = []
        for module in modules:
            Condition.not_in(module, self.modules, "module", "modules")
            module.register_base(
                portfolio=portfolio,
                msgbus=msgbus,
                cache=cache,
                clock=clock,
            )
            # OptionExerciseModule 在 `register_venue` 方法中订阅持仓事件。
            # 订阅事件需要消息总线可用。
            # 因此，`register_base` 在 `register_venue` 之前调用。
            module.register_venue(self)
            self.modules.append(module)
            self._log.info(f"已加载 {module}")
 
        # 市场
        self.instruments: dict[InstrumentId, Instrument] = {}
        self._matching_engines: dict[InstrumentId, OrderMatchingEngine] = {}
 
        self._message_queue = deque()
        self._inflight_queue: list[tuple[(uint64_t, uint64_t), TradingCommand]] = []
        self._inflight_counter: dict[uint64_t, uint64_t] = {}
 
        # 用于来自 SpreadQuoteAggregator 的直接通信
        spread_quote_endpoint = f"SimulatedExchange.spread_quote.{venue}"
        if spread_quote_endpoint not in self.msgbus._endpoints:
            self.msgbus.register(endpoint=spread_quote_endpoint, handler=self.process_quote_tick)

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}("
            f"id={self.id}, "
            f"oms_type={oms_type_to_str(self.oms_type)}, "
            f"account_type={account_type_to_str(self.account_type)})"
        )

# -- REGISTRATION ---------------------------------------------------------------------------------

    cpdef void register_client(self, BacktestExecClient client):
        """
        向模拟交易所注册给定的执行客户端。

        Parameters
        ----------
        client : BacktestExecClient
            要注册的客户端。
 
        """
        Condition.not_none(client, "client")
 
        self.exec_client = client
 
        self._log.info(f"已注册 ExecutionClient-{client}")

    cpdef void set_fill_model(self, FillModel fill_model):
        """
        设置所有撮合引擎的成交模型。

        Parameters
        ----------
        fill_model : FillModel
            要设置的成交模型。

        """
        Condition.not_none(fill_model, "fill_model")

        self.fill_model = fill_model

        cdef OrderMatchingEngine matching_engine
        for matching_engine in self._matching_engines.values():
            matching_engine.set_fill_model(fill_model)
            self._log.info(
                f"已将 {matching_engine.venue} 的 `FillModel` "
                f"更改为 {self.fill_model}",
            )

    cpdef void set_latency_model(self, LatencyModel latency_model):
        """
        更改此交易所的延迟模型。

        Parameters
        ----------
        latency_model : LatencyModel
            要设置的延迟模型。

        """
        Condition.not_none(latency_model, "latency_model")
 
        self.latency_model = latency_model
 
        self._log.info("已更改延迟模型")

    cpdef void initialize_account(self):
        """
        初始化账户至期初余额。

        """
        self._generate_fresh_account_state()

    cpdef void add_instrument(self, Instrument instrument):
        """
        将给定的工具添加到交易所。

        Parameters
        ----------
        instrument : Instrument
            要添加的工具。

        Raises
        ------
        ValueError
            如果 `instrument.id.venue` 不等于场所 ID。
        InvalidConfiguration
            如果 `instrument` 对此场所无效。

        """
        Condition.not_none(instrument, "instrument")
        Condition.equal(instrument.id.venue, self.id, "instrument.id.venue", "self.id")

        # 验证工具
        if isinstance(instrument, (CryptoPerpetual, CryptoFuture)):
            if self.account_type == AccountType.CASH:
                raise InvalidConfiguration(
                    f"无法将 `{type(instrument).__name__}` 类型的工具添加到具有 `CASH` 账户类型的场所。"
                    f"请添加至具有 `MARGIN` 账户类型的场所。",
                )

        self.instruments[instrument.id] = instrument

        cdef OrderMatchingEngine matching_engine = OrderMatchingEngine(
            instrument=instrument,
            raw_id=len(self.instruments),
            fill_model=self.fill_model,
            fee_model=self.fee_model,
            book_type=self.book_type,
            oms_type=self.oms_type,
            account_type=self.account_type,
            msgbus=self.msgbus,
            cache=self.cache,
            clock=self._clock,
            reject_stop_orders=self.reject_stop_orders,
            support_gtd_orders=self.support_gtd_orders,
            support_contingent_orders=self.support_contingent_orders,
            oto_full_trigger=self.oto_full_trigger,
            use_position_ids=self.use_position_ids,
            use_random_ids=self.use_random_ids,
            use_reduce_only=self.use_reduce_only,
            use_market_order_acks=self.use_market_order_acks,
            bar_execution=self.bar_execution,
            bar_adaptive_high_low_ordering=self.bar_adaptive_high_low_ordering,
            trade_execution=self.trade_execution,
            liquidity_consumption=self.liquidity_consumption,
            price_protection_points=self.price_protection_points,
        )

        self._matching_engines[instrument.id] = matching_engine
 
        self._log.info(f"已添加工具 {instrument.id} 并创建撮合引擎")

# -- QUERIES --------------------------------------------------------------------------------------

    cpdef Price best_bid_price(self, InstrumentId instrument_id):
        """
        返回给定工具 ID 的最佳买入价格（如果找到）。

        Parameters
        ----------
        instrument_id : InstrumentId
            价格的工具 ID。

        Returns
        -------
        Price or ``None``

        """
        Condition.not_none(instrument_id, "instrument_id")

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(instrument_id)
        if matching_engine is None:
            return None

        return matching_engine.best_bid_price()

    cpdef Price best_ask_price(self, InstrumentId instrument_id):
        """
        返回给定工具 ID 的最佳卖出价格（如果找到）。

        Parameters
        ----------
        instrument_id : InstrumentId
            价格的工具 ID。

        Returns
        -------
        Price or ``None``

        """
        Condition.not_none(instrument_id, "instrument_id")

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(instrument_id)
        if matching_engine is None:
            return None

        return matching_engine.best_ask_price()

    cpdef OrderBook get_book(self, InstrumentId instrument_id):
        """
        返回给定工具 ID 的订单簿。

        Parameters
        ----------
        instrument_id : InstrumentId
            价格的工具 ID。

        Returns
        -------
        OrderBook or ``None``

        """
        Condition.not_none(instrument_id, "instrument_id")

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(instrument_id)
        if matching_engine is None:
            return None

        return matching_engine.get_book()

    cpdef OrderMatchingEngine get_matching_engine(self, InstrumentId instrument_id):
        """
        返回给定工具 ID 的撮合引擎（如果找到）。

        Parameters
        ----------
        instrument_id : InstrumentId
            撮合引擎的工具 ID。

        Returns
        -------
        OrderMatchingEngine or ``None``

        """
        return self._matching_engines.get(instrument_id)

    cpdef dict get_matching_engines(self):
        """
        返回交易所的所有撮合引擎（针对每个工具）。

        Returns
        -------
        dict[InstrumentId, OrderMatchingEngine]

        """
        return self._matching_engines.copy()

    cpdef dict get_books(self):
        """
        返回交易所内的所有订单簿。

        Returns
        -------
        dict[InstrumentId, OrderBook]

        """
        cdef dict[InstrumentId, OrderBook] books = {}

        cdef OrderMatchingEngine matching_engine
        for matching_engine in self._matching_engines.values():
            books[matching_engine.instrument.id] = matching_engine.get_book()

        return books

    cpdef list[Order] get_open_orders(self, InstrumentId instrument_id = None):
        """
        返回交易所的挂单。

        Parameters
        ----------
        instrument_id : InstrumentId, optional
            工具 ID 查询过滤器。

        Returns
        -------
        list[Order]

        """
        cdef OrderMatchingEngine matching_engine
        if instrument_id is not None:
            matching_engine = self._matching_engines.get(instrument_id)
            if matching_engine is None:
                return []
            else:
                return matching_engine.get_open_orders()

        cdef list[Order] open_orders = []
        for matching_engine in self._matching_engines.values():
            open_orders += matching_engine.get_open_orders()

        return open_orders

    cpdef list[Order] get_open_bid_orders(self, InstrumentId instrument_id = None):
        """
        返回交易所的买入挂单。

        Parameters
        ----------
        instrument_id : InstrumentId, optional
            工具 ID 查询过滤器。

        Returns
        -------
        list[Order]

        """
        cdef OrderMatchingEngine matching_engine
        if instrument_id is not None:
            matching_engine = self._matching_engines.get(instrument_id)
            if matching_engine is None:
                return []
            else:
                return matching_engine.get_open_bid_orders()

        cdef list[Order] open_bid_orders = []
        for matching_engine in self._matching_engines.values():
            open_bid_orders += matching_engine.get_open_bid_orders()

        return open_bid_orders

    cpdef list[Order] get_open_ask_orders(self, InstrumentId instrument_id = None):
        """
        返回交易所的卖出挂单。

        Parameters
        ----------
        instrument_id : InstrumentId, optional
            工具 ID 查询过滤器。

        Returns
        -------
        list[Order]

        """
        cdef OrderMatchingEngine matching_engine
        if instrument_id is not None:
            matching_engine = self._matching_engines.get(instrument_id)
            if matching_engine is None:
                return []
            else:
                return matching_engine.get_open_ask_orders()

        cdef list[Order] open_ask_orders = []
        for matching_engine in self._matching_engines.values():
            open_ask_orders += matching_engine.get_open_ask_orders()

        return open_ask_orders

    cpdef Account get_account(self):
        """
        返回注册客户端的账户（如果已注册）。

        Returns
        -------
        Account or ``None``

        """
        Condition.not_none(self.exec_client, "self.exec_client")

        return self.exec_client.get_account()

# -- COMMANDS -------------------------------------------------------------------------------------

    cpdef void adjust_account(self, Money adjustment):
        """
        使用给定的调整量调整交易所的账户。

        Parameters
        ----------
        adjustment : Money
            账户的调整量。

        """
        Condition.not_none(adjustment, "adjustment")

        if self.is_frozen_account:
            return  # Nothing to adjust

        cdef Account account = self.cache.account_for_venue(self.exec_client.venue)
        if account is None:
            self._log.error(
                f"无法调整账户：未找到 {self.exec_client.venue} 的账户"
            )
            return

        cdef AccountBalance balance = account.balance(adjustment.currency)
        if balance is None:
            self._log.error(
                f"无法调整账户：未找到 {adjustment.currency} 的余额"
            )
            return

        balance.total = balance.total + adjustment
        balance.free = balance.free + adjustment

        cdef list[MarginBalance] margins = []
        if account.is_margin_account:
            margins = list(account.margins().values())
 
        # 生成并处理事件
        self.exec_client.generate_account_state(
            balances=[balance],
            margins=margins,
            reported=True,
            ts_event=self._clock.timestamp_ns(),
        )

    cpdef void update_instrument(self, Instrument instrument):
        """
        使用给定的工具更新场所当前的工具定义。

        Parameters
        ----------
        instrument : Instrument
            要更新的工具定义。

        """
        Condition.not_none(instrument, "instrument")

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(instrument.id)
        if matching_engine is None:
            self.add_instrument(instrument)
            return

        matching_engine.update_instrument(instrument)

    cpdef void send(self, TradingCommand command):
        """
        将给定的交易指令发送到交易所。

        Parameters
        ----------
        command : TradingCommand
            要发送的指令。

        """
        Condition.not_none(command, "command")

        if not self.use_message_queue:
            self._process_trading_command(command)
        elif self.latency_model is None:
            self._message_queue.appendleft(command)
        else:
            heappush(self._inflight_queue, self.generate_inflight_command(command))

    cdef tuple generate_inflight_command(self, TradingCommand command):
        cdef uint64_t ts
        if isinstance(command, (SubmitOrder, SubmitOrderList)):
            ts = command.ts_init + self.latency_model.insert_latency_nanos
        elif isinstance(command, ModifyOrder):
            ts = command.ts_init + self.latency_model.update_latency_nanos
        elif isinstance(command, (CancelOrder, CancelAllOrders, BatchCancelOrders)):
            ts = command.ts_init + self.latency_model.cancel_latency_nanos
        else:
            raise ValueError(f"无效的 `TradingCommand`，原为 {command}")  # pragma: no cover (设计时错误)

        if ts not in self._inflight_counter:
            self._inflight_counter[ts] = 0

        self._inflight_counter[ts] += 1
        cdef (uint64_t, uint64_t) key = (ts, self._inflight_counter[ts])

        return key, command

    cpdef void process_order_book_delta(self, OrderBookDelta delta):
        """
        处理给定订单簿增量的交易所市场。

        Parameters
        ----------
        data : OrderBookDelta
            要处理的订单簿增量。

        """
        Condition.not_none(delta, "delta")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(delta)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(delta.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(delta.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {delta.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[delta.instrument_id]

        matching_engine.process_order_book_delta(delta)

    cpdef void process_order_book_deltas(self, OrderBookDeltas deltas):
        """
        处理给定订单簿增量的交易所市场。

        Parameters
        ----------
        data : OrderBookDeltas
            要处理的订单簿增量。

        """
        Condition.not_none(deltas, "deltas")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(deltas)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(deltas.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(deltas.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {deltas.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[deltas.instrument_id]

        matching_engine.process_order_book_deltas(deltas)

    cpdef void process_order_book_depth10(self, OrderBookDepth10 depth):
        """
        处理给定订单簿深度的交易所市场。

        Parameters
        ----------
        depth : OrderBookDepth10
            要处理的订单簿深度。

        """
        Condition.not_none(depth, "depth")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(depth)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(depth.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(depth.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {depth.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[depth.instrument_id]

        matching_engine.process_order_book_depth10(depth)

    cpdef void process_quote_tick(self, QuoteTick tick):
        """
        处理给定报价 Tick 的交易所市场。

        通过拍卖挂单来模拟市场动态。

        Parameters
        ----------
        tick : QuoteTick
            要处理的 Tick。

        """
        Condition.not_none(tick, "tick")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(tick)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(tick.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(tick.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {tick.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[tick.instrument_id]
 
        matching_engine.process_quote_tick(tick)

    cpdef void process_trade_tick(self, TradeTick tick):
        """
        处理给定成交 Tick 的交易所市场。

        通过拍卖挂单来模拟市场动态。

        Parameters
        ----------
        tick : TradeTick
            要处理的 Tick。

        """
        Condition.not_none(tick, "tick")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(tick)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(tick.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(tick.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {tick.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[tick.instrument_id]

        matching_engine.process_trade_tick(tick)

    cpdef void process_bar(self, Bar bar):
        """
        处理给定 Bar 的交易所市场。

        通过拍卖挂单来模拟市场动态。

        Parameters
        ----------
        bar : Bar
            要处理的 Bar。

        """
        Condition.not_none(bar, "bar")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(bar)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(bar.bar_type.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(bar.bar_type.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {bar.bar_type.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[bar.bar_type.instrument_id]

        matching_engine.process_bar(bar)

    cpdef void process_instrument_status(self, InstrumentStatus data):
        """
        处理特定工具状态。

        Parameters
        ----------
        data : InstrumentStatus
            要处理的工具状态更新。

        """
        Condition.not_none(data, "data")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(data)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(data.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(data.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {data.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[data.instrument_id]

        matching_engine.process_status(data.action)

    cpdef void process_instrument_close(self, InstrumentClose close):
        """
        处理给定工具收盘的交易所市场。

        Parameters
        ----------
        close : InstrumentClose
            要处理的工具收盘。

        """
        Condition.not_none(close, "close")

        cdef SimulationModule module
        for module in self.modules:
            module.pre_process(close)

        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(close.instrument_id)
        if matching_engine is None:
            instrument = self.cache.instrument(close.instrument_id)
            if instrument is None:
                raise RuntimeError(f"未找到 {close.instrument_id} 的撮合引擎")
 
            self.add_instrument(instrument)
            matching_engine = self._matching_engines[close.instrument_id]

        matching_engine.process_instrument_close(close)

    cpdef void process(self, uint64_t ts_now):
        """
        处理交易所到给定时间。

        所有待处理的指令将与所有模拟模块一起处理。

        Parameters
        ----------
        ts_now : uint64_t
            当前 UNIX 时间戳（纳秒）。

        """
        self._clock.set_time(ts_now)

        cdef:
            uint64_t ts
        while self._inflight_queue:
            # 查看下一条在途消息的时间戳
            ts = self._inflight_queue[0][0][0]
            if ts <= ts_now:
                # 将消息放入队列进行处理
                self._message_queue.appendleft(self._inflight_queue.pop(0)[1])
                self._inflight_counter.pop(ts, None)
            else:
                break

        cdef TradingCommand command
        while self._message_queue:
            command = self._message_queue.pop()
            self._process_trading_command(command)
 
        # 遍历模块
        cdef SimulationModule module
        for module in self.modules:
            module.process(ts_now)

    cpdef void reset(self):
        """
        重置模拟交易所。
 
        所有有状态字段都将重置为其初始值。
        """
        self._log.debug(f"正在重置")

        for module in self.modules:
            module.reset()

        self._generate_fresh_account_state()

        for matching_engine in self._matching_engines.values():
            matching_engine.reset()

        self._message_queue = deque()
        self._inflight_queue.clear()
        self._inflight_counter.clear()
 
        self._log.info("已重置")

    cdef void _process_trading_command(self, TradingCommand command):
 
        cdef OrderMatchingEngine matching_engine = self._matching_engines.get(command.instrument_id)
        if matching_engine is None:
            raise RuntimeError(f"无法处理指令：未找到 {command.instrument_id} 的撮合引擎")

        cdef:
            Order order
            list[Order] orders
        if isinstance(command, SubmitOrder):
            matching_engine.process_order(command.order, self.exec_client.account_id)
        elif isinstance(command, SubmitOrderList):
            for order in command.order_list.orders:
                matching_engine.process_order(order, self.exec_client.account_id)
        elif isinstance(command, ModifyOrder):
            # 检查订单是否处于 SUBMITTED 状态或具有先前 SUBMITTED 状态的 PENDING_UPDATE 状态
            # （挂单尚未到达撮合引擎）
            order = self.cache.order(command.client_order_id)
            if (order is not None and
                (order.status_c() == OrderStatus.SUBMITTED or
                 (order.status_c() == OrderStatus.PENDING_UPDATE and order._previous_status == OrderStatus.SUBMITTED))):
                # 为尚未发送到撮合引擎的挂单在本地处理修改
                self._process_modify_submitted_order(command)
            else:
                matching_engine.process_modify(command, self.exec_client.account_id)
        elif isinstance(command, CancelOrder):
            matching_engine.process_cancel(command, self.exec_client.account_id)
        elif isinstance(command, CancelAllOrders):
            matching_engine.process_cancel_all(command, self.exec_client.account_id)
        elif isinstance(command, BatchCancelOrders):
            matching_engine.process_batch_cancel(command, self.exec_client.account_id)

    cdef void _process_modify_submitted_order(self, ModifyOrder command):
        cdef Order order = self.cache.order(command.client_order_id)
        if order is None:
            self._generate_order_modify_rejected(
                command.trader_id,
                command.strategy_id,
                command.instrument_id,
                command.client_order_id,
                None,
                f"未找到 {command.client_order_id!r}",
                self.exec_client.account_id,
            )
            return
 
        # 直接将修改应用于订单
        cdef:
            Quantity new_quantity = command.quantity if command.quantity is not None else order.quantity
            Price new_price = command.price if command.price is not None else (order.price if hasattr(order, 'price') else None)
            Price new_trigger_price = command.trigger_price if command.trigger_price is not None else (order.trigger_price if hasattr(order, 'trigger_price') else None)
 
        # 生成 OrderUpdated 事件
        self._generate_order_updated(
            order,
            new_quantity,
            new_price,
            new_trigger_price,
        )

    cdef void _generate_order_modify_rejected(
        self,
        TraderId trader_id,
        StrategyId strategy_id,
        InstrumentId instrument_id,
        ClientOrderId client_order_id,
        VenueOrderId venue_order_id,
        str reason,
        AccountId account_id,
    ):
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderModifyRejected event = OrderModifyRejected(
            trader_id=trader_id,
            strategy_id=strategy_id,
            instrument_id=instrument_id,
            client_order_id=client_order_id,
            venue_order_id=venue_order_id,
            account_id=account_id,
            reason=reason,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_updated(
        self,
        Order order,
        Quantity quantity,
        Price price,
        Price trigger_price,
    ):
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderUpdated event = OrderUpdated(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=order.venue_order_id,
            account_id=order.account_id,
            quantity=quantity,
            price=price,
            trigger_price=trigger_price,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

# -- EVENT GENERATORS -----------------------------------------------------------------------------

    cdef void _generate_fresh_account_state(self):
        cdef list[AccountBalance] balances = [
            AccountBalance(
                total=money,
                locked=Money(0, money.currency),
                free=money,
            )
            for money in self.starting_balances
        ]

        self.exec_client.generate_account_state(
            balances=balances,
            margins=[],
            reported=True,
            ts_event=self._clock.timestamp_ns(),
        )
 
        # 设置杠杆和保证金模型
        cdef Account account = self.get_account()
        if account.is_margin_account:
            account.set_default_leverage(self.default_leverage)
 
            # 设置特定工具的杠杆
            for instrument_id, leverage in self.leverages.items():
                account.set_leverage(instrument_id, leverage)
 
            # 如果提供了保证金模型，则进行设置
            if self.margin_model is not None:
                account.set_margin_model(self.margin_model)


cdef class OrderMatchingEngine:
    """
    为单个市场提供订单撮合引擎。

    Parameters
    ----------
    instrument : Instrument
        撮合引擎的市场工具。
    raw_id : uint32_t
        工具的原始整数 ID。
    fill_model : FillModel
        撮合引擎的成交模型。
    fee_model : FeeModel
        撮合引擎的费用模型。
    book_type : BookType
        引擎的订单簿类型。
    oms_type : OmsType
        撮合引擎的订单管理系统类型。决定场所持仓 ID 的生成和处理。
    account_type : AccountType
        撮合引擎的账户类型。根据工具确定允许的执行方式。
    msgbus : MessageBus
        撮合引擎的消息总线。
    cache : CacheFacade
        撮合引擎的只读缓存。
    clock : TestClock
        撮合引擎的时钟。
    logger : Logger
        撮合引擎的日志记录器。
    bar_execution : bool, default True
        是否应由撮合引擎处理 Bar 数据（并推动市场）。
    trade_execution : bool, default False
        是否应由撮合引擎处理 Trade 数据（并推动市场）。
    liquidity_consumption : bool, default False
        是否应按价格水平跟踪流动性消耗。
    reject_stop_orders : bool, default True
        如果提交时止损单已在市场中，是否拒绝。
    support_gtd_orders : bool, default True
        场所是否支持 GTD（Good Till Date）有效时间的订单。
    support_contingent_orders : bool, default True
        场所是否支持/遵循条件订单。
        如果为 False，则预期策略将管理任何条件订单。
    use_position_ids : bool, default True
        是否在订单成交时生成场所持仓 ID。
    use_random_ids : bool, default False
        是否所有场所生成的标识符都是随机 UUID4。
    use_reduce_only : bool, default True
        是否遵循订单上的 `reduce_only` 执行指令。
    bar_adaptive_high_low_ordering : bool, default False
        决定是否根据启发式算法自适应处理 Bar 价格顺序。
        此设置仅在 `bar_execution` 为 True 时相关。
        如果为 False，Bar 价格始终按固定顺序处理：Open, High, Low, Close。
        如果为 True，处理顺序随启发式算法调整：
        - 如果 High 比 Low 更接近 Open，则处理顺序为 Open, High, Low, Close。
        - 如果 Low 比 High 更接近 Open，则处理顺序为 Open, Low, High, Close。

    """

    def __init__(
        self,
        Instrument instrument not None,
        uint32_t raw_id,
        FillModel fill_model not None,
        FeeModel fee_model not None,
        BookType book_type,
        OmsType oms_type,
        AccountType account_type,
        MessageBus msgbus not None,
        CacheFacade cache not None,
        TestClock clock not None,
        bint reject_stop_orders = True,
        bint support_gtd_orders = True,
        bint support_contingent_orders = True,
        bint oto_full_trigger = False,
        bint use_position_ids = True,
        bint use_random_ids = False,
        bint use_reduce_only = True,
        bint use_market_order_acks = False,
        bint bar_execution = True,
        bint bar_adaptive_high_low_ordering = False,
        bint trade_execution = False,
        bint liquidity_consumption = False,
        price_protection_points=None,
    ) -> None:
        self._clock = clock
        self._log = Logger(name=f"{type(self).__name__}({instrument.id.venue})")
        self.msgbus = msgbus
        self.cache = cache

        self.venue = instrument.id.venue
        self.instrument = instrument
        self.raw_id = raw_id
        self.book_type = book_type
        self.oms_type = oms_type
        self.account_type = account_type
        self.market_status = MarketStatus.OPEN

        self._instrument_has_expiration = instrument.instrument_class in EXPIRING_INSTRUMENT_CLASSES
        self._instrument_close = None
        self._reject_stop_orders = reject_stop_orders
        self._support_gtd_orders = support_gtd_orders
        self._support_contingent_orders = support_contingent_orders
        self._oto_full_trigger = oto_full_trigger
        self._use_position_ids = use_position_ids
        self._use_random_ids = use_random_ids
        self._use_reduce_only = use_reduce_only
        self._use_market_order_acks = use_market_order_acks
        self._bar_execution = bar_execution
        self._bar_adaptive_high_low_ordering = bar_adaptive_high_low_ordering
        self._trade_execution = trade_execution
        self._liquidity_consumption = liquidity_consumption
        self._price_protection_points = price_protection_points if price_protection_points is not None else 0

        self._fill_model = fill_model
        self._fee_model = fee_model
        self._book = OrderBook(
            instrument_id=instrument.id,
            book_type=book_type,
        )

        self._account_ids: dict[TraderId, AccountId]  = {}
        self._execution_bar_types: dict[InstrumentId, BarType]  =  {}
        self._execution_bar_deltas: dict[BarType, timedelta]  =  {}
        self._cached_filled_qty: dict[ClientOrderId, Quantity] = {}

        # 市场
        self._core = MatchingCore(
            instrument_id=instrument.id,
            price_increment=instrument.price_increment,
            trigger_stop_order=self.trigger_stop_order,
            fill_market_order=self.fill_market_order,
            fill_limit_order=self.fill_limit_order,
        )
        self._price_prec = instrument.price_precision
        self._size_prec = instrument.size_precision

        self._target_bid = 0
        self._target_ask = 0
        self._target_last = 0
        self._has_targets = False
        self._last_bid_bar: Bar | None = None
        self._last_ask_bar: Bar | None = None
        self._last_trade_size: Quantity | None = None
        self._fill_at_market = True  # 以市价（而非触发价）成交止损单
        self._bid_consumption = {}
        self._ask_consumption = {}
        self._trade_consumption = 0

        self._position_count = 0
        self._order_count = 0
        self._execution_count = 0

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}("
            f"venue={self.venue.value}, "
            f"instrument_id={self.instrument.id.value}, "
            f"raw_id={self.raw_id})"
        )
 
    cpdef void reset(self):
        self._log.debug(f"正在重置撮合引擎 {self.instrument.id}")
 
        self._book.clear(0, 0)
        self._account_ids.clear()
        self._execution_bar_types.clear()
        self._execution_bar_deltas.clear()
        self._cached_filled_qty.clear()
        self._core.reset()
        self._target_bid = 0
        self._target_ask = 0
        self._target_last = 0
        self._has_targets = False
        self._last_bid_bar = None
        self._last_ask_bar = None
        self._last_trade_size = None
        self._bid_consumption.clear()
        self._ask_consumption.clear()
        self._trade_consumption = 0
 
        self._position_count = 0
        self._order_count = 0
        self._execution_count = 0
 
        self._log.info(f"已重置撮合引擎 {self.instrument.id}")

    cpdef void set_fill_model(self, FillModel fill_model):
        """
        将成交模型设置为给定模型。

        Parameters
        ----------
        fill_model : FillModel
            要设置的成交模型。

        """
        Condition.not_none(fill_model, "fill_model")
 
        self._fill_model = fill_model
 
        self._log.debug(f"已将 `FillModel` 更改为 {self._fill_model}")

    cpdef void update_instrument(self, Instrument instrument):
        """
        使用给定的工具更新撮合引擎当前的工具定义。

        Parameters
        ----------
        instrument : Instrument
            要更新的工具定义。

        """
        Condition.not_none(instrument, "instrument")
        Condition.equal(instrument.id, self.instrument.id, "instrument.id", "self.instrument.id")

        self.instrument = instrument
        self._price_prec = instrument.price_precision
        self._size_prec = instrument.size_precision
 
        self._log.debug(f"已更新 {instrument.id} 的工具定义")

# -- QUERIES --------------------------------------------------------------------------------------

    cpdef Price best_bid_price(self):
        """
        返回给定工具 ID 的最佳买入价格（如果找到）。

        Returns
        -------
        Price or ``None``

        """
        return self._book.best_bid_price()

    cpdef Price best_ask_price(self):
        """
        返回给定工具 ID 的最佳卖出价格（如果找到）。

        Returns
        -------
        Price or ``None``

        """
        return self._book.best_ask_price()

    cpdef OrderBook get_book(self):
        """
        返回内部订单簿。

        Returns
        -------
        OrderBook

        """
        return self._book

    cpdef list[Order] get_open_orders(self):
        """
        返回撮合引擎中的挂单。

        Returns
        -------
        list[Order]

        """
        return self.get_open_bid_orders() + self.get_open_ask_orders()

    cpdef list[Order] get_open_bid_orders(self):
        """
        返回撮合引擎中的买入挂单。

        Returns
        -------
        list[Order]

        """
        return self._core.get_orders_bid()

    cpdef list[Order] get_open_ask_orders(self):
        """
        返回交易所的卖出挂单。

        Returns
        -------
        list[Order]
 
        """
        return self._core.get_orders_ask()

    cpdef bint order_exists(self, ClientOrderId client_order_id):
        return self._core.order_exists(client_order_id)

# -- DATA PROCESSING ------------------------------------------------------------------------------

    cpdef void process_order_book_delta(self, OrderBookDelta delta):
        """
        处理给定订单簿增量的交易所市场。

        Parameters
        ----------
        delta : OrderBookDelta
            要处理的订单簿增量。

        Raises
        ------
        RuntimeError
            如果增量价格精度与撮合引擎的工具不匹配。
        RuntimeError
            如果增量大小精度与撮合引擎的工具不匹配。

        """
        Condition.not_none(delta, "delta")

        if is_logging_initialized():
            self._log.debug(f"正在处理 {delta!r}")

        # 验证 ADD 和 UPDATE 动作的精度
        if delta._mem.action == BookAction.ADD or delta._mem.action == BookAction.UPDATE:
            if delta._mem.order.price.precision != self._price_prec:
                raise RuntimeError(
                    f"无效的增量价格精度={delta._mem.order.price.precision} "
                    f"与 instrument.price_precision={self._price_prec} 不匹配",
                )
            if delta._mem.order.size.precision != self._size_prec:
                raise RuntimeError(
                    f"无效的增量数量精度={delta._mem.order.size.precision} "
                    f"与 instrument.size_precision={self._size_prec} 不匹配",
                )

        # 发生快照 (F_SNAPSHOT = 32) 或 CLEAR 动作时重置消耗跟踪
        if self._liquidity_consumption and (
            (delta._mem.flags & 32) or delta._mem.action == BookAction.CLEAR
        ):
            self._bid_consumption.clear()
            self._ask_consumption.clear()

        # 在 UPDATE 或 DELETE（价位更改或删除）时清除消耗跟踪
        if self._liquidity_consumption and (
            delta._mem.action == BookAction.UPDATE or delta._mem.action == BookAction.DELETE
        ):
            if delta._mem.order.side == OrderSide.SELL:
                self._ask_consumption.pop(delta._mem.order.price.raw, None)
            elif delta._mem.order.side == OrderSide.BUY:
                self._bid_consumption.pop(delta._mem.order.price.raw, None)

        self._book.apply_delta(delta)

        self.iterate(delta.ts_init)

    cpdef void process_order_book_deltas(self, OrderBookDeltas deltas):
        """
        处理给定订单簿增量的交易所市场。

        Parameters
        ----------
        deltas : OrderBookDeltas
            要处理的订单簿增量。

        Raises
        ------
        RuntimeError
            如果任何增量价格精度与撮合引擎的工具不匹配。
        RuntimeError
            如果任何增量大小精度与撮合引擎的工具不匹配。

        """
        Condition.not_none(deltas, "deltas")

        if is_logging_initialized():
            self._log.debug(f"正在处理 {deltas!r}")

        # 验证 ADD 和 UPDATE 动作的精度
        cdef bint has_snapshot_or_clear = False
        cdef OrderBookDelta delta
        for delta in deltas.deltas:
            if delta._mem.action == BookAction.ADD or delta._mem.action == BookAction.UPDATE:
                if delta._mem.order.price.precision != self._price_prec:
                    raise RuntimeError(
                        f"无效的增量价格精度={delta._mem.order.price.precision} "
                        f"与 instrument.price_precision={self._price_prec} 不匹配",
                    )
                if delta._mem.order.size.precision != self._size_prec:
                    raise RuntimeError(
                        f"无效的增量数量精度={delta._mem.order.size.precision} "
                        f"与 instrument.size_precision={self._size_prec} 不匹配",
                    )
            if (delta._mem.flags & 32) or delta._mem.action == BookAction.CLEAR:
                has_snapshot_or_clear = True

            # 在 UPDATE 或 DELETE（价位更改或删除）时清除消耗跟踪
            if self._liquidity_consumption and (
                delta._mem.action == BookAction.UPDATE or delta._mem.action == BookAction.DELETE
            ):
                if delta._mem.order.side == OrderSide.SELL:
                    self._ask_consumption.pop(delta._mem.order.price.raw, None)
                elif delta._mem.order.side == OrderSide.BUY:
                    self._bid_consumption.pop(delta._mem.order.price.raw, None)

        # 发生快照 (F_SNAPSHOT = 32) 或 CLEAR 动作时重置消耗跟踪
        if self._liquidity_consumption and has_snapshot_or_clear:
            self._bid_consumption.clear()
            self._ask_consumption.clear()

        self._book.apply_deltas(deltas)

        self.iterate(deltas.ts_init)

    cpdef void process_order_book_depth10(self, OrderBookDepth10 depth):
        """
        处理给定订单簿深度的交易所市场。

        Parameters
        ----------
        depth : OrderBookDepth10
            要处理的订单簿深度。

        Raises
        ------
        RuntimeError
            如果任何订单价格精度与撮合引擎的工具不匹配。
        RuntimeError
            如果任何订单大小精度与撮合引擎的工具不匹配。

        """
        Condition.not_none(depth, "depth")

        if is_logging_initialized():
            self._log.debug(f"正在处理 {depth!r}")

        # 为非空订单验证精度
        cdef BookOrder order
        for order in depth.bids:
            if order._mem.side == OrderSide.NO_ORDER_SIDE:
                continue  # 跳过空订单
            if order._mem.price.precision != self._price_prec:
                raise RuntimeError(
                    f"无效的深度买入价格精度={order._mem.price.precision} "
                    f"与 instrument.price_precision={self._price_prec} 不匹配",
                )
            if order._mem.size.precision != self._size_prec:
                raise RuntimeError(
                    f"无效的深度买入数量精度={order._mem.size.precision} "
                    f"与 instrument.size_precision={self._size_prec} 不匹配",
                )

        for order in depth.asks:
            if order._mem.side == OrderSide.NO_ORDER_SIDE:
                continue  # 跳过空订单
            if order._mem.price.precision != self._price_prec:
                raise RuntimeError(
                    f"无效的深度卖出价格精度={order._mem.price.precision} "
                    f"与 instrument.price_precision={self._price_prec} 不匹配",
                )
            if order._mem.size.precision != self._size_prec:
                raise RuntimeError(
                    f"无效的深度卖出数量精度={order._mem.size.precision} "
                    f"与 instrument.size_precision={self._size_prec} 不匹配",
                )

        # 发生快照 (F_SNAPSHOT = 32) 时重置消耗跟踪
        if self._liquidity_consumption and (depth._mem.flags & 32):
            self._bid_consumption.clear()
            self._ask_consumption.clear()

        self._book.apply_depth(depth)

        self.iterate(depth.ts_init)

    cpdef void process_quote_tick(self, QuoteTick tick):
        """
        处理给定报价 Tick 的交易所市场。

        仅当场所的 `book_type` 为 'L1_MBP' 时，内部订单簿才会更新。

        Parameters
        ----------
        tick : QuoteTick
            要处理的 Tick。

        Raises
        ------
        RuntimeError
            如果价格精度与撮合引擎的工具不匹配。
        RuntimeError
            如果大小精度与撮合引擎的工具不匹配。

        """
        Condition.not_none(tick, "tick")

        if is_logging_initialized():
            self._log.debug(f"正在处理 {tick!r}")
 
        # 验证精度
        if tick._mem.bid_price.precision != self._price_prec:
            raise RuntimeError(
                f"无效的 {tick.bid_price.precision=}，与 instrument.price_precision={self._price_prec} 不匹配",
            )
        if tick._mem.ask_price.precision != self._price_prec:
            raise RuntimeError(
                f"无效的 {tick.ask_price.precision=}，与 instrument.price_precision={self._price_prec} 不匹配",
            )
        if tick._mem.bid_size.precision != self._size_prec:
            raise RuntimeError(
                f"无效的 {tick.bid_size.precision=}，与 instrument.size_precision={self._size_prec} 不匹配",
            )
        if tick._mem.ask_size.precision != self._size_prec:
            raise RuntimeError(
                f"无效的 {tick.ask_size.precision=}，与 instrument.size_precision={self._size_prec} 不匹配",
            )

        if self.book_type == BookType.L1_MBP:
            self._book.update_quote_tick(tick)

        self.iterate(tick.ts_init)

    cpdef void process_trade_tick(self, TradeTick tick):
        """
        处理给定成交 Tick 的交易所市场。

        仅当场所的 `book_type` 为 'L1_MBP' 时，内部订单簿才会更新。

        Parameters
        ----------
        tick : TradeTick
            要处理的 Tick。

        Raises
        ------
        RuntimeError
            如果成交价格精度与撮合引擎的工具不匹配。
        RuntimeError
            如果成交大小精度与撮合引擎的工具不匹配。

        """
        Condition.not_none(tick, "tick")

        if is_logging_initialized():
            self._log.debug(f"正在处理 {tick!r}")
 
        # 验证精度
        if tick._mem.price.precision != self._price_prec:
            raise RuntimeError(
                f"无效的 {tick.price.precision=}，与 instrument.price_precision={self._price_prec} 不匹配",
            )
        if tick._mem.size.precision != self._size_prec:
            raise RuntimeError(
                f"无效的 {tick.size.precision=}，与 instrument.size_precision={self._size_prec} 不匹配",
            )

        if self.book_type == BookType.L1_MBP:
            self._book.update_trade_tick(tick)

        cdef AggressorSide aggressor_side = AggressorSide.NO_AGGRESSOR
        cdef PriceRaw price_raw = tick._mem.price.raw
        cdef PriceRaw original_bid = 0
        cdef PriceRaw original_ask = 0

        self._core.set_last_raw(price_raw)

        if self._trade_execution:
            aggressor_side = tick.aggressor_side

            # 根据成交更新自然侧
            if aggressor_side == AggressorSide.BUYER:
                if not self._core.is_ask_initialized or price_raw > self._core.ask_raw:
                    self._core.set_ask_raw(price_raw)
                if not self._core.is_bid_initialized or price_raw < self._core.bid_raw:
                    self._core.set_bid_raw(price_raw)
            elif aggressor_side == AggressorSide.SELLER:
                if not self._core.is_bid_initialized or price_raw < self._core.bid_raw:
                    self._core.set_bid_raw(price_raw)
                if not self._core.is_ask_initialized or price_raw > self._core.ask_raw:
                    self._core.set_ask_raw(price_raw)
            elif aggressor_side == AggressorSide.NO_AGGRESSOR:
                if not self._core.is_bid_initialized or price_raw <= self._core.bid_raw:
                    self._core.set_bid_raw(price_raw)
                if not self._core.is_ask_initialized or price_raw >= self._core.ask_raw:
                    self._core.set_ask_raw(price_raw)
            else:
                aggressor_side_str = aggressor_side_to_str(aggressor_side)
                raise RuntimeError(  # pragma: no cover (设计时错误)
                    f"成交执行时 `AggressorSide` 无效，为 {aggressor_side_str}",  # pragma: no cover
                )

            # 瞬时覆盖：暂时将对侧拖向成交价格
            original_bid = self._core.bid_raw
            original_ask = self._core.ask_raw

            if aggressor_side == AggressorSide.SELLER and price_raw < original_ask:
                self._core.set_ask_raw(price_raw)
            elif aggressor_side == AggressorSide.BUYER and price_raw > original_bid:
                self._core.set_bid_raw(price_raw)

            self._last_trade_size = tick.size
            self._trade_consumption = 0

        self.iterate(tick.ts_init, aggressor_side)

        if self._trade_execution:
            self._last_trade_size = None
            self._trade_consumption = 0

            if aggressor_side == AggressorSide.SELLER and price_raw < original_ask:
                self._core.set_ask_raw(original_ask)
            elif aggressor_side == AggressorSide.BUYER and price_raw > original_bid:
                self._core.set_bid_raw(original_bid)

    cpdef void process_bar(self, Bar bar):
        """
        处理给定 Bar 的交易所市场。

        通过拍卖挂单来模拟市场动态。

        Parameters
        ----------
        bar : Bar
            要处理的 Bar。

        Raises
        ------
        RuntimeError
            如果价格精度与撮合引擎的工具不匹配。
        RuntimeError
            如果大小精度与撮合引擎的工具不匹配。

        """
        Condition.not_none(bar, "bar")

        if not self._bar_execution:
            return

        if self.book_type != BookType.L1_MBP:
            return  # 只能通过 Bar 处理 L1 订单簿

        cdef BarType bar_type = bar.bar_type
        if bar_type.aggregation_source == AggregationSource.INTERNAL:
            return  # 不处理内部聚合的 Bar

        # 验证精度
        if bar._mem.open.precision != self._price_prec:
            raise RuntimeError(
                f"无效的 {bar.open.precision=}，与 instrument.price_precision={self._price_prec} 不匹配",
            )
        if bar._mem.high.precision != self._price_prec:
            raise RuntimeError(
                f"无效的 {bar.high.precision=}，与 instrument.price_precision={self._price_prec} 不匹配",
            )
        if bar._mem.low.precision != self._price_prec:
            raise RuntimeError(
                f"无效的 {bar.low.precision=}，与 instrument.price_precision={self._price_prec} 不匹配",
            )
        if bar._mem.close.precision != self._price_prec:
            raise RuntimeError(
                f"无效的 {bar.close.precision=}，与 instrument.price_precision={self._price_prec} 不匹配",
            )
        if bar._mem.volume.precision != self._size_prec:
            raise RuntimeError(
                f"无效的 {bar.volume.precision=}，与 instrument.size_precision={self._size_prec} 不匹配",
            )

        cdef InstrumentId instrument_id = bar_type.instrument_id
        cdef BarType execution_bar_type = self._execution_bar_types.get(instrument_id)

        if execution_bar_type is None:
            execution_bar_type = bar_type
            self._execution_bar_types[instrument_id] = bar_type
            self._execution_bar_deltas[bar_type] = bar_type.spec.timedelta

        if execution_bar_type != bar_type:
            bar_type_timedelta = self._execution_bar_deltas.get(bar_type)

            if bar_type_timedelta is None:
                bar_type_timedelta = bar_type.spec.timedelta
                self._execution_bar_deltas[bar_type] = bar_type_timedelta

            if self._execution_bar_deltas[execution_bar_type] >= bar_type_timedelta:
                self._execution_bar_types[instrument_id] = bar_type
            else:
                return

        if is_logging_initialized():
            self._log.debug(f"正在处理 {bar!r}")

        cdef PriceType price_type = bar_type.spec.price_type
        if price_type == PriceType.LAST or price_type == PriceType.MID:
            self._process_trade_ticks_from_bar(bar)
        elif price_type == PriceType.BID:
            self._last_bid_bar = bar
            self._process_quote_ticks_from_bar()
        elif price_type == PriceType.ASK:
            self._last_ask_bar = bar
            self._process_quote_ticks_from_bar()
        else:
            raise RuntimeError(  # pragma: no cover (设计时错误)
                f"无效的 `PriceType`，为 {price_type}",  # pragma: no cover
            )

    cpdef void process_status(self, MarketStatusAction status):
        """
        处理交易所状态。

        Parameters
        ----------
        status : MarketStatusAction
            要处理的状态动作。

        """
        if (self.market_status, status) == (MarketStatus.CLOSED, MarketStatusAction.TRADING):
            self.market_status = MarketStatus.OPEN
        elif (self.market_status, status) == (MarketStatus.CLOSED, MarketStatusAction.PRE_OPEN):
            self.market_status = MarketStatus.OPEN

    cpdef void process_instrument_close(self, InstrumentClose close):
        """
        处理工具收盘。

        Parameters
        ----------
        close : InstrumentClose
            要处理的收盘价格。

        """
        if close.instrument_id != self.instrument.id:
            self._log.warning(f"接收到未知 instrument_id 的工具收盘： {close.instrument_id}")
            return

        if close.close_type == InstrumentCloseType.CONTRACT_EXPIRED:
            self._instrument_close = close
            self.iterate(close.ts_init)

    cdef void _process_trade_ticks_from_bar(self, Bar bar):
        cdef QuantityRaw min_size_raw = self.instrument.size_increment._mem.raw
        cdef QuantityRaw quarter_raw
        cdef QuantityRaw close_raw
        quarter_raw, close_raw = compute_bar_quarter_sizes(
            bar._mem.volume.raw,
            min_size_raw,
        )

        cdef Quantity size = Quantity.from_raw_c(quarter_raw, bar._mem.volume.precision)
        cdef Quantity close_size = Quantity.from_raw_c(close_raw, bar._mem.volume.precision)

        # 创建基础成交 Tick 模板
        cdef TradeTick tick = self._create_base_trade_tick(bar, size)

        # 为每个价位进行处理
        cdef bint process_high_first = (
            not self._bar_adaptive_high_low_ordering
            or abs(bar._mem.high.raw - bar._mem.open.raw) < abs(bar._mem.low.raw - bar._mem.open.raw)
        )
        self._process_trade_bar_open(bar, tick)

        if process_high_first:
            self._process_trade_bar_high(bar, tick)
            self._process_trade_bar_low(bar, tick)
        else:
            self._process_trade_bar_low(bar, tick)
            self._process_trade_bar_high(bar, tick)

        self._process_trade_bar_close(bar, tick, close_size)

        # Bar 处理后重置标志，以确保 Bar 间的行为正确
        self._fill_at_market = True

    cdef TradeTick _create_base_trade_tick(self, Bar bar, Quantity size):
        return TradeTick(
            bar.bar_type.instrument_id,
            bar.open,
            size,
            AggressorSide.BUYER if not self._core.is_last_initialized or bar._mem.open.raw > self._core.last_raw else AggressorSide.SELLER,
            self._generate_trade_id(),
            bar.ts_init,
            bar.ts_init,
        )

    cdef void _process_trade_bar_open(self, Bar bar, TradeTick tick):
        if not self._core.is_last_initialized or bar._mem.open.raw != self._core.last_raw:
            if is_logging_initialized():
                self._log.debug(f"正在使用开盘价 {bar.open} 更新")

            self._fill_at_market = True  # 与上一根 Bar 之间存在缺口
            self._book.update_trade_tick(tick)
            self.iterate(tick.ts_init)
            self._core.set_last_raw(bar._mem.open.raw)

    cdef void _process_trade_bar_high(self, Bar bar, TradeTick tick):
        if bar._mem.high.raw > self._core.last_raw:
            if is_logging_initialized():
                self._log.debug(f"正在使用最高价 {bar.high} 更新")

            self._fill_at_market = False  # 市场价格移动穿过
            tick._mem.price = bar._mem.high
            tick._mem.aggressor_side = AggressorSide.BUYER
            tick._mem.trade_id = trade_id_new(pystr_to_cstr(self._generate_trade_id_str()))
            self._book.update_trade_tick(tick)
            self.iterate(tick.ts_init)
            self._core.set_last_raw(bar._mem.high.raw)

    cdef void _process_trade_bar_low(self, Bar bar, TradeTick tick):
        if bar._mem.low.raw < self._core.last_raw:
            if is_logging_initialized():
                self._log.debug(f"正在使用最低价 {bar.low} 更新")

            self._fill_at_market = False  # 市场价格移动穿过
            tick._mem.price = bar._mem.low
            tick._mem.aggressor_side = AggressorSide.SELLER
            tick._mem.trade_id = trade_id_new(pystr_to_cstr(self._generate_trade_id_str()))
            self._book.update_trade_tick(tick)
            self.iterate(tick.ts_init)
            self._core.set_last_raw(bar._mem.low.raw)

    cdef void _process_trade_bar_close(self, Bar bar, TradeTick tick, Quantity close_size = None):
        if bar._mem.close.raw != self._core.last_raw:
            if is_logging_initialized():
                self._log.debug(f"正在使用收盘价 {bar.close} 更新")

            self._fill_at_market = False  # 市场价格移动穿过
            tick._mem.price = bar._mem.close
            if close_size is not None:
                tick._mem.size = close_size._mem
            if bar._mem.close.raw > self._core.last_raw:
                tick._mem.aggressor_side = AggressorSide.BUYER
            else:
                tick._mem.aggressor_side = AggressorSide.SELLER
            tick._mem.trade_id = trade_id_new(pystr_to_cstr(self._generate_trade_id_str()))
            self._book.update_trade_tick(tick)
            self.iterate(tick.ts_init)
            self._core.set_last_raw(bar._mem.close.raw)

    cdef void _process_quote_ticks_from_bar(self):
        if self._last_bid_bar is None or self._last_ask_bar is None:
            return  # 等待下一根 Bar
 
        if self._last_bid_bar.ts_init != self._last_ask_bar.ts_init:
            return  # 等待下一根 Bar

        cdef QuantityRaw min_size_raw = self.instrument.size_increment._mem.raw
        cdef QuantityRaw bid_quarter
        cdef QuantityRaw bid_close_raw
        cdef QuantityRaw ask_quarter
        cdef QuantityRaw ask_close_raw

        bid_quarter, bid_close_raw = compute_bar_quarter_sizes(
            self._last_bid_bar._mem.volume.raw,
            min_size_raw,
        )
        ask_quarter, ask_close_raw = compute_bar_quarter_sizes(
            self._last_ask_bar._mem.volume.raw,
            min_size_raw,
        )

        cdef Quantity bid_size = Quantity.from_raw_c(bid_quarter, self._last_bid_bar._mem.volume.precision)
        cdef Quantity ask_size = Quantity.from_raw_c(ask_quarter, self._last_ask_bar._mem.volume.precision)
        cdef Quantity bid_close_size = Quantity.from_raw_c(bid_close_raw, self._last_bid_bar._mem.volume.precision)
        cdef Quantity ask_close_size = Quantity.from_raw_c(ask_close_raw, self._last_ask_bar._mem.volume.precision)

        # 创建基础报价 Tick 模板
        cdef QuoteTick tick = self._create_base_quote_tick(bid_size, ask_size)
 
        # 处理每个价位
        cdef bint process_high_first = (
            not self._bar_adaptive_high_low_ordering
            or abs(self._last_bid_bar._mem.high.raw - self._last_bid_bar._mem.open.raw) < abs(self._last_bid_bar._mem.low.raw - self._last_bid_bar._mem.open.raw)
        )
        self._process_quote_bar_open(tick)

        if process_high_first:
            self._process_quote_bar_high(tick)
            self._process_quote_bar_low(tick)
        else:
            self._process_quote_bar_low(tick)
            self._process_quote_bar_high(tick)

        self._process_quote_bar_close(tick, bid_close_size, ask_close_size)
 
        self._last_bid_bar = None
        self._last_ask_bar = None
 
        # Bar 处理后重置标志，以确保 Bar 间的行为正确
        self._fill_at_market = True

    cdef QuoteTick _create_base_quote_tick(self, Quantity bid_size, Quantity ask_size):
        return QuoteTick(
            self._book.instrument_id,
            self._last_bid_bar.open,
            self._last_ask_bar.open,
            bid_size,
            ask_size,
            self._last_bid_bar.ts_init,
            self._last_ask_bar.ts_init,
        )

    cdef void _process_quote_bar_open(self, QuoteTick tick):
        self._fill_at_market = True  # 与上一根 Bar 之间存在缺口
        self._book.update_quote_tick(tick)
        self.iterate(tick.ts_init)

    cdef void _process_quote_bar_high(self, QuoteTick tick):
        self._fill_at_market = False  # 市场价格移动穿过
        tick._mem.bid_price = self._last_bid_bar._mem.high
        tick._mem.ask_price = self._last_ask_bar._mem.high
        self._book.update_quote_tick(tick)
        self.iterate(tick.ts_init)

    cdef void _process_quote_bar_low(self, QuoteTick tick):
        self._fill_at_market = False  # 市场价格移动穿过
        tick._mem.bid_price = self._last_bid_bar._mem.low
        tick._mem.ask_price = self._last_ask_bar._mem.low
        self._book.update_quote_tick(tick)
        self.iterate(tick.ts_init)

    cdef void _process_quote_bar_close(self, QuoteTick tick, Quantity bid_close_size = None, Quantity ask_close_size = None):
        self._fill_at_market = False  # 市场价格移动穿过
        tick._mem.bid_price = self._last_bid_bar._mem.close
        tick._mem.ask_price = self._last_ask_bar._mem.close
        if bid_close_size is not None:
            tick._mem.bid_size = bid_close_size._mem
        if ask_close_size is not None:
            tick._mem.ask_size = ask_close_size._mem
        self._book.update_quote_tick(tick)
        self.iterate(tick.ts_init)

    # -- TRADING COMMANDS -----------------------------------------------------------------------------

    cpdef void process_order(self, Order order, AccountId account_id):
        if self._core.order_exists(order.client_order_id):
            return  # 已经处理过
 
        # 索引标识符
        self._account_ids[order.trader_id] = account_id

        cdef uint64_t now_ns
        if self._instrument_has_expiration:
            now_ns = self._clock.timestamp_ns()

            if now_ns < self.instrument.activation_ns:
                self._generate_order_rejected(
                    order,
                    f"合约 {self.instrument.id} 尚未激活，"
                    f"激活时间为 {format_iso8601(unix_nanos_to_dt(self.instrument.activation_ns))}"
                )
                return
            elif now_ns > self.instrument.expiration_ns:
                self._generate_order_rejected(
                    order,
                    f"合约 {self.instrument.id} 已过期，"
                    f"过期时间为 {format_iso8601(unix_nanos_to_dt(self.instrument.expiration_ns))}"
                )
                return

        cdef:
            Order parent
            Order contingenct_order
            ClientOrderId client_order_id
            OrderStatus parent_status
        if self._support_contingent_orders and order.parent_order_id is not None:
            parent = self.cache.order(order.parent_order_id)
            assert parent is not None and parent.contingency_type == ContingencyType.OTO, "未找到 OTO 父订单"

            parent_status = parent.status_c()

            if parent_status == OrderStatus.REJECTED and order.is_open_c():
                self._generate_order_rejected(order, f"拒绝来自 {parent.client_order_id} 的 OTO")
                return  # 订单被拒绝
            elif (
                parent_status == OrderStatus.ACCEPTED
                or parent_status == OrderStatus.TRIGGERED
                or (self._oto_full_trigger and parent_status == OrderStatus.PARTIALLY_FILLED)
            ):
                self._log.info(f"等待来自 {parent.client_order_id} 触发的待定 OTO {order.client_order_id}")
                return  # 等待触发
 
            if order.linked_order_ids is not None:
                # 检查条件订单是否仍然打开
                for client_order_id in order.linked_order_ids or []:
                    contingent_order = self.cache.order(client_order_id)
 
                    if contingent_order is None:
                        raise RuntimeError(f"找不到 {client_order_id!r} 的条件订单")  # pragma: no cover
 
                    if order.contingency_type == ContingencyType.OCO or order.contingency_type == ContingencyType.OUO:
                        if not order.is_closed_c() and contingent_order.is_closed_c():
                            self._generate_order_rejected(order, f"条件订单 {client_order_id} 已经关闭")
                            return  # 订单被拒绝

        # 检查订单数量精度（必须 <= 工具精度）
        if order.quantity._mem.precision > self._size_prec:
            self._generate_order_rejected(
                order,
                f"订单 {order.client_order_id} 的数量精度无效，"
                f"精度为 {order.quantity.precision} "
                f"而 {self.instrument.id} 的数量精度为 {self._size_prec}"
            )
            return  # 无效订单
 
        cdef Price price
        if order.has_price_c():
            # 检查订单价格精度（必须 <= 工具精度）
            price = order.price
 
            if price._mem.precision > self._price_prec:
                self._generate_order_rejected(
                    order,
                    f"订单 {order.client_order_id} 的价格精度无效，"
                    f"精度为 {price.precision} "
                    f"而 {self.instrument.id} 的价格精度为 {self._price_prec}"
                )
                return  # 无效订单

        cdef Price trigger_price
        if order.has_trigger_price_c():
            # 检查订单触发价格精度（必须 <= 工具精度）
            trigger_price = order.trigger_price
 
            if trigger_price._mem.precision > self._price_prec:
                self._generate_order_rejected(
                    order,
                    f"订单 {order.client_order_id} 的触发价格精度无效，"
                    f"精度为 {trigger_price.precision} "
                    f"而 {self.instrument.id} 的价格精度为 {self._price_prec}"
                )
                return  # 无效订单
 
        cdef Price activation_price
        if order.has_activation_price_c():
            # 检查订单激活价格精度（必须 <= 工具精度）
            activation_price = order.activation_price
 
            if activation_price._mem.precision > self._price_prec:
                self._generate_order_rejected(
                    order,
                    f"订单 {order.client_order_id} 的激活价格精度无效，"
                    f"精度为 {activation_price.precision} "
                    f"而 {self.instrument.id} 的价格精度为 {self._price_prec}"
                )
                return  # 无效订单

        cdef Position position = self.cache.position_for_order(order.client_order_id)

        cdef PositionId position_id
        if position is None and self.oms_type == OmsType.NETTING:
            position_id = PositionId(f"{order.instrument_id}-{order.strategy_id}")
            position = self.cache.position(position_id)
 
        # 检查是否在非保证金账户中卖空股票
        if (
            order.side == OrderSide.SELL
            and self.account_type != AccountType.MARGIN
            and isinstance(self.instrument, Equity)
            and (position is None or not order.would_reduce_only(position.side, position.quantity))
        ):
            self._generate_order_rejected(
                order,
                f"现金账户不允许卖空，持有头寸为 {position}，订单为 {order!r}"
            )
            return  # 无法卖空
 
        # 检查只减仓指令
        if self._use_reduce_only and order.is_reduce_only and not order.is_closed_c():
            if (
                not position
                or position.is_closed_c()
                or (order.is_buy_c() and position.is_long_c())
                or (order.is_sell_c() and position.is_short_c())
            ):
                self._generate_order_rejected(
                    order,
                    f"只减仓 (REDUCE_ONLY) {order.type_string_c()} {order.side_string_c()} 订单"
                    f"会导致仓位增加",
                )
                return  # 只减仓限制

        if order.order_type == OrderType.MARKET:
            self._process_market_order(order)
        elif order.order_type == OrderType.MARKET_TO_LIMIT:
            self._process_market_to_limit_order(order)
        elif order.order_type == OrderType.LIMIT:
            self._process_limit_order(order)
        elif order.order_type == OrderType.STOP_MARKET:
            self._process_stop_market_order(order)
        elif order.order_type == OrderType.STOP_LIMIT:
            self._process_stop_limit_order(order)
        elif order.order_type == OrderType.MARKET_IF_TOUCHED:
            self._process_market_if_touched_order(order)
        elif order.order_type == OrderType.LIMIT_IF_TOUCHED:
            self._process_limit_if_touched_order(order)
        elif (
            order.order_type == OrderType.TRAILING_STOP_MARKET
            or order.order_type == OrderType.TRAILING_STOP_LIMIT
        ):
            self._process_trailing_stop_order(order)
        else:
            raise RuntimeError(  # pragma: no cover (设计时错误)
                f"{order_type_to_str(order.order_type)} "  # pragma: no cover
                f"订单在当前版本的运行中不支持回测",  # pragma: no cover
            )

    cpdef void process_modify(self, ModifyOrder command, AccountId account_id):
        cdef Order order = self._core.get_order(command.client_order_id)
        if order is None:
            self._generate_order_modify_rejected(
                trader_id=command.trader_id,
                strategy_id=command.strategy_id,
                account_id=account_id,
                instrument_id=command.instrument_id,
                client_order_id=command.client_order_id,
                venue_order_id=command.venue_order_id,
                reason=f"找不到 {command.client_order_id!r}",
            )
        else:
            self.update_order(
                order,
                command.quantity,
                command.price,
                command.trigger_price,
            )

    cpdef void process_cancel(self, CancelOrder command, AccountId account_id):
        cdef Order order = self._core.get_order(command.client_order_id)
        if order is None:
            self._generate_order_cancel_rejected(
                trader_id=command.trader_id,
                strategy_id=command.strategy_id,
                account_id=account_id,
                instrument_id=command.instrument_id,
                client_order_id=command.client_order_id,
                venue_order_id=command.venue_order_id,
                reason=f"找不到 {command.client_order_id!r}",
            )
        else:
            if order.is_inflight_c() or order.is_open_c():
                self.cancel_order(order)

    cpdef void process_batch_cancel(self, BatchCancelOrders command, AccountId account_id):
        cdef CancelOrder cancel
        for cancel in command.cancels:
            self.process_cancel(cancel, account_id)

    cpdef void process_cancel_all(self, CancelAllOrders command, AccountId account_id):
        cdef Order order
        for order in self.cache.orders_open(venue=None, instrument_id=command.instrument_id):
            if command.order_side != OrderSide.NO_ORDER_SIDE and command.order_side != order.side:
                continue

            if order.is_inflight_c() or order.is_open_c():
                self.cancel_order(order)

    cdef void _process_market_order(self, MarketOrder order):
        # 检查 AT_THE_OPEN/AT_THE_CLOSE 生效时间
        if order.time_in_force == TimeInForce.AT_THE_OPEN or order.time_in_force == TimeInForce.AT_THE_CLOSE:
            self._generate_order_rejected(
                order,
                f"生效时间 {time_in_force_to_str(order.time_in_force)} "
                "目前不支持",
            )
            return
 
        # 检查市场是否存在
        if order.side == OrderSide.BUY and not self._core.is_ask_initialized:
            self._generate_order_rejected(order, f"{order.instrument_id} 没有行情")
            return  # 无法接受订单
        elif order.side == OrderSide.SELL and not self._core.is_bid_initialized:
            self._generate_order_rejected(order, f"{order.instrument_id} 没有行情")
            return  # 无法接受订单
 
        if self._use_market_order_acks:
            self._generate_order_accepted(order, venue_order_id=self._get_venue_order_id(order))
 
        # 立即成交市价单
        self.fill_market_order(order)

    cdef void _process_market_to_limit_order(self, MarketToLimitOrder order):
        # 检查市场是否存在
        if order.side == OrderSide.BUY and not self._core.is_ask_initialized:
            self._generate_order_rejected(order, f"{order.instrument_id} 没有行情")
            return  # 无法接受订单
        elif order.side == OrderSide.SELL and not self._core.is_bid_initialized:
            self._generate_order_rejected(order, f"{order.instrument_id} 没有行情")
            return  # 无法接受订单
 
        if self._use_market_order_acks:
            self._generate_order_accepted(order, venue_order_id=self._get_venue_order_id(order))
 
        # 立即成交市价单
        self.fill_market_order(order)
 
        if order.is_open_c():
            self.accept_order(order)

    cdef void _process_limit_order(self, LimitOrder order):
        # 检查 AT_THE_OPEN/AT_THE_CLOSE 生效时间
        if order.time_in_force == TimeInForce.AT_THE_OPEN or order.time_in_force == TimeInForce.AT_THE_CLOSE:
            self._generate_order_rejected(
                order,
                f"生效时间 {time_in_force_to_str(order.time_in_force)} "
                "目前不支持",
            )
            return
 
        if order.is_post_only and self._core.is_limit_matched(order.side, order.price):
            self._generate_order_rejected(
                order,
                f"被动委托 (POST_ONLY) {order.type_string_c()} {order.side_string_c()} 订单"
                f"限制价格 {order.price} 会导致其成为 TAKER： "
                f"买入价={self._core.bid}, "
                f"卖出价={self._core.ask}",
                True,  # 由于被动委托限制
            )
            return  # 无效价格
 
        # 订单有效并接受
        self.accept_order(order)
 
        # 检查是否立即成交
        if self._core.is_limit_matched(order.side, order.price):
            # 作为流动性提取者 (TAKER) 成交
            if order.liquidity_side == LiquiditySide.NO_LIQUIDITY_SIDE:
                order.liquidity_side = LiquiditySide.TAKER
 
            self.fill_limit_order(order)
        elif order.time_in_force == TimeInForce.FOK or order.time_in_force == TimeInForce.IOC:
            self.cancel_order(order)

    cdef void _process_stop_market_order(self, StopMarketOrder order):
        if self._core.is_stop_triggered(order.side, order.trigger_price):
            if self._reject_stop_orders:
                self._generate_order_rejected(
                    order,
                    f"{order.type_string_c()} {order.side_string_c()} 订单"
                    f"止损价 {order.trigger_price} 已处于市场中： "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                )
                return  # 无效价格
 
            self.fill_market_order(order)
            return
 
        # 订单有效并接受
        self.accept_order(order)

    cdef void _process_stop_limit_order(self, StopLimitOrder order):
        if self._core.is_stop_triggered(order.side, order.trigger_price):
            if self._reject_stop_orders:
                self._generate_order_rejected(
                    order,
                    f"{order.type_string_c()} {order.side_string_c()} 订单"
                    f"触发止损价 {order.trigger_price} 已处于市场中： "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                )
                return  # 无效价格
 
            self.accept_order(order)
            self._generate_order_triggered(order)
 
            # 检查是否立即符合成交条件
            if self._core.is_limit_matched(order.side, order.price):
                order.liquidity_side = LiquiditySide.TAKER
                self.fill_limit_order(order)
 
            return
 
        # 订单有效并接受
        self.accept_order(order)

    cdef void _process_market_if_touched_order(self, MarketIfTouchedOrder order):
        if self._core.is_touch_triggered(order.side, order.trigger_price):
            if self._reject_stop_orders:
                self._generate_order_rejected(
                    order,
                    f"{order.type_string_c()} {order.side_string_c()} 订单"
                    f"止损价 {order.trigger_price} 已处于市场中： "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                )
                return  # 无效价格
 
            self.fill_market_order(order)
            return
 
        # 订单有效并接受
        self.accept_order(order)

    cdef void _process_limit_if_touched_order(self, LimitIfTouchedOrder order):
        if self._core.is_touch_triggered(order.side, order.trigger_price):
            if self._reject_stop_orders:
                self._generate_order_rejected(
                    order,
                    f"{order.type_string_c()} {order.side_string_c()} 订单"
                    f"触发止损价 {order.trigger_price} 已处于市场中： "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                )
                return  # 无效价格
 
            self.accept_order(order)
            self._generate_order_triggered(order)
 
            # 检查是否立即符合成交条件
            if self._core.is_limit_matched(order.side, order.price):
                order.liquidity_side = LiquiditySide.TAKER
                self.fill_limit_order(order)
 
            return
 
        # 订单有效并接受
        self.accept_order(order)

    cdef void _process_trailing_stop_order(self, Order order):
        assert order.order_type == OrderType.TRAILING_STOP_MARKET \
            or order.order_type == OrderType.TRAILING_STOP_LIMIT

        cdef Price market_price = None
        if order.activation_price is None:
            # 如果未给出激活价格，
            # 将激活价格设置为最新价格，并激活订单
            market_price = self._core.ask if order.side == OrderSide.BUY else self._core.bid
 
            if market_price is None:
                # 如果没有市场价格，我们无法处理订单
                raise RuntimeError(  # pragma: no cover (设计时错误)
                    f"无法处理追踪止损，"
                    f"没有 {order.instrument_id} 的买入价或卖出价 "
                    f"（请添加报价或使用 Bar）",
                )
 
            order.set_activated_c(market_price)
        else:
            # 如果给出了激活价格，
            # 激活价格不应已经处于市场中，类似于触达触发订单。
            if self._core.is_touch_triggered(order.side, order.activation_price):
                # 注意：是否需要对激活价格应用 'reject_stop_orders'？
                if self._reject_stop_orders:
                    self._generate_order_rejected(
                        order,
                        f"{order.type_string_c()} {order.side_string_c()} 订单"
                        f"激活价 {order.activation_price} 已处于市场中： "
                        f"买入价={self._core.bid}, "
                        f"卖出价={self._core.ask}",
                    )
                    return  # 无效价格
 
                # 如果我们不能拒绝订单，就激活它
                order.set_activated_c(None)

        if order.is_activated:
            if order.has_trigger_price_c() and self._core.is_stop_triggered(order.side, order.trigger_price):
                self._generate_order_rejected(
                    order,
                    f"{order.type_string_c()} {order.side_string_c()} 订单"
                    f"触发止损价 {order.trigger_price} 已处于市场中： "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                )
                return  # 无效价格
 
        # 订单有效并接受
        self.accept_order(order)

    cdef void _update_limit_order(
        self,
        Order order,
        Quantity qty,
        Price price,
    ):
        if self._core.is_limit_matched(order.side, price):
            if order.is_post_only:
                self._generate_order_modify_rejected(
                    trader_id=order.trader_id,
                    strategy_id=order.strategy_id,
                    account_id=order.account_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                    reason=f"被动委托 (POST_ONLY) {order.type_string_c()} {order.side_string_c()} 订单"
                    f"新的限制价格 {price} 会导致其成为 TAKER： "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                )
                return  # 无法更新订单
 
            self._generate_order_updated(order, qty, price, None)
            order.liquidity_side = LiquiditySide.TAKER
            self.fill_limit_order(order)  # 立即作为 TAKER 成交
            return  # 已成交

        self._generate_order_updated(order, qty, price, None)

    cdef void _update_stop_market_order(
        self,
        Order order,
        Quantity qty,
        Price trigger_price,
    ):
        if self._core.is_stop_triggered(order.side, trigger_price):
            self._generate_order_modify_rejected(
                trader_id=order.trader_id,
                strategy_id=order.strategy_id,
                account_id=order.account_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                venue_order_id=order.venue_order_id,
                reason=f"{order.type_string_c()} {order.side_string_c()} 订单"
                f"新的止损价 {trigger_price} 已处于市场中： "
                f"买入价={self._core.bid}, "
                f"卖出价={self._core.ask}",
            )
            return  # 无法更新订单

        self._generate_order_updated(order, qty, None, trigger_price)

    cdef void _update_stop_limit_order(
        self,
        Order order,
        Quantity qty,
        Price price,
        Price trigger_price,
    ):
        if not order.is_triggered:
            # 更新止损价
            if self._core.is_stop_triggered(order.side, trigger_price):
                self._generate_order_modify_rejected(
                    trader_id=order.trader_id,
                    strategy_id=order.strategy_id,
                    account_id=order.account_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                    reason=f"{order.type_string_c()} {order.side_string_c()} 订单"
                    f"新的触发止损价 {trigger_price} 已处于市场中： "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                )
                return  # 无法更新订单
        else:
            # 更新限制价格
            if self._core.is_limit_matched(order.side, price):
                if order.is_post_only:
                    self._generate_order_modify_rejected(
                        trader_id=order.trader_id,
                        strategy_id=order.strategy_id,
                        account_id=order.account_id,
                        instrument_id=order.instrument_id,
                        client_order_id=order.client_order_id,
                        venue_order_id=order.venue_order_id,
                        reason=f"被动委托 (POST_ONLY) {order.type_string_c()} {order.side_string_c()} 订单  "
                        f"新的限制价格 {price} 会导致其成为 TAKER： "
                        f"买入价={self._core.bid}, "
                        f"卖出价={self._core.ask}",
                    )
                    return  # 无法更新订单
                else:
                    self._generate_order_updated(order, qty, price, trigger_price or order.trigger_price)
                    order.liquidity_side = LiquiditySide.TAKER
                    self.fill_limit_order(order)  # 立即作为 TAKER 成交
                    return  # 已成交

        self._generate_order_updated(order, qty, price, trigger_price or order.trigger_price)

    cdef void _update_market_if_touched_order(
        self,
        Order order,
        Quantity qty,
        Price trigger_price,
    ):
        if self._core.is_touch_triggered(order.side, trigger_price):
            self._generate_order_modify_rejected(
                trader_id=order.trader_id,
                strategy_id=order.strategy_id,
                account_id=order.account_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                venue_order_id=order.venue_order_id,
                reason=f"{order.type_string_c()} {order.side_string_c()} 订单"
                       f"新的止损价 {trigger_price} 已处于市场中： "
                       f"买入价={self._core.bid}, "
                       f"卖出价={self._core.ask}",
            )
            return  # 无法更新订单

        self._generate_order_updated(order, qty, None, trigger_price)

    cdef void _update_limit_if_touched_order(
        self,
        Order order,
        Quantity qty,
        Price price,
        Price trigger_price,
    ):
        if not order.is_triggered:
            # 更新止损价
            if self._core.is_touch_triggered(order.side, trigger_price):
                self._generate_order_modify_rejected(
                    trader_id=order.trader_id,
                    strategy_id=order.strategy_id,
                    account_id=order.account_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                    reason=f"{order.type_string_c()} {order.side_string_c()} 订单"
                           f"新的触发止损价 {trigger_price} 已处于市场中： "
                           f"买入价={self._core.bid}, "
                           f"卖出价={self._core.ask}",
                )
                return  # 无法更新订单
        else:
            # 更新限制价格
            if self._core.is_limit_matched(order.side, price):
                if order.is_post_only:
                    self._generate_order_modify_rejected(
                        trader_id=order.trader_id,
                        strategy_id=order.strategy_id,
                        account_id=order.account_id,
                        instrument_id=order.instrument_id,
                        client_order_id=order.client_order_id,
                        venue_order_id=order.venue_order_id,
                        reason=f"被动委托 (POST_ONLY) {order.type_string_c()} {order.side_string_c()} 订单  "
                               f"新的限制价格 {price} 会导致其成为 TAKER： "
                               f"买入价={self._core.bid}, "
                               f"卖出价={self._core.ask}",
                    )
                    return  # 无法更新订单
                else:
                    self._generate_order_updated(order, qty, price, trigger_price or order.trigger_price)
                    order.liquidity_side = LiquiditySide.TAKER
                    self.fill_limit_order(order)  # 立即作为 TAKER 成交
                    return  # 已成交

        self._generate_order_updated(order, qty, price, trigger_price or order.trigger_price)

    cdef void _update_trailing_stop_market_order(
        self,
        Order order,
        Quantity qty,
        Price trigger_price,
    ):
        if order.is_activated:
            # 已激活的追踪止损可能还没有 trigger_price；
            # 等待下一次市场更新来计算它
            if trigger_price is None:
                return
 
            self._update_stop_market_order(order, qty, trigger_price)
        elif qty or trigger_price:
            self._generate_order_updated(order, qty, None, trigger_price)

    cdef void _update_trailing_stop_limit_order(
        self,
        Order order,
        Quantity qty,
        Price price,
        Price trigger_price,
    ):
        if order.is_activated:
            # 已激活的追踪止损可能还没有 trigger_price；
            # 等待下一次市场更新来计算它
            if trigger_price is None:
                return
 
            self._update_stop_limit_order(order, qty, price, trigger_price)
        elif qty or trigger_price:
            self._generate_order_updated(order, qty, price, trigger_price)

    cdef void _trail_stop_order(self, Order order):
        cdef Price market_price = None

        if not order.is_activated:
            if order.activation_price is None:
                # 注意
                # 激活价格本应在 OrderMatchingEngine._process_trailing_stop_order() 中设置
                # 但是模拟器的实现绕过了这一步，而是在 match_order() 中直接调用此方法。
                market_price = self._core.ask if order.side == OrderSide.BUY else self._core.bid
 
                if market_price is None:
                    # 如果没有市场价格，我们无法处理订单
                    raise RuntimeError(  # pragma: no cover (设计时错误)
                        f"无法处理追踪止损，"
                        f"没有 {order.instrument_id} 的买入价或卖出价 "
                        f"（请添加报价或使用 Bar）",
                    )
 
                order.set_activated_c(market_price)
            elif self._core.is_touch_triggered(order.side, order.activation_price):
                order.set_activated_c(None)
            else:
                return  # 不执行任何操作

        cdef tuple output = TrailingStopCalculator.calculate(
            price_increment=self.instrument.price_increment,
            order=order,
            bid=self._core.bid,
            ask=self._core.ask,
            last=self._core.last,
        )

        cdef Price new_trigger_price = output[0]
        cdef Price new_price = output[1]
        if new_trigger_price is None and new_price is None:
            return  # 未更新

        self._generate_order_updated(
            order=order,
            quantity=order.quantity,
            price=new_price,
            trigger_price=new_trigger_price,
        )

# -- ORDER PROCESSING -----------------------------------------------------------------------------

    cpdef void iterate(self, uint64_t timestamp_ns, AggressorSide aggressor_side = AggressorSide.NO_AGGRESSOR):
        """
        通过处理买入和卖出订单侧并将时间推进到给定的 UNIX `timestamp_ns` 来迭代撮合引擎。

        Parameters
        ----------
        timestamp_ns : uint64_t
            撮合引擎时间要推进到的 UNIX 时间戳。
        aggressor_side : AggressorSide, 默认 'NO_AGGRESSOR'
            成交执行处理的主动侧。

        """
        self._clock.set_time(timestamp_ns)

        cdef Price_t bid
        cdef Price_t ask
        if orderbook_has_bid(&self._book._mem) and aggressor_side == AggressorSide.NO_AGGRESSOR:
            bid = orderbook_best_bid_price(&self._book._mem)
            self._core.set_bid_raw(bid.raw)

        if orderbook_has_ask(&self._book._mem) and aggressor_side == AggressorSide.NO_AGGRESSOR:
            ask = orderbook_best_ask_price(&self._book._mem)
            self._core.set_ask_raw(ask.raw)

        self._core.iterate(timestamp_ns)

        cdef list[Order] orders = self._core.get_orders()
        cdef Order order
        for order in orders:
            if order.is_closed_c():
                self._cached_filled_qty.pop(order.client_order_id, None)
                continue

            # 检查过期
            if self._support_gtd_orders:
                if order.expire_time_ns > 0 and timestamp_ns >= order.expire_time_ns:
                    self._core.delete_order(order)
                    self._cached_filled_qty.pop(order.client_order_id, None)
                    self.expire_order(order)
                    continue

            # 管理追踪止损
            if order.order_type == OrderType.TRAILING_STOP_MARKET or order.order_type == OrderType.TRAILING_STOP_LIMIT:
                self._trail_stop_order(order)

            # 将市场价格移动回目标
            if self._has_targets:
                self._core.set_bid_raw(self._target_bid)
                self._core.set_ask_raw(self._target_ask)
                self._core.set_last_raw(self._target_last)
                self._has_targets = False

        # 迭代后重置所有目标
        self._target_bid = 0
        self._target_ask = 0
        self._target_last = 0
        self._has_targets = False

        # 工具到期
        if (self._instrument_has_expiration and timestamp_ns > self.instrument.expiration_ns) or self._instrument_close is not None:
            self._log.info(f"{self.instrument.id} 已达到到期时间")

            # 取消所有挂单
            for order in self.get_open_orders():
                self.cancel_order(order)

            # 关闭所有未平仓头寸
            for position in self.cache.positions_open(None, self.instrument.id):
                order = MarketOrder(
                    trader_id=position.trader_id,
                    strategy_id=position.strategy_id,
                    instrument_id=position.instrument_id,
                    client_order_id=ClientOrderId(f"EXPIRATION-LEG-{uuid.uuid4()}"),
                    order_side=Order.closing_side_c(position.side),
                    quantity=position.quantity,
                    init_id=UUID4(),
                    ts_init=self._clock.timestamp_ns(),
                    reduce_only=True,
                    tags=[f"EXPIRATION_{self.venue}_CLOSE"],
                )
                self.cache.add_order(order, position_id=position.id)
                self.fill_market_order(order)

    cpdef void fill_market_order(self, Order order):
        """
        成交给定的 *可立即成交 (marketable)* 订单。

        Parameters
        ----------
        order : Order
            要成交的订单。

        """
        cdef Quantity cached_filled_qty = self._cached_filled_qty.get(order.client_order_id)
        if cached_filled_qty is not None and cached_filled_qty._mem.raw >= order.quantity._mem.raw:
            self._log.debug(
                f"忽略成交，因为在应用事件期间已完成填充： "
                f"{cached_filled_qty=}, {order.quantity=}, {order.filled_qty=}, {order.leaves_qty=}",
            )
            return
 
        cdef PositionId venue_position_id = self._get_position_id(order)
        cdef Position position = None
        if venue_position_id is not None:
            position = self.cache.position(venue_position_id)
 
        if self._use_reduce_only and order.is_reduce_only and position is None:
            self._log.warning(
                f"正在取消只减仓 (REDUCE_ONLY) {order.type_string_c()}，"
                f"因为其会导致仓位增加",
            )
            self.cancel_order(order)
            return  # 订单已取消

        order.liquidity_side = LiquiditySide.TAKER
        cdef list[tuple[Price, Quantity]] fills = self.determine_market_fills_with_simulation(order)

        self.apply_fills(
            order=order,
            fills=fills,
            liquidity_side=order.liquidity_side,
            venue_position_id=venue_position_id,
            position=position,
        )

    cdef list[tuple[Price, Quantity]] determine_market_fills_with_simulation(self, Order order):
        """
        如果可用，使用 FillModel 模拟来确定市价单成交。

        此方法首先检查 FillModel 是否提供模拟 OrderBook 用于成交模拟。
        如果是，则使用该模拟进行成交判定。否则，回退到标准的市价成交逻辑。
        """
        if self._fill_model is None:
            return self.determine_market_price_and_volume(order)
 
        # 获取当前最佳买入/卖出价格用于模拟
        cdef Price best_bid = self._core.bid
        cdef Price best_ask = self._core.ask
 
        if best_bid is None or best_ask is None:
            return []  # 市场不可用
 
        # 尝试从 FillModel 获取模拟 OrderBook
        cdef OrderBook simulated_book = self._fill_model.get_orderbook_for_fill_simulation(
            self.instrument, order, best_bid, best_ask
        )
 
        if simulated_book is not None:
            # 使用模拟 OrderBook 进行成交判定
            fills = simulated_book.simulate_fills(
                order,
                price_prec=self._price_prec,
                size_prec=self._size_prec,
                is_aggressive=True,
            )
            # 如果模拟未产生任何成交（例如，自定义模型移除了最佳档位），
            # 则回退到标准市价逻辑以保持预期行为。
            if not fills:
                return self.determine_market_price_and_volume(order)
            return fills
        else:
            # 回退到标准逻辑
            return self.determine_market_price_and_volume(order)

    cdef list[tuple[Price, Quantity]] _apply_liquidity_consumption(
        self,
        list[tuple[Price, Quantity]] fills,
        OrderSide order_side,
        QuantityRaw max_qty_raw=0,
        list[Price] book_prices=None,
    ):
        if not self._liquidity_consumption:
            return fills

        cdef dict[PriceRaw, tuple[QuantityRaw, QuantityRaw]] consumption

        if order_side == OrderSide.BUY:
            consumption = self._ask_consumption
        elif order_side == OrderSide.SELL:
            consumption = self._bid_consumption
        else:
            return fills

        cdef list[tuple[Price, Quantity]] adjusted_fills = []
        cdef QuantityRaw remaining_qty = max_qty_raw  # 0 means no limit

        cdef:
            Price price
            Price book_price
            Quantity qty
            Quantity level_size
            tuple level_state
            PriceRaw price_raw
            PriceRaw book_price_raw
            PriceRaw p_raw
            QuantityRaw qty_raw
            QuantityRaw q_raw
            QuantityRaw level_size_raw
            QuantityRaw original_size
            QuantityRaw consumed
            QuantityRaw available
            QuantityRaw adjusted_qty_raw
            QuantityRaw fill_total
            Quantity adjusted_qty
            int fill_idx

        # 每个价格的聚合成交数量（根据对缺失档位的需求计算）
        cdef dict[PriceRaw, QuantityRaw] fill_totals = None
 
        for fill_idx, fill in enumerate(fills):
            if max_qty_raw > 0 and remaining_qty == 0:
                break
 
            price = fill[0]
            qty = fill[1]
            price_raw = price._mem.raw
 
            # 使用 book_price 进行消耗跟踪（MAKER 调整前的原始价格），
            # 但使用 price（可能经过调整）用于输出成交。
            if book_prices is not None and fill_idx < len(book_prices):
                book_price = book_prices[fill_idx]
                book_price_raw = book_price._mem.raw
            else:
                book_price = price
                book_price_raw = price_raw
 
            level_size = self._book.get_quantity_at_level(book_price, order_side, self._size_prec)
            level_size_raw = level_size._mem.raw
 
            level_state = consumption.get(book_price_raw)
 
            # 处理档位在订单簿中不再存在的竞态条件（返回 0）
            if level_size_raw == 0:
                # 档位在成交确定和消耗之间被删除/修改。
                # 对此价格使用聚合成交总额（处理具有相同价格多个成交的 L3 订单簿）。
                # 如果存在先前的状态，则使用先前 original_size 和成交总额的最大值，
                # 以确保所有成交都能得到处理。
 
                if fill_totals is None:
                    fill_totals = {}
 
                    for idx, f in enumerate(fills):
                        if book_prices is not None and idx < len(book_prices):
                            p_raw = (<Price>book_prices[idx])._mem.raw
                        else:
                            p_raw = (<Price>f[0])._mem.raw
                        q_raw = (<Quantity>f[1])._mem.raw
                        if p_raw in fill_totals:
                            fill_totals[p_raw] += q_raw
                        else:
                            fill_totals[p_raw] = q_raw
 
                fill_total = fill_totals.get(book_price_raw, qty._mem.raw)
 
                if level_state is not None:
                    level_size_raw = max(level_state[0], fill_total)
                    self._log.debug(
                        f"流动性消耗：在订单簿中未找到档位 {book_price}，"
                        f"使用先前大小 {level_state[0]} 和成交总额 {fill_total} 的最大值",
                    )
                else:
                    level_size_raw = fill_total
                    self._log.debug(
                        f"流动性消耗：在订单簿中未找到档位 {book_price}，"
                        f"使用聚合成交总额 {fill_total} 作为回退值",
                    )
 
            if level_state is None:
                original_size = level_size_raw
                consumed = 0
            else:
                original_size = level_state[0]
                consumed = level_state[1]
 
            # 当订单簿大小更改时重置消耗（新鲜数据）
            if original_size != level_size_raw:
                original_size = level_size_raw
                consumed = 0
 
            available = original_size - consumed if original_size > consumed else 0
            if available == 0:
                self._log.debug(
                    f"流动性已耗尽：跳过档位 {book_price} "
                    f"(original_size={original_size}, consumed={consumed}, level_size_raw={level_size_raw})",
                )
                continue

            adjusted_qty_raw = min(qty._mem.raw, available)
 
            if max_qty_raw > 0:
                adjusted_qty_raw = min(adjusted_qty_raw, remaining_qty)
                remaining_qty -= adjusted_qty_raw
 
            if adjusted_qty_raw == 0:
                continue
 
            consumed += adjusted_qty_raw
            consumption[book_price_raw] = (original_size, consumed)
 
            adjusted_qty = Quantity.from_raw_c(adjusted_qty_raw, qty._mem.precision)
            adjusted_fills.append((price, adjusted_qty))
 
        return adjusted_fills
 
    cpdef list[tuple[Price, Quantity]] determine_market_price_and_volume(self, Order order):
        """
        返回给定的 *可立即成交 (marketable)* 订单在向反向订单侧主动填充时的预计成交情况。
 
        如果没有成交，列表可能为空。
 
        Parameters
        ----------
        order : Order
            要确定成交情况的订单。
 
        Returns
        -------
        list[tuple[Price, Quantity]]
 
        """
        cdef list[tuple[Price, Quantity]] fills = self._book.simulate_fills(
            order,
            price_prec=self._price_prec,
            size_prec=self._size_prec,
            is_aggressive=True,
        )
 
        # 对于 Bar H/L/C 处理期间的止损市价单和触及市价单，按触发价成交
        # （市场移动穿过了触发价）。对于缺口/立即触发，按市场价成交。
        cdef Price triggered_price
        if (
            not self._fill_at_market
            and self._book.book_type == BookType.L1_MBP
            and fills
            and (
                order.order_type == OrderType.STOP_MARKET
                or order.order_type == OrderType.TRAILING_STOP_MARKET
                or order.order_type == OrderType.MARKET_IF_TOUCHED
            )
        ):
            triggered_price = order.get_triggered_price_c()
            if triggered_price is not None:
                fills[0] = (triggered_price, fills[0][1])
                # 对于触发价成交，跳过流动性消耗（可能在具有无订单簿流动性的缺口价格处成交）
                return fills
 
        return self._apply_liquidity_consumption(fills, order.side, order.leaves_qty._mem.raw)

    cpdef void fill_limit_order(self, Order order):
        """
        成交给定的限价单。

        Parameters
        ----------
        order : Order
            要成交的订单。

        Raises
        ------
        ValueError
            如果 `order` 没有限价 `price`。

        """
        Condition.is_true(order.has_price_c(), "订单没有限价 `price`")
 
        cdef Price price = order.price
        cdef Quantity cached_filled_qty = self._cached_filled_qty.get(order.client_order_id)
        cdef bint at_limit = False
 
        if cached_filled_qty is not None and cached_filled_qty._mem.raw >= order.quantity._mem.raw:
            self._log.debug(
                f"忽略成交，因为在应用事件期间已完成填充： "
                f"{cached_filled_qty=}, {order.quantity=}, {order.filled_qty=}, {order.leaves_qty=}",
            )
            return
 
        # 检查成交模型中处于限价价格的挂单 (MAKER)
        if order.liquidity_side == LiquiditySide.MAKER and self._fill_model:
            # 对于成交执行：检查成交价格是否等于订单价格
            # 对于报价更新：检查买入价/卖出价是否等于订单价格
            if self._last_trade_size is not None and self._core.is_last_initialized:
                at_limit = self._core.last_raw == price._mem.raw
            elif order.side == OrderSide.BUY:
                at_limit = self._core.bid_raw == price._mem.raw
            elif order.side == OrderSide.SELL:
                at_limit = self._core.ask_raw == price._mem.raw
 
            if at_limit and not self._fill_model.is_limit_filled():
                return  # 未成交（模拟排队位置）
 
        cdef PositionId venue_position_id = self._get_position_id(order)
        cdef Position position = None
        if venue_position_id is not None:
            position = self.cache.position(venue_position_id)
 
        if self._use_reduce_only and order.is_reduce_only and position is None:
            self._log.warning(
                f"正在取消只减仓 (REDUCE_ONLY) {order.type_string_c()}，"
                f"因为其会导致仓位增加",
            )
            self.cancel_order(order)
            return  # 订单已取消

        cdef list[tuple[Price, Quantity]] fills = self.determine_limit_fills_with_simulation(order)
 
        # 当流动性消耗调整后导致无成交时，跳过 apply_fills。
        # 当不相关的增量到达且在该订单的价格水平上没有新流动性可用时，部分成交的订单会发生这种情况。
        if not fills and self._liquidity_consumption:
            self._log.debug(
                f"跳过 {order.client_order_id} 的成交：消耗后无可用流动性",
            )
 
            if order.time_in_force == TimeInForce.FOK or order.time_in_force == TimeInForce.IOC:
                self.cancel_order(order)
 
            return
 
        self.apply_fills(
            order=order,
            fills=fills,
            liquidity_side=order.liquidity_side,
            venue_position_id=venue_position_id,
            position=position,
        )
 
    cdef list[tuple[Price, Quantity]] determine_limit_fills_with_simulation(self, Order order):
        """
        如果可用，使用 FillModel 模拟来确定限价单成交。
 
        此方法首先检查 FillModel 是否提供模拟 OrderBook 用于成交模拟。
        如果是，则使用该模拟进行成交判定。否则，回退到标准的限价成交逻辑。
        """
        if self._fill_model is None:
            return self.determine_limit_price_and_volume(order)
 
        # 获取当前最佳买入/卖出价格用于模拟
        cdef Price best_bid = self._core.bid
        cdef Price best_ask = self._core.ask
 
        if best_bid is None or best_ask is None:
            return []  # 市场不可用
 
        # 尝试从 FillModel 获取模拟 OrderBook
        cdef OrderBook simulated_book = self._fill_model.get_orderbook_for_fill_simulation(
            self.instrument, order, best_bid, best_ask
        )
 
        if simulated_book is not None:
            # 使用模拟 OrderBook 进行成交判定
            return simulated_book.simulate_fills(
                order,
                price_prec=self._price_prec,
                size_prec=self._size_prec,
                is_aggressive=False,
            )
        else:
            # 回退到标准逻辑
            return self.determine_limit_price_and_volume(order)

    cdef Quantity determine_trade_fill_qty(self, Order order):
        """
        确定成交执行模式下的成交数量。

        当成交执行模式通过瞬时价格覆盖触发匹配时，此方法将成交数量计算为以下各项的最小值：
        - 订单的剩余数量 (leaves_qty)
        - 剩余成交数量（启用消耗时）或成交 tick 大小

        如果没有可用于成交的数量，则返回 None。
        """
        cdef QuantityRaw leaves_raw = order.quantity._mem.raw - order.filled_qty._mem.raw if order.quantity._mem.raw > order.filled_qty._mem.raw else 0
 
        if leaves_raw == 0:
            return None
 
        cdef QuantityRaw fill_raw = leaves_raw
        cdef QuantityRaw available_raw
        cdef QuantityRaw trade_size_raw
 
        if self._last_trade_size is not None:
            trade_size_raw = self._last_trade_size._mem.raw
 
            # 计算成交中的可用数量（减去任何消耗）
            if self._liquidity_consumption:
                available_raw = trade_size_raw - self._trade_consumption
            else:
                available_raw = trade_size_raw
 
            if available_raw == 0:
                return None
 
            fill_raw = min(leaves_raw, available_raw)
 
            if self._liquidity_consumption:
                self._trade_consumption += fill_raw
 
        return Quantity.from_raw_c(fill_raw, self._size_prec)
 
    cpdef list[tuple[Price, Quantity]] determine_limit_price_and_volume(self, Order order):
        """
        返回给定的 *限价* 订单在其限价价格处被动填充时的预计成交情况。
 
        如果没有成交，列表可能为空。
 
        Parameters
        ----------
        order : Order
            要确定成交情况的订单。
 
        Returns
        -------
        list[tuple[Price, Quantity]]
 
        Raises
        ------
        ValueError
            如果 `order` 没有限价 `price`。
 
        """
        Condition.is_true(order.has_price_c(), "订单没有限价 `price`")
 
        cdef list[tuple[Price, Quantity]] fills
 
        # 当启用流动性消耗时，我们需要考虑所有交叉的价格水平，而不只是满足 leaves_qty 的部分。
        # 这是因为某些档位可能已被消耗，我们需要从后续档位进行成交。
        if self._liquidity_consumption:
            fills = self._book.get_all_crossed_levels(order.side, order.price, self._size_prec)
        else:
            fills = self._book.simulate_fills(
                order,
                price_prec=self._price_prec,
                size_prec=self._size_prec,
                is_aggressive=False,
            )
 
        cdef Price triggered_price = order.get_triggered_price_c()
        cdef Price price = order.price
 
        # 成交执行：当订单簿不反映成交价格时，使用成交驱动的填充
        cdef:
            Price trade_price
            bint fills_at_trade_price
            bint skip_trade_fill
            Quantity fill_qty
            Price fill_px
 
        if self._last_trade_size is not None and self._core.is_last_initialized:
            trade_price = Price.from_raw_c(self._core.last_raw, self._price_prec)
 
            fills_at_trade_price = False
            for fill in fills:
                fill_px = fill[0]
                if fill_px == trade_price:
                    fills_at_trade_price = True
                    break
 
            if (
                not fills_at_trade_price
                and self._core.is_limit_matched(order.side, order.price)
            ):
                # 限价挂单 (MAKER) 的成交模型检查已在 fill_limit_order 中处理，
                # 这里不再重复检查，以避免两次调用 is_limit_filled()（p² 概率）。
                fill_qty = self.determine_trade_fill_qty(order)
                if fill_qty is not None:
                    self._log.debug(
                        f"成交执行填充：{fill_qty} @ {order.price} "
                        f"(trade_price={trade_price}, trade_size={self._last_trade_size})",
                    )
 
                    # 按照限价价格（保守）而不是成交价格进行成交。
                    # 成交轨迹填充已经通过 _trade_consumption 考虑了消耗，
                    # 提前返回以绕过 _apply_liquidity_consumption，
                    # 否则当成交价格不在订单簿中时，它会错误地丢弃这些成交。
                    return [(order.price, fill_qty)]

        # 在进行任何成交价格修改之前，保存原始订单簿价格，以便进行消耗跟踪，
        # 因为下面的 TAKER 和 MAKER 循环可能会调整成交价格。目前，消耗应针对
        # 流动性来源的原始订单簿价格水平进行跟踪。
        # 我们必须创建新的 Price 对象，因为 MAKER 循环会原地修改价格。
        cdef list[Price] book_prices = None
        cdef Price orig_price
 
        if self._liquidity_consumption and fills:
            book_prices = []
            for fill in fills:
                orig_price = fill[0]
                book_prices.append(Price.from_raw_c(orig_price._mem.raw, orig_price._mem.precision))
 
        if (
            fills
            and triggered_price is not None
            and order.liquidity_side == LiquiditySide.TAKER
        ):
            ########################################################################
            # 作为触发产生的流动性提取者 (TAKER) 进行成交
            ########################################################################
            if order.side == OrderSide.BUY and price._mem.raw > triggered_price._mem.raw:
                fills[0] = (triggered_price, fills[0][1])
                self._has_targets = True
                self._target_bid = self._core.bid_raw
                self._target_ask = self._core.ask_raw
                self._target_last = self._core.last_raw
                self._core.set_ask_raw(price._mem.raw)
                self._core.set_last_raw(price._mem.raw)
            elif order.side == OrderSide.SELL and price._mem.raw < triggered_price._mem.raw:
                fills[0] = (triggered_price, fills[0][1])
                self._has_targets = True
                self._target_bid = self._core.bid_raw
                self._target_ask = self._core.ask_raw
                self._target_last = self._core.last_raw
                self._core.set_bid_raw(price._mem.raw)
                self._core.set_last_raw(price._mem.raw)
 
        cdef Price last_px
 
        if fills and order.liquidity_side == LiquiditySide.MAKER:
            ########################################################################
            # 作为流动性提供者 (MAKER) 进行成交
            ########################################################################
            price = order.price
 
            if order.side == OrderSide.BUY:
                if triggered_price and price > triggered_price:
                    price = triggered_price
 
                for fill in fills:
                    last_px = fill[0]
 
                    if last_px._mem.raw < price._mem.raw:
                        # 可立即成交的买入单本应以限价成交
                        self._has_targets = True
                        self._target_bid = self._core.bid_raw
                        self._target_ask = self._core.ask_raw
                        self._target_last = self._core.last_raw
                        self._core.set_ask_raw(price._mem.raw)
                        self._core.set_last_raw(price._mem.raw)
                        last_px._mem.raw = price._mem.raw
            elif order.side == OrderSide.SELL:
                if triggered_price and price < triggered_price:
                    price = triggered_price
 
                for fill in fills:
                    last_px = fill[0]
 
                    if last_px._mem.raw > price._mem.raw:
                        # 可立即成交的卖出单本应以限价成交
                        self._has_targets = True
                        self._target_bid = self._core.bid_raw
                        self._target_ask = self._core.ask_raw
                        self._target_last = self._core.last_raw
                        self._core.set_bid_raw(price._mem.raw)
                        self._core.set_last_raw(price._mem.raw)
                        last_px._mem.raw = price._mem.raw
            else:
                raise RuntimeError(f"无效的 `OrderSide`，为 {order.side}")  # pragma: no cover (设计时错误)
 
        return self._apply_liquidity_consumption(fills, order.side, order.leaves_qty._mem.raw, book_prices)

    cpdef void apply_fills(
        self,
        Order order,
        list[tuple[Price, Quantity]] fills,
        LiquiditySide liquidity_side,
        PositionId venue_position_id: PositionId | None = None,
        Position position: Position | None = None,
    ):
        """
        将给定的成交列表应用到给定的订单。可选地提供现有的头寸详情。
 
        - 如果 `fills` 列表为空，将记录错误。
        - 如果没有可用于履约的反向订单，市价单将被拒绝。
 
        Parameters
        ----------
        order : Order
            要成交的订单。
        fills : list[tuple[Price, Quantity]]
            要应用到订单的成交。
        liquidity_side : LiquiditySide
            成交的流动性侧。
        venue_position_id : PositionId, 可选
            与订单相关的当前场所头寸 ID（如果已分配）。
        position : Position, 可选
            与订单相关的当前头寸（如有）。
 
        Raises
        ------
        ValueError
            如果 `liquidity_side` 为 ``NO_LIQUIDITY_SIDE``。
 
        Warnings
        --------
        `liquidity_side` 将覆盖订单上之前设置的任何值。
 
        """
        Condition.not_none(order, "order")
        Condition.not_none(fills, "fills")
        Condition.not_equal(liquidity_side, LiquiditySide.NO_LIQUIDITY_SIDE, "liquidity_side", "NO_LIQUIDITY_SIDE")
 
        order.liquidity_side = liquidity_side

        cdef:
            Price fill_px
            Quantity fill_qty
            QuantityRaw total_size_raw = 0
        if order.time_in_force == TimeInForce.FOK:
            # 检查 FOK 要求
            for fill in fills:
                fill_px, fill_qty = fill
                total_size_raw += fill_qty._mem.raw
 
            if order.leaves_qty._mem.raw > total_size_raw:
                self.cancel_order(order)
                return  # 无法全额成交 - 因此将其失效/取消
 
        cdef:
            bint initial_market_to_limit_fill = False
            Price last_fill_px = None
 
        if not fills:
            # 对于具有消耗跟踪的 L1，成交列表为空意味着流动性已被消耗
            # 允许订单滑向下一档（保持 L1 订单簿耗尽行为）
            if self._liquidity_consumption and self.book_type == BookType.L1_MBP:
                if order.side == OrderSide.BUY:
                    if not self._core.is_ask_initialized:
                        if order.status_c() == OrderStatus.SUBMITTED:
                            self._generate_order_rejected(order, f"{order.instrument_id} 没有行情")
                        return
                    last_fill_px = Price.from_raw_c(self._core.ask_raw, self._price_prec)
                elif order.side == OrderSide.SELL:
                    if not self._core.is_bid_initialized:
                        if order.status_c() == OrderStatus.SUBMITTED:
                            self._generate_order_rejected(order, f"{order.instrument_id} 没有行情")
                        return
                    last_fill_px = Price.from_raw_c(self._core.bid_raw, self._price_prec)
                else:
                    raise ValueError(f"无效的 `OrderSide`，为 {order.side}")
                # 向下进入下面的滑差逻辑（由订单类型控制）
            else:
                if order.status_c() == OrderStatus.SUBMITTED:
                    self._generate_order_rejected(order, f"{order.instrument_id} 没有行情且无成交")
                else:
                    self._log.error(
                        "无法成交订单：预期有成交时订单簿未提供（请检查数据）",
                    )
                return  # 无成交
 
        if self.oms_type == OmsType.NETTING:
            venue_position_id = None  # 场所不生成头寸 ID
 
        if is_logging_initialized():
            self._log.debug(
                "市场： "
                f"买入价={self._book.best_bid_size()} @ {self._book.best_bid_price()}, "
                f"卖出价={self._book.best_ask_size()} @ {self._book.best_ask_price()}, "
                f"最新价={self._core.last}",
            )
            self._log.debug(
                f"正在向 {order} 应用成交, "
                f"venue_position_id={venue_position_id}, "
                f"position={position}, "
                f"fills={fills}",
            )
 
        for fill in fills:
            fill_px = fill[0]
            fill_qty = fill[1]
 
            # 验证价格精度
            if fill_px._mem.precision != self._price_prec:
                raise RuntimeError(
                    f"成交价格精度无效 {fill_px.precision} "
                    f"而工具价格精度为 {self._price_prec}。 "
                    f"请检查数据价格精度是否与 {self.instrument.id} 工具匹配"
                )
 
            # 验证数量精度
            if fill_qty._mem.precision != self._size_prec:
                raise RuntimeError(
                    f"成交数量精度无效 {fill_qty.precision} "
                    f"而工具数量精度为 {self._size_prec}。 "
                    f"请检查数据数量精度是否与 {self.instrument.id} 工具匹配"
                )
 
            if order.filled_qty._mem.raw == 0:
                if order.order_type == OrderType.MARKET_TO_LIMIT:
                    self._generate_order_updated(
                        order,
                        qty=order.quantity,
                        price=fill_px,
                        trigger_price=None,
                    )
                    initial_market_to_limit_fill = True
 
            if self.book_type == BookType.L1_MBP and self._fill_model.is_slipped():
                if order.side == OrderSide.BUY:
                    fill_px = fill_px.add(self.instrument.price_increment)
                elif order.side == OrderSide.SELL:
                    fill_px = fill_px.sub(self.instrument.price_increment)
                else:
                    raise ValueError(  # pragma: no cover (设计时错误)
                        f"无效的 `OrderSide`，为 {order.side}",  # pragma: no cover (设计时错误)
                    )
 
            # 检查只减仓订单
            if self._use_reduce_only and order.is_reduce_only and fill_qty._mem.raw > position.quantity._mem.raw:
                if position.quantity._mem.raw == 0:
                    return  # 完成
 
                # 调整成交以遵从只减仓执行（仅填充剩余头寸大小）
                fill_qty = Quantity.from_raw_c(position.quantity._mem.raw, self._size_prec)
 
                self._generate_order_updated(
                    order=order,
                    qty=fill_qty,
                    price=None,
                    trigger_price=None,
                )
 
            if fill_qty._mem.raw == 0:
                if len(fills) == 1 and order.status_c() == OrderStatus.SUBMITTED:
                    self._generate_order_rejected(order, f"{order.instrument_id} 没有行情")
 
                return  # 完成
 
            self.fill_order(
                order=order,
                last_px=fill_px,
                last_qty=fill_qty,
                liquidity_side=order.liquidity_side,
                venue_position_id=venue_position_id,
                position=position,
            )
            if order.order_type == OrderType.MARKET_TO_LIMIT and initial_market_to_limit_fill:
                return  # 已成交初始档位
 
            last_fill_px = fill_px
 
        if order.time_in_force == TimeInForce.IOC and order.is_open_c():
            # IOC 订单已填充所有可用大小
            self.cancel_order(order)
            return
 
        # 在耗尽订单簿成交量时检查市价单 (MARKET)
        if (
            order.is_open_c()
            and self.book_type == BookType.L1_MBP
            and (
            order.order_type == OrderType.MARKET
            or order.order_type == OrderType.MARKET_IF_TOUCHED
            or order.order_type == OrderType.STOP_MARKET
            or order.order_type == OrderType.TRAILING_STOP_MARKET
        )
        ):
            # 模拟订单簿成交量耗尽（继续主动填充到下一档）
            # 这是一个非常基础的跳过一个 tick 的滑差实现，未来我们将实现更详细的成交建模。
            if order.side == OrderSide.BUY:
                fill_px = last_fill_px.add(self.instrument.price_increment)
            elif order.side == OrderSide.SELL:
                fill_px = last_fill_px.sub(self.instrument.price_increment)
            else:
                raise ValueError(  # pragma: no cover (设计时错误)
                    f"无效的 `OrderSide`，为 {order.side}",  # pragma: no cover (设计时错误)
                )
 
            self.fill_order(
                order=order,
                last_px=fill_px,
                last_qty=order.leaves_qty,
                liquidity_side=order.liquidity_side,
                venue_position_id=venue_position_id,
                position=position,
            )
 
        # 在耗尽订单簿成交量时检查限价单 (LIMIT)
        if (
            order.is_open_c()
            and self.book_type == BookType.L1_MBP
            and (
            order.order_type == OrderType.LIMIT
            or order.order_type == OrderType.LIMIT_IF_TOUCHED
            or order.order_type == OrderType.MARKET_TO_LIMIT
            or order.order_type == OrderType.STOP_LIMIT
            or order.order_type == OrderType.TRAILING_STOP_LIMIT
        )
        ):
            if not self._has_targets and ((order.side == OrderSide.BUY and order.price == self._core.ask) or (order.side == OrderSide.SELL and order.price == self._core.bid)):
                return  # 限价等于盘口，不再继续成交
 
            if order.liquidity_side == LiquiditySide.MAKER:
                # 市场移动穿过了限价，假设有足够的流动性来填充整个订单
                fill_px = order.price
            else:  # 可立即成交的限价单
                # 模拟订单簿成交量耗尽（继续主动填充到下一档）
                # 这是一个非常基础的跳过一个 tick 的滑差实现，未来我们将实现更详细的成交建模。
                if order.side == OrderSide.BUY:
                    fill_px = last_fill_px.add(self.instrument.price_increment)
                elif order.side == OrderSide.SELL:
                    fill_px = last_fill_px.sub(self.instrument.price_increment)
                else:
                    raise ValueError(  # pragma: no cover (设计时错误)
                        f"无效的 `OrderSide`，为 {order.side}",  # pragma: no cover (设计时错误)
                    )
 
            self.fill_order(
                order=order,
                last_px=fill_px,
                last_qty=order.leaves_qty,
                liquidity_side=order.liquidity_side,
                venue_position_id=venue_position_id,
                position=position,
            )

        cdef Instrument instrument = self.cache.instrument(order.instrument_id)
        if instrument is None:
            return

        # Generate leg fills for spread orders after normal combo fill processing
        if instrument.is_spread():
            self._generate_spread_leg_fills(order, fills, liquidity_side)

    cdef void _generate_spread_leg_fills(
        self,
        Order order,
        list[tuple[Price, Quantity]] fills,
        LiquiditySide liquidity_side,
    ):
        """
        在价差订单成交后，为头寸跟踪生成单独的成分股 (leg) 成交。
 
        此方法生成具有 "-LEG-" 标识符的合成成分股成交，这些成交将由 ExecutionEngine 处理以进行头寸跟踪，遵循 IB 模式。
        """
        if not fills:
            return
 
        # 获取价差工具
        cdef Instrument instrument = self.cache.instrument(order.instrument_id)
        if instrument is None:
            self._log.error(f"在缓存中找不到价差工具： {order.instrument_id}")
            return
 
        # 从工具中解析价差成分股
        leg_tuples = instrument.legs()
        spread_instrument_ids = [leg[0] for leg in leg_tuples]
 
        cdef Price spread_fill_px = fills[0][0]
        cdef Quantity spread_fill_qty = fills[0][1]
 
        # 计算成分股执行价格
        leg_prices = self._calculate_leg_execution_prices(
            leg_tuples=leg_tuples,
            spread_execution_price=spread_fill_px,
            spread_quantity=spread_fill_qty,
        )
 
        if not leg_prices:
            self._log.warning(f"无法为价差 {order.instrument_id} 计算成分股价格")
            return
 
        # 为每个成分股生成成交
        for leg_instrument_id, ratio in leg_tuples:
            if leg_instrument_id not in leg_prices:
                continue
 
            leg_price = leg_prices[leg_instrument_id]
 
            # 计算成分股数量：spread_quantity * abs(ratio)
            leg_quantity = Quantity(
                spread_fill_qty.as_double() * abs(ratio),
                precision=spread_fill_qty._mem.precision,
            )
 
            # 获取成分股工具用于精度验证
            leg_instrument = self.cache.instrument(leg_instrument_id)
 
            if leg_instrument is None:
                self._log.warning(f"在缓存中找不到成分股工具： {leg_instrument_id}")
                continue
 
            # 直接生成合成成分股成交
            adjusted_leg_price = leg_price
 
            # 使用 make_qty 进行适当的数量增量舍入
            adjusted_leg_quantity = leg_instrument.make_qty(
                leg_quantity.as_double(),
                round_down=True,  # 向下舍入以确保数量有效
            )
 
            # 计算成分股的佣金
            commission = self._fee_model.get_commission(
                order=order,  # 使用价差订单作为费用计算背景
                fill_qty=adjusted_leg_quantity,
                fill_px=adjusted_leg_price,
                instrument=leg_instrument,
            )
 
            # 生成成分股成交的唯一 ID（遵循 IB 适配器模式）
            # 获取成分股在价差中的位置以进行唯一识别
            leg_position = spread_instrument_ids.index(leg_instrument_id) if leg_instrument_id in spread_instrument_ids else 0
 
            # 为成分股成交生成唯一的客户端订单 ID（避免订单状态冲突）
            leg_client_order_id = ClientOrderId(f"{order.client_order_id.value}-LEG-{leg_instrument_id.symbol.value}")
 
            # 为成分股成交生成唯一的场所订单 ID
            leg_venue_order_id = VenueOrderId(f"{order.venue_order_id.value}-LEG-{leg_position}")
 
            # 为成分股成交生成唯一的交易 ID（匹配 IB 模式：{execution.execId}-{leg_position}）
            # 使用与组合成交相同的基础执行 ID 格式，但附加成分股位置
            leg_trade_id = TradeId(f"{self.venue.to_str()}-{self.raw_id}-{self._execution_count:03d}-{leg_position}")
 
            # 根据价差订单方向映射成分股侧
            # 如果价差买入 (BUY)：正比率 = 买入成分股，负比率 = 卖出成分股
            # 如果价差卖出 (SELL)：正比率 = 卖出成分股，负比率 = 买入成分股
            order_side = order.side if ratio > 0 else (OrderSide.SELL if order.side == OrderSide.BUY else OrderSide.BUY)
 
            # 为成分股创建 OrderFilled 事件
            ts_now = self._clock.timestamp_ns()
            leg_fill = OrderFilled(
                trader_id=order.trader_id,
                strategy_id=order.strategy_id,
                instrument_id=leg_instrument_id,
                client_order_id=leg_client_order_id,  # 使用唯一的成分股客户端订单 ID
                venue_order_id=leg_venue_order_id,  # 使用唯一的成分股场所订单 ID
                account_id=order.account_id,
                trade_id=leg_trade_id,
                order_side=order_side,
                order_type=order.order_type,
                last_qty=adjusted_leg_quantity,
                last_px=adjusted_leg_price,
                currency=leg_instrument.quote_currency,
                liquidity_side=liquidity_side,
                event_id=UUID4(),
                ts_event=ts_now,
                ts_init=ts_now,
                reconciliation=False,
                position_id=None,
                commission=commission,
            )
 
            # 发布成分股成交事件（与常规订单成交相同）
            self.msgbus.send(endpoint="ExecEngine.process", msg=leg_fill)
 
    cdef dict _calculate_leg_execution_prices(
        self,
        list leg_tuples,
        Price spread_execution_price,
        Quantity spread_quantity,
    ):
        """
        使用中间价加调整来计算成分股执行价格。
 
        对除了最高价格成分股之外的所有成分股使用中间价，
        最高价格成分股会被调整以满足：Σ(leg_price × ratio) = spread_execution_price
        """
        cdef dict[InstrumentId, double] leg_mid_prices = {}
        cdef dict[InstrumentId, Price] leg_prices = {}
        cdef double highest_mid_price = 0.0
        cdef InstrumentId highest_price_leg_id = None
 
        # 获取所有成分股的中间价
        for leg_instrument_id, ratio in leg_tuples:
            leg_quote = self.cache.quote_tick(leg_instrument_id)
 
            if leg_quote is None:
                self._log.warning(f"由于没有报价，导致成分股 {leg_instrument_id} 不可用")
                return {}
 
            mid_price = (leg_quote.bid_price.as_double() + leg_quote.ask_price.as_double()) * 0.5
            leg_mid_prices[leg_instrument_id] = mid_price
 
            # 跟踪中间价最高的成分股（它将被调整）
            if mid_price > highest_mid_price:
                highest_mid_price = mid_price
                highest_price_leg_id = leg_instrument_id
 
        if highest_price_leg_id is None:
            return {}
 
        # 计算加权和，对除了最高价格成分股之外的所有价格使用中间价
        cdef double weighted_sum = 0.0
        cdef int highest_price_ratio = 1
 
        for leg_instrument_id, ratio in leg_tuples:
            if leg_instrument_id != highest_price_leg_id:
                weighted_sum += leg_mid_prices[leg_instrument_id] * ratio
 
                # 获取实际工具以使用其 make_price 方法进行适当的 tick 舍入
                leg_instrument = self.cache.instrument(leg_instrument_id)
 
                if leg_instrument is not None:
                    leg_prices[leg_instrument_id] = leg_instrument.make_price(
                        leg_mid_prices[leg_instrument_id]
                    )
                else:
                    # 如果找不到工具，记录警告并中止
                    self._log.warning(
                        f"在缓存中找不到成分股工具 {leg_instrument_id}，"
                        f"中止价差的成分股价格计算"
                    )
                    return {}
            else:
                # 存储最高面额成分股的比率以用于调整计算
                highest_price_ratio = ratio
 
        # 为最高价格成分股计算调整后的价格
        # spread_execution_price = Σ(leg_price × ratio)
        # adjusted_price = (spread_execution_price - weighted_sum) / highest_price_ratio
        cdef double adjusted_price = (spread_execution_price.as_double() - weighted_sum) / highest_price_ratio
 
        # 获取最高价格成分股的实际工具以使用其 make_price 方法
        highest_leg_instrument = self.cache.instrument(highest_price_leg_id)
 
        if highest_leg_instrument is not None:
            leg_prices[highest_price_leg_id] = highest_leg_instrument.make_price(adjusted_price)
        else:
            # 如果找不到工具，记录警告并中止
            self._log.warning(
                f"在缓存中找不到最高面额的成分股工具 {highest_price_leg_id}，"
                f"中止价差的成分股价格计算"
            )
            return {}
 
        return leg_prices

    cpdef void fill_order(
        self,
        Order order,
        Price last_px,
        Quantity last_qty,
        LiquiditySide liquidity_side,
        PositionId venue_position_id: PositionId | None = None,
        Position position: Position | None = None,
    ):
        """
        将给定的成交应用到给定的订单。可选地提供现有的头寸详情。
 
        Parameters
        ----------
        order : Order
            要成交的订单。
        last_px : Price
            订单的成交价格。
        last_qty : Quantity
            订单的成交数量。
        liquidity_side : LiquiditySide
            成交的流动性侧。
        venue_position_id : PositionId, 可选
            与订单相关的当前场所头寸 ID（如果已分配）。
        position : Position, 可选
            与订单相关的当前头寸（如有）。
 
        Raises
        ------
        ValueError
            如果 `liquidity_side` 为 ``NO_LIQUIDITY_SIDE``。
 
        Warnings
        --------
        `liquidity_side` 将覆盖订单上之前设置的任何值。
 
        """
        Condition.not_none(order, "order")
        Condition.not_none(last_px, "last_px")
        Condition.not_none(last_qty, "last_qty")
        Condition.not_equal(liquidity_side, LiquiditySide.NO_LIQUIDITY_SIDE, "liquidity_side", "NO_LIQUIDITY_SIDE")

        # 使用工具数量精度作为单一事实来源
        cdef uint8_t size_prec = self._size_prec
 
        # 验证传入的成交精度是否与工具匹配
        if last_qty._mem.precision != size_prec:
            raise RuntimeError(
                f"成交的数量精度无效 {last_qty._mem.precision} "
                f"而工具数量精度为 {size_prec}；"
                f"请检查数据数量精度是否与 {self.instrument.id} 工具匹配"
            )

        order.liquidity_side = liquidity_side

        cdef Quantity cached_filled_qty = self._cached_filled_qty.get(order.client_order_id)
        cdef Quantity leaves_qty = None
        if cached_filled_qty is None:
            # 将首次成交限制在订单数量内，以避免超额成交
            last_qty = Quantity.from_raw_c(min(order.quantity._mem.raw, last_qty._mem.raw), size_prec)
            self._cached_filled_qty[order.client_order_id] = Quantity.from_raw_c(last_qty._mem.raw, size_prec)
        else:
            if order.quantity._mem.raw <= cached_filled_qty._mem.raw:
                self._core.delete_order(order)
                self._cached_filled_qty.pop(order.client_order_id, None)
                return

            leaves_qty = Quantity.from_raw_c(order.quantity._mem.raw - cached_filled_qty._mem.raw, size_prec)
            last_qty = Quantity.from_raw_c(min(leaves_qty._mem.raw, last_qty._mem.raw), size_prec)
            cached_filled_qty._mem.raw += last_qty._mem.raw

        # 当调整后的 last_qty <= 0 时，无需进行任何填充。
        # 先更新 _cached_filled_qty 以吸收重复或乱序的成交
        # （在沙盒/异步环境中可见），并避免发出零或负数成交。
        if last_qty <= 0:
            return
 
        # 计算佣金
        cdef Money commission = self._fee_model.get_commission(
            order=order,
            fill_qty=last_qty,
            fill_px=last_px,
            instrument=self.instrument,
        )

        self._generate_order_filled(
            order=order,
            venue_order_id=self._get_venue_order_id(order),
            venue_position_id=venue_position_id,
            last_qty=last_qty,
            last_px=last_px,
            quote_currency=self.instrument.quote_currency,
            commission=commission,
            liquidity_side=order.liquidity_side,
        )

        if order.is_passive_c() and order.is_closed_c():
            # 从市场中移除订单
            self._core.delete_order(order)
            self._cached_filled_qty.pop(order.client_order_id, None)

        if not self._support_contingent_orders:
            return

        # 检查关联订单
        cdef ClientOrderId client_order_id
        cdef Order child_order
        if order.contingency_type == ContingencyType.OTO:
            for client_order_id in order.linked_order_ids or []:
                child_order = self.cache.order(client_order_id)
                assert child_order is not None, "未找到 OTO 子订单"
 
                if child_order.is_closed_c():
                    continue
 
                if child_order.is_active_local_c():
                    continue  # 订单尚未进入交易所
 
                if child_order.position_id is None and order.position_id is not None:
                    self.cache.add_position_id(
                        position_id=order.position_id,
                        venue=self.venue,
                        client_order_id=client_order_id,
                        strategy_id=child_order.strategy_id,
                    )
                    self._log.debug(
                        f"已为 {child_order.client_order_id!r} "
                        f"索引 {order.position_id!r}",
                    )
                if not child_order.is_open_c() or (child_order.status_c() == OrderStatus.PENDING_UPDATE and child_order._previous_status == OrderStatus.SUBMITTED):
                    self.process_order(
                        order=child_order,
                        account_id=order.account_id or self._account_ids[order.trader_id],
                    )
        elif order.contingency_type == ContingencyType.OCO:
            for client_order_id in order.linked_order_ids or []:
                oco_order = self.cache.order(client_order_id)
                assert oco_order is not None, "未找到 OCO 订单"
 
                if oco_order.is_closed_c():
                    continue
 
                if oco_order.is_active_local_c():
                    continue  # 订单尚未进入交易所
 
                self.cancel_order(oco_order)
        elif order.contingency_type == ContingencyType.OUO:
            for client_order_id in order.linked_order_ids or []:
                ouo_order = self.cache.order(client_order_id)
                assert ouo_order is not None, "未找到 OUO 订单"
 
                if ouo_order.is_active_local_c():
                    continue  # 订单尚未进入交易所
 
                if order.is_closed_c() and ouo_order.is_open_c():
                    self.cancel_order(ouo_order)
                elif order.leaves_qty._mem.raw != 0 and order.leaves_qty._mem.raw != ouo_order.leaves_qty._mem.raw:
                    self.update_order(
                        ouo_order,
                        order.leaves_qty,
                        price=ouo_order.price if ouo_order.has_price_c() else None,
                        trigger_price=ouo_order.trigger_price if ouo_order.has_trigger_price_c() else None,
                        update_contingencies=False,
                    )

        if position is None:
            return  # 成交完成
 
        # 检查头寸的只减仓 (reduce only) 订单
        # 以前，所有只减仓订单都被强制同步到净头寸大小，
        # 这种方式错误地合并了独立挂单组 (bracket orders) 之间的数量。
        # 现在，优先将每个只减仓子订单（止盈/止损）与其自身的父入口订单成交数量同步（如有可用）；
        # 仅对没有父订单的独立只减仓订单回退到同步头寸大小。
        cdef:
            Order ro_order
            Order parent_order
            Quantity cached_ro_filled
            Quantity cached_parent_filled
            Quantity target_qty
        for ro_order in self.cache.orders_for_position(position.id):
            if (
                self._use_reduce_only
                and ro_order.is_reduce_only
                and ro_order.is_open_c()
                and ro_order.is_passive_c()
            ):
                # 跳过正在成交的订单 - 它已经在处理中
                if ro_order.client_order_id == order.client_order_id:
                    continue
 
                if position.quantity._mem.raw == 0:
                    self.cancel_order(ro_order)
                    continue
 
                # 订单对象可能尚未通过成交更新
                cached_ro_filled = self._cached_filled_qty.get(ro_order.client_order_id, ro_order.filled_qty)
 
                # 使用 Quantity 对象进行比较，以正确处理精度
                parent_order = None
                if ro_order.parent_order_id is not None:
                    parent_order = self.cache.order(ro_order.parent_order_id)
 
                target_qty = position.quantity
 
                if parent_order is not None:
                    cached_parent_filled = self._cached_filled_qty.get(parent_order.client_order_id, parent_order.filled_qty)
 
                    # 使用父订单填充数量和头寸数量的最小值
                    if cached_parent_filled < position.quantity:
                        target_qty = cached_parent_filled
 
                # 安全钳位：更新后的总量绝不能低于已成交的数量
                if cached_ro_filled > target_qty:
                    target_qty = cached_ro_filled
 
                if ro_order.quantity != target_qty:
                    self.update_order(
                        ro_order,
                        target_qty,
                        price=ro_order.price if ro_order.has_price_c() else None,
                        trigger_price=ro_order.trigger_price if ro_order.has_trigger_price_c() else None,
                    )

# -- 标识符生成器 ------------------------------------------------------------------------

    cdef VenueOrderId _get_venue_order_id(self, Order order):
        # 检查订单上是否已存在
        cdef VenueOrderId venue_order_id = order.venue_order_id
        if venue_order_id is not None:
            return venue_order_id

        # 检查缓存中是否已存在
        venue_order_id = self.cache.venue_order_id(order.client_order_id)
        if venue_order_id is not None:
            return venue_order_id

        venue_order_id = self._generate_venue_order_id()
        self.cache.add_venue_order_id(order.client_order_id, venue_order_id)

        return venue_order_id

    cdef PositionId _get_position_id(self, Order order, bint generate=True):
        cdef PositionId position_id
        if self.oms_type == OmsType.HEDGING:
            position_id = self.cache.position_id(order.client_order_id)

            if position_id is not None:
                return position_id

            if generate:
                # 生成场所头寸 ID
                return self._generate_venue_position_id()

        ####################################################################
        # 净头寸 (NETTING) OMS (头寸 ID 将为 `{instrument_id}-{strategy_id}`)
        ####################################################################
        cdef list[Position] positions_open = self.cache.positions_open(
            venue=None,  # 更快的查询过滤
            instrument_id=order.instrument_id,
        )
        if positions_open:
            return positions_open[0].id
        else:
            return None

    cdef PositionId _generate_venue_position_id(self):
        if not self._use_position_ids:
            return None

        self._position_count += 1

        if self._use_random_ids:
            return PositionId(str(uuid.uuid4()))
        else:
            return PositionId(f"{self.venue.to_str()}-{self.raw_id}-{self._position_count:03d}")

    cdef VenueOrderId _generate_venue_order_id(self):
        self._order_count += 1

        if self._use_random_ids:
            return VenueOrderId(str(uuid.uuid4()))
        else:
            return VenueOrderId(f"{self.venue.to_str()}-{self.raw_id}-{self._order_count:03d}")

    cdef TradeId _generate_trade_id(self):
        self._execution_count += 1
        return TradeId(self._generate_trade_id_str())

    cdef str _generate_trade_id_str(self):
        if self._use_random_ids:
            return str(uuid.uuid4())
        else:
            return f"{self.venue.to_str()}-{self.raw_id}-{self._execution_count:03d}"

# -- 事件处理 -------------------------------------------------------------------------------

    cpdef void accept_order(self, Order order):
        if order.is_closed_c():
            return  # 临时防护，防止无效处理

        # 检查订单是否已被接受（被添加回撮合引擎）
        if not order.status_c() == OrderStatus.ACCEPTED:
            self._generate_order_accepted(order, venue_order_id=self._get_venue_order_id(order))

            if (
                order.order_type == OrderType.TRAILING_STOP_MARKET
                or order.order_type == OrderType.TRAILING_STOP_LIMIT
            ):
                if order.trigger_price is None:
                    self._trail_stop_order(order)

        self._core.add_order(order)

    cpdef void expire_order(self, Order order):
        if self._support_contingent_orders and order.contingency_type != ContingencyType.NO_CONTINGENCY:
            self._cancel_contingent_orders(order)

        self._generate_order_expired(order)

    cpdef void cancel_order(self, Order order, bint cancel_contingencies=True):
        if order.is_active_local_c():
            self._log.error(
                f"无法从撮合引擎取消状态为 {order.status_string_c()} 的订单",
            )
            return

        self._core.delete_order(order)
        self._cached_filled_qty.pop(order.client_order_id, None)

        self._generate_order_canceled(order, venue_order_id=self._get_venue_order_id(order))

        if self._support_contingent_orders and order.contingency_type != ContingencyType.NO_CONTINGENCY and cancel_contingencies:
            self._cancel_contingent_orders(order)

    cpdef void update_order(
        self,
        Order order,
        Quantity qty,
        Price price = None,
        Price trigger_price = None,
        bint update_contingencies = True,
    ):
        if qty is None:
            qty = order.quantity

        # 验证更新参数的精度（必须 <= 工具精度）
        if qty._mem.precision > self._size_prec:
            raise RuntimeError(
                f"更新数量精度 {qty._mem.precision} 无效，"
                f"而 {self.instrument.id} 的数量精度为 {self._size_prec}"
            )
        if price is not None and price._mem.precision > self._price_prec:
            raise RuntimeError(
                f"更新价格精度 {price._mem.precision} 无效，"
                f"而 {self.instrument.id} 的价格精度为 {self._price_prec}"
            )
        if trigger_price is not None and trigger_price._mem.precision > self._price_prec:
            raise RuntimeError(
                f"更新触发价格精度 {trigger_price._mem.precision} 无效，"
                f"而 {self.instrument.id} 的价格精度为 {self._price_prec}"
            )

        # 使用 _cached_filled_qty，因为订单对象的 filled_qty 可能尚未更新
        cdef Quantity filled_qty = self._cached_filled_qty.get(order.client_order_id, order.filled_qty)
        if qty < filled_qty:
            self._generate_order_modify_rejected(
                trader_id=order.trader_id,
                strategy_id=order.strategy_id,
                account_id=order.account_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                venue_order_id=order.venue_order_id,
                reason=f"无法将订单数量 {qty} 降低至已成交数量 {filled_qty} 以下",
            )
            return

        if order.order_type == OrderType.LIMIT or order.order_type == OrderType.MARKET_TO_LIMIT:
            if price is None:
                price = order.price

            self._update_limit_order(order, qty, price)
        elif order.order_type == OrderType.STOP_MARKET:
            if trigger_price is None:
                trigger_price = order.trigger_price

            self._update_stop_market_order(order, qty, trigger_price)
        elif order.order_type == OrderType.STOP_LIMIT:
            if price is None:
                price = order.price

            if trigger_price is None:
                trigger_price = order.trigger_price

            self._update_stop_limit_order(order, qty, price, trigger_price)
        elif order.order_type == OrderType.MARKET_IF_TOUCHED:
            if trigger_price is None:
                trigger_price = order.trigger_price

            self._update_market_if_touched_order(order, qty, trigger_price)
        elif order.order_type == OrderType.LIMIT_IF_TOUCHED:
            if price is None:
                price = order.price

            if trigger_price is None:
                trigger_price = order.trigger_price

            self._update_limit_if_touched_order(order, qty, price, trigger_price)
        elif order.order_type == OrderType.TRAILING_STOP_MARKET:
            if trigger_price is None:
                trigger_price = order.trigger_price

            self._update_trailing_stop_market_order(order, qty, trigger_price)
        elif order.order_type == OrderType.TRAILING_STOP_LIMIT:
            if price is None:
                price = order.price

            if trigger_price is None:
                trigger_price = order.trigger_price

            self._update_trailing_stop_limit_order(order, qty, price, trigger_price)
        else:
            raise ValueError(
                f"无效的 `OrderType`，为 {order.order_type}")  # pragma: no cover (设计时错误)

        # 如果更新后订单的剩余数量为零，则取消该订单
        cdef QuantityRaw new_leaves_raw = qty._mem.raw - filled_qty._mem.raw if qty._mem.raw > filled_qty._mem.raw else 0
        if new_leaves_raw == 0:
            if self._support_contingent_orders and order.contingency_type != ContingencyType.NO_CONTINGENCY and update_contingencies:
                self._update_contingent_orders(order)
            # 传入 False，因为我们已经在上面处理了关联订单
            self.cancel_order(order, cancel_contingencies=False)
            return

        if self._support_contingent_orders and order.contingency_type != ContingencyType.NO_CONTINGENCY and update_contingencies:
            self._update_contingent_orders(order)

    cpdef void trigger_stop_order(self, Order order):
        # 始终为 STOP_LIMIT 或 LIMIT_IF_TOUCHED 订单
        cdef Price trigger_price = order.trigger_price
        cdef Price price = order.price

        self._generate_order_triggered(order)

        # 检查是否立即成交（将作为挂单 (MAKER) 被动成交）
        if order.side == OrderSide.BUY and trigger_price._mem.raw > price._mem.raw > self._core.ask_raw:
            order.liquidity_side = LiquiditySide.MAKER
            self.fill_limit_order(order)
            return
        elif order.side == OrderSide.SELL and trigger_price._mem.raw < price._mem.raw < self._core.bid_raw:
            order.liquidity_side = LiquiditySide.MAKER
            self.fill_limit_order(order)
            return

        if self._core.is_limit_matched(order.side, price):
            if order.is_post_only:
                # 将成为流动性提取者 (TAKER)
                self._core.delete_order(order)
                self._cached_filled_qty.pop(order.client_order_id, None)
                self._generate_order_rejected(
                    order,
                    f"只挂单 (POST_ONLY) {order.type_string_c()} {order.side_string_c()} 订单 "
                    f"限价 {order.price} 会导致其成为提取者 (TAKER): "
                    f"买入价={self._core.bid}, "
                    f"卖出价={self._core.ask}",
                    True,  # 由于只挂单原因 (due_post_only)
                )
                return

            order.liquidity_side = LiquiditySide.TAKER
            self.fill_limit_order(order)

    cdef void _update_contingent_orders(self, Order order):
        self._log.debug(f"正在从 {order.client_order_id} 更新 OUO 订单", LogColor.MAGENTA)

        cdef Quantity parent_filled_qty = self._cached_filled_qty.get(order.client_order_id, order.filled_qty)
        cdef QuantityRaw parent_leaves_raw = order.quantity._mem.raw - parent_filled_qty._mem.raw if order.quantity._mem.raw > parent_filled_qty._mem.raw else 0

        cdef ClientOrderId client_order_id
        cdef Order ouo_order
        cdef Quantity child_filled_qty
        cdef QuantityRaw child_leaves_raw
        for client_order_id in order.linked_order_ids or []:
            ouo_order = self.cache.order(client_order_id)
            assert ouo_order is not None, "未找到 OUO 订单"

            if ouo_order.is_active_local_c():
                continue  # 订单尚未进入交易所

            if ouo_order.order_type == OrderType.MARKET or ouo_order.is_closed_c():
                continue

            child_filled_qty = self._cached_filled_qty.get(ouo_order.client_order_id, ouo_order.filled_qty)

            if parent_leaves_raw == 0:
                self.cancel_order(ouo_order, cancel_contingencies=False)
            elif child_filled_qty._mem.raw >= parent_leaves_raw:
                # 子订单成交已超过父订单剩余数量，将其取消
                self.cancel_order(ouo_order, cancel_contingencies=False)
            else:
                child_leaves_raw = ouo_order.quantity._mem.raw - child_filled_qty._mem.raw if ouo_order.quantity._mem.raw > child_filled_qty._mem.raw else 0
                if child_leaves_raw != parent_leaves_raw:
                    self.update_order(
                        ouo_order,
                        Quantity.from_raw_c(parent_leaves_raw, self._size_prec),
                        price=ouo_order.price if ouo_order.has_price_c() else None,
                        trigger_price=ouo_order.trigger_price if ouo_order.has_trigger_price_c() else None,
                        update_contingencies=False,
                    )

    cdef void _cancel_contingent_orders(self, Order order):
        # 迭代所有关联订单，如果处于活跃状态则将其取消
        cdef ClientOrderId client_order_id
        cdef Order contingent_order
        for client_order_id in order.linked_order_ids or []:
            contingent_order = self.cache.order(client_order_id)
            assert contingent_order is not None, "未找到关联订单"

            if contingent_order.is_active_local_c():
                continue  # 订单尚未进入交易所

            if not contingent_order.is_closed_c():
                self.cancel_order(contingent_order, cancel_contingencies=False)

# -- EVENT GENERATORS -----------------------------------------------------------------------------

    cdef void _generate_order_rejected(self, Order order, str reason, bint due_post_only=False):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderRejected event = OrderRejected(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            account_id=order.account_id or self._account_ids[order.trader_id],
            reason=reason,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
            due_post_only=due_post_only,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_accepted(self, Order order, VenueOrderId venue_order_id):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderAccepted event = OrderAccepted(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=venue_order_id,
            account_id=order.account_id or self._account_ids[order.trader_id],
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_modify_rejected(
        self,
        TraderId trader_id,
        StrategyId strategy_id,
        AccountId account_id,
        InstrumentId instrument_id,
        ClientOrderId client_order_id,
        VenueOrderId venue_order_id,
        str reason,
    ):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderModifyRejected event = OrderModifyRejected(
            trader_id=trader_id,
            strategy_id=strategy_id,
            instrument_id=instrument_id,
            client_order_id=client_order_id,
            venue_order_id=venue_order_id,
            account_id=account_id,
            reason=reason,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_cancel_rejected(
        self,
        TraderId trader_id,
        StrategyId strategy_id,
        AccountId account_id,
        InstrumentId instrument_id,
        ClientOrderId client_order_id,
        VenueOrderId venue_order_id,
        str reason,
    ):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderCancelRejected event = OrderCancelRejected(
            trader_id=trader_id,
            strategy_id=strategy_id,
            instrument_id=instrument_id,
            client_order_id=client_order_id,
            venue_order_id=venue_order_id,
            account_id=account_id,
            reason=reason,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cpdef void _generate_order_updated(
        self,
        Order order,
        Quantity quantity,
        Price price,
        Price trigger_price,
    ):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderUpdated event = OrderUpdated(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=order.venue_order_id,
            account_id=order.account_id or self._account_ids[order.trader_id],
            quantity=quantity,
            price=price,
            trigger_price=trigger_price,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )

        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_canceled(self, Order order, VenueOrderId venue_order_id):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderCanceled event = OrderCanceled(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=venue_order_id,
            account_id=order.account_id or self._account_ids[order.trader_id],
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_triggered(self, Order order):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderTriggered event = OrderTriggered(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=order.venue_order_id,
            account_id=order.account_id or self._account_ids[order.trader_id],
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_expired(self, Order order):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderExpired event = OrderExpired(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=order.venue_order_id,
            account_id=order.account_id or self._account_ids[order.trader_id],
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)

    cdef void _generate_order_filled(
        self,
        Order order,
        VenueOrderId venue_order_id,
        PositionId venue_position_id,
        Quantity last_qty,
        Price last_px,
        Currency quote_currency,
        Money commission,
        LiquiditySide liquidity_side
    ):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderFilled event = OrderFilled(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=venue_order_id,
            account_id=order.account_id or self._account_ids[order.trader_id],
            trade_id=self._generate_trade_id(),
            position_id=venue_position_id,
            order_side=order.side,
            order_type=order.order_type,
            last_qty=last_qty,
            last_px=last_px,
            currency=quote_currency,
            commission=commission,
            liquidity_side=liquidity_side,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )
        self.msgbus.send(endpoint="ExecEngine.process", msg=event)
