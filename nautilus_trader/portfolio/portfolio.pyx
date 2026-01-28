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

"""
`Portfolio` 促进交易操作的管理。

预期的用例是每个运行系统有一个 ``Portfolio`` 实例，
一队交易策略将协助 `Trader` 类围绕投资组合进行组织。

投资组合可以满足对账户信息、保证金余额、总风险敞口和总净头寸的查询。
"""

import pickle
from collections import defaultdict
from decimal import Decimal

from nautilus_trader.analysis import AvgLoser
from nautilus_trader.analysis import AvgWinner
from nautilus_trader.analysis import Expectancy
from nautilus_trader.analysis import LongRatio
from nautilus_trader.analysis import MaxLoser
from nautilus_trader.analysis import MaxWinner
from nautilus_trader.analysis import MinLoser
from nautilus_trader.analysis import MinWinner
from nautilus_trader.analysis import PortfolioAnalyzer
from nautilus_trader.analysis import ProfitFactor
from nautilus_trader.analysis import ReturnsAverage
from nautilus_trader.analysis import ReturnsAverageLoss
from nautilus_trader.analysis import ReturnsAverageWin
from nautilus_trader.analysis import ReturnsVolatility
from nautilus_trader.analysis import RiskReturnRatio
from nautilus_trader.analysis import SharpeRatio
from nautilus_trader.analysis import SortinoRatio
from nautilus_trader.analysis import WinRate
from nautilus_trader.core import nautilus_pyo3
from nautilus_trader.portfolio.config import PortfolioConfig

from libc.stdint cimport uint64_t

from nautilus_trader.accounting.accounts.base cimport Account
from nautilus_trader.accounting.factory cimport AccountFactory
from nautilus_trader.accounting.manager cimport AccountsManager
from nautilus_trader.cache.base cimport CacheFacade
from nautilus_trader.common.component cimport LogColor
from nautilus_trader.common.component cimport Logger
from nautilus_trader.common.component cimport MessageBus
from nautilus_trader.core.correctness cimport Condition
from nautilus_trader.core.rust.core cimport NANOSECONDS_IN_MILLISECOND
from nautilus_trader.core.rust.model cimport FIXED_PRECISION
from nautilus_trader.core.rust.model cimport AccountType
from nautilus_trader.core.rust.model cimport OrderSide
from nautilus_trader.core.rust.model cimport OrderType
from nautilus_trader.core.rust.model cimport PositionSide
from nautilus_trader.core.rust.model cimport PriceType
from nautilus_trader.model.data cimport Bar
from nautilus_trader.model.data cimport QuoteTick
from nautilus_trader.model.events.account cimport AccountState
from nautilus_trader.model.events.order cimport OrderAccepted
from nautilus_trader.model.events.order cimport OrderCanceled
from nautilus_trader.model.events.order cimport OrderEvent
from nautilus_trader.model.events.order cimport OrderExpired
from nautilus_trader.model.events.order cimport OrderFilled
from nautilus_trader.model.events.order cimport OrderRejected
from nautilus_trader.model.events.order cimport OrderUpdated
from nautilus_trader.model.events.position cimport PositionEvent
from nautilus_trader.model.functions cimport position_side_to_str
from nautilus_trader.model.functions cimport price_type_to_str
from nautilus_trader.model.identifiers cimport AccountId
from nautilus_trader.model.identifiers cimport InstrumentId
from nautilus_trader.model.identifiers cimport PositionId
from nautilus_trader.model.identifiers cimport Venue
from nautilus_trader.model.instruments.base cimport Instrument
from nautilus_trader.model.instruments.betting cimport BettingInstrument
from nautilus_trader.model.instruments.betting cimport order_side_to_bet_side
from nautilus_trader.model.instruments.crypto_perpetual cimport CryptoPerpetual
from nautilus_trader.model.instruments.currency_pair cimport CurrencyPair
from nautilus_trader.model.objects cimport Currency
from nautilus_trader.model.objects cimport Money
from nautilus_trader.model.orders.base cimport Order
from nautilus_trader.model.position cimport Position
from nautilus_trader.portfolio.base cimport PortfolioFacade


cdef tuple[OrderEvent] _UPDATE_ORDER_EVENTS = (
    OrderAccepted,
    OrderCanceled,
    OrderExpired,
    OrderRejected,
    OrderUpdated,
    OrderFilled,
)


cdef class Portfolio(PortfolioFacade):
    """
提供交易组合。

目前限制为每个 ``ExecutionClient`` 实例对应一个账户。

参数
----------
msgbus : MessageBus
    引擎的消息总线。
cache : CacheFacade
    投资组合的只读缓存。
clock : Clock
    投资组合的时钟。
config : PortfolioConfig, 可选
    实例的配置。

引发
------
TypeError
    如果 `config` 不是 `PortfolioConfig` 类型。
    """

    def __init__(
        self,
        MessageBus msgbus not None,
        CacheFacade cache not None,
        Clock clock not None,
        config: PortfolioConfig | None = None,
    ) -> None:
        if config is None:
            config = PortfolioConfig()

        Condition.type(config, PortfolioConfig, "config")

        self._clock = clock
        self._log = Logger(name=type(self).__name__)
        self._msgbus = msgbus
        self._cache = cache
        self._accounts = AccountsManager(
            cache=cache,
            clock=clock,
            logger=self._log,
        )

        # 配置
        self._config: PortfolioConfig = config
        self._debug: bool = config.debug
        self._use_mark_prices: bool = config.use_mark_prices
        self._use_mark_xrates: bool = config.use_mark_xrates
        self._convert_to_account_base_currency: bool = config.convert_to_account_base_currency
        self._log_price: str = "标记价格" if config.use_mark_prices else "报价、交易或 Bar 价格"
        self._log_xrate: str = "标记" if config.use_mark_xrates else "计算所需数据"

        if config.min_account_state_logging_interval_ms:
            interval_ns = config.min_account_state_logging_interval_ms * NANOSECONDS_IN_MILLISECOND
            self._min_account_state_logging_interval_ns = interval_ns

        self._unrealized_pnls: dict[InstrumentId, dict[AccountId, Money]] = {}
        self._realized_pnls: dict[InstrumentId, dict[AccountId, Money]] = {}
        self._snapshot_sum_per_position: dict[PositionId, Money] = {}
        self._snapshot_last_per_position: dict[PositionId, Money] = {}
        self._snapshot_processed_counts: dict[PositionId, int] = {}
        self._snapshot_account_ids: dict[PositionId, AccountId] = {}
        self._net_positions: dict[InstrumentId, dict[AccountId, Decimal]] = {}
        self._bet_positions: dict[InstrumentId, object] = {}
        self._index_bet_positions: dict[InstrumentId, set[PositionId]] = defaultdict(set)
        self._pending_calcs: set[InstrumentId] = set()
        self._bar_close_prices: dict[InstrumentId, Price] = {}
        self._last_account_state_log_ts: dict[AccountId, uint64_t] = {}

        self.analyzer = PortfolioAnalyzer()

        # 注册默认统计指标
        self.analyzer.register_statistic(MaxWinner())
        self.analyzer.register_statistic(AvgWinner())
        self.analyzer.register_statistic(MinWinner())
        self.analyzer.register_statistic(MinLoser())
        self.analyzer.register_statistic(AvgLoser())
        self.analyzer.register_statistic(MaxLoser())
        self.analyzer.register_statistic(Expectancy())
        self.analyzer.register_statistic(WinRate())
        self.analyzer.register_statistic(ReturnsVolatility())
        self.analyzer.register_statistic(ReturnsAverage())
        self.analyzer.register_statistic(ReturnsAverageLoss())
        self.analyzer.register_statistic(ReturnsAverageWin())
        self.analyzer.register_statistic(SharpeRatio())
        self.analyzer.register_statistic(SortinoRatio())
        self.analyzer.register_statistic(ProfitFactor())
        self.analyzer.register_statistic(RiskReturnRatio())
        self.analyzer.register_statistic(LongRatio())

        # 注册端点
        self._msgbus.register(endpoint="Portfolio.update_account", handler=self.update_account)
        self._msgbus.register(endpoint="Portfolio.update_order", handler=self.update_order)
        self._msgbus.register(endpoint="Portfolio.update_position", handler=self.update_position)

        # 必需的订阅
        self._msgbus.subscribe(topic="events.order.*", handler=self.on_order_event, priority=10)
        self._msgbus.subscribe(topic="events.position.*", handler=self.on_position_event, priority=10)

        if config.use_mark_prices:
            self._msgbus.subscribe(topic="data.mark_prices.*", handler=self.update_mark_price, priority=10)
        else:
            self._msgbus.subscribe(topic="data.quotes.*", handler=self.update_quote_tick, priority=10)

        if config.bar_updates:
            self._msgbus.subscribe(topic="data.bars.*EXTERNAL", handler=self.update_bar, priority=10)

        self.initialized = False

# -- COMMANDS -------------------------------------------------------------------------------------

    cpdef void set_use_mark_prices(self, bint value):
        """
        使用给定的 `value` 设置 `use_mark_prices` 设置。

        参数
        ----------
        value : bool
            要设置的值。

        """
        self._use_mark_prices = value

    cpdef void set_use_mark_xrates(self, bint value):
        """
        使用给定的 `value` 设置 `use_mark_xrates` 设置。

        参数
        ----------
        value : bool
            要设置的值。

        """
        self._use_mark_xrates = value

    cpdef void initialize_orders(self):
        """
        初始化投资组合订单。

        对当前订单状态执行所有账户计算。
        """
        cdef list all_orders_open = self._cache.orders_open()
        cdef set instruments = set()
        cdef Order order
        for order in all_orders_open:
            instruments.add(order.instrument_id)

        # 更新初始（订单）保证金以初始化投资组合
        cdef bint initialized = True
        cdef:
            Order o
            list orders_open
            bint result
            dict orders_by_account
            AccountId account_id
            Account account
            list account_orders
        for instrument_id in instruments:
            instrument = self._cache.instrument(instrument_id)
            if instrument is None:
                self._log.error(
                    f"无法更新初始（订单）保证金： "
                    f"在缓存中未找到工具 {instrument_id}",
                )
                initialized = False
                break

            orders_open = self._cache.orders_open(
                venue=None,  # 更快的查询过滤
                instrument_id=instrument.id,
            )
            orders_by_account = self._group_by_account_id(orders_open)
            for account_id, account_orders in orders_by_account.items():
                account = self._cache.account(account_id)
                if account is None:
                    self._log.error(
                        f"无法更新初始（订单）保证金： "
                        f"在缓存中未找到账户 {account_id}",
                    )
                    initialized = False
                    break

                result = self._accounts.update_orders(
                    account=account,
                    instrument=instrument,
                    orders_open=[o for o in account_orders if o.is_passive_c()],
                    ts_event=account.last_event_c().ts_event,
                )
                if not result:
                    initialized = False

            if not initialized:
                break

        cdef int open_count = len(all_orders_open)
        self._log.info(
            f"已初始化 {open_count} 个未平仓订单",
            color=LogColor.BLUE if open_count else LogColor.NORMAL,
        )
        self.initialized = initialized

    cpdef void initialize_positions(self):
        """
        初始化投资组合头寸。

        针对当前持仓状态执行所有账户计算。
        """
        # 清空数据
        self._realized_pnls.clear()
        self._unrealized_pnls.clear()

        cdef list all_positions_open = self._cache.positions_open()
        cdef set instruments = set()
        cdef Position position
        for position in all_positions_open:
            instruments.add(position.instrument_id)

        cdef bint initialized = True

        # 更新维持（持仓）保证金以初始化投资组合
        cdef:
            InstrumentId instrument_id
            Instrument instrument
            list positions_open
            Account account
            bint result
            dict positions_by_account
            list account_positions
            AccountId account_id
            Money realized_pnl
            Money unrealized_pnl
        for instrument_id in instruments:
            instrument = self._cache.instrument(instrument_id)
            if instrument is None:
                self._log.error(
                    f"无法更新维持（持仓）保证金： "
                    f"在缓存中未找到工具 {instrument_id}",
                )
                initialized = False
                break

            positions_open = self._cache.positions_open(
                venue=None,  # 更快的查询过滤
                instrument_id=instrument_id,
            )

            self._update_net_position(
                instrument_id=instrument_id,
                positions_open=positions_open,
            )

            positions_by_account = self._group_by_account_id(positions_open)
            for account_id, account_positions in positions_by_account.items():
                account = self._cache.account(account_id)
                if account is None:
                    self._log.error(
                        f"无法更新维持（持仓）保证金： "
                        f"在缓存中未找到账户 {account_id}",
                    )
                    initialized = False
                    continue

                # 为该账户计算并缓存盈亏
                realized_pnl = self.realized_pnl(instrument_id, account_id)
                unrealized_pnl = self.unrealized_pnl(instrument_id, price=None, account_id=account_id)

                if account.type != AccountType.MARGIN:
                    continue

                result = self._accounts.update_positions(
                    account=account,
                    instrument=instrument,
                    positions_open=account_positions,
                    ts_event=account.last_event_c().ts_event,
                )
                if not result:
                    initialized = False

        cdef int open_count = len(all_positions_open)
        self._log.info(
            f"已初始化 {open_count} 个未平仓头寸",
            color=LogColor.BLUE if open_count else LogColor.NORMAL,
        )
        self.initialized = initialized

    cpdef void update_quote_tick(self, QuoteTick tick):
        """
        使用给定的报价 Tick 更新投资组合。

        清除关联该工具的缓存未实现盈亏，并执行任何可能挂起的初始化计算。

        参数
        ----------
        quote_tick : QuoteTick
            要更新的报价 Tick。

        """
        Condition.not_none(tick, "tick")

        self._update_instrument_id(tick.instrument_id)

        cdef Instrument instrument
        if self._use_mark_xrates:
            instrument = self._cache.instrument(tick.instrument_id)
            if isinstance(instrument, (CurrencyPair, CryptoPerpetual)):
                self._update_mark_xrate(
                    instrument=instrument,
                    xrate=(tick.bid_price.as_double() + tick.ask_price.as_double()) / 2.0,
                    instrument_id=tick.instrument_id,
                )

    cpdef void update_mark_price(self, object mark_price):
        """
        使用给定的标记价格更新投资组合。
        """
        Condition.not_none(mark_price, "mark_price")

        self._update_instrument_id(mark_price.instrument_id)

        cdef Instrument instrument
        if self._use_mark_xrates:
            instrument = self._cache.instrument(mark_price.instrument_id)
            if isinstance(instrument, (CurrencyPair, CryptoPerpetual)):
                self._update_mark_xrate(
                    instrument=instrument,
                    xrate=(<Price>mark_price.value).as_double(),
                    instrument_id=mark_price.instrument_id,
                )

    cdef void _update_mark_xrate(self, Instrument instrument, double xrate, InstrumentId instrument_id):
        if xrate > 0:
            self._cache.set_mark_xrate(
                from_currency=instrument.base_currency,
                to_currency=instrument.quote_currency,
                xrate=xrate,
            )
        else:
            self._log.debug(f"跳过 {instrument_id} 的标记汇率更新：价格为零")

    cpdef void update_bar(self, Bar bar):
        """
        使用给定的 Bar 更新投资组合。

        清除该工具关联的缓存未实现盈亏，并执行任何可能挂起的初始化计算。

        参数
        ----------
        bar : Bar
            要更新的 Bar。

        """
        Condition.not_none(bar, "bar")

        cdef InstrumentId instrument_id = bar.bar_type.instrument_id
        self._bar_close_prices[instrument_id] = bar.close
        self._update_instrument_id(instrument_id)

    cpdef void update_account(self, AccountState event):
        """
        应用给定的账户状态。

        参数
        ----------
        event : AccountState
            要应用的账户状态。

        """
        Condition.not_none(event, "event")

        self._update_account(event)

        self._msgbus.publish_c(
            topic=f"events.account.{event.account_id}",
            msg=event,
        )

    cpdef void update_order(self, OrderEvent event):
        """
        使用给定的订单更新投资组合。

        参数
        ----------
        event : OrderEvent
            要更新的事件。

        """
        Condition.not_none(event, "event")

        cdef:
            Account account
            Instrument instrument
            PositionId position_id

        account, instrument = self._validate_event_account_and_instrument(event, "order")
        if account is None or instrument is None:
            return

        if not account.calculate_account_state:
            return  # 无需计算

        if not isinstance(event, _UPDATE_ORDER_EVENTS):
            return  # 账户状态无变化

        cdef Order order = self._cache.order(event.client_order_id)

        # 允许即使缓存中没有订单也继续执行 OrderFilled 事件
        # （例如，来自价差订单的腿填充或期权行权）
        # 余额更新只需要成交和持仓，而不需要订单
        if order is None and not isinstance(event, OrderFilled):
            self._log.error(
                f"无法更新订单： "
                f"{event.client_order_id!r} 在缓存中未找到",
            )
            return  # 未找到订单

        if isinstance(event, OrderRejected) and order.order_type != OrderType.STOP_LIMIT:
            return  # 账户状态无变化

        if self._debug:
            self._log.debug(f"正在更新 {order!r}", LogColor.MAGENTA)

        cdef Money unrealized_pnl
        if isinstance(event, OrderFilled):
            # 跳过组合合约工具成交的余额更新
            # 组合合约工具不创建持仓，只有腿成交应该更新余额以避免重复计算（组合成交 + 腿成交）
            if not instrument.is_spread():
                self._accounts.update_balances(
                    account=account,
                    instrument=instrument,
                    fill=event,
                )

            if isinstance(instrument, BettingInstrument):
                position_id = event.position_id or PositionId(instrument.id.value)
                bet_position = self._bet_positions.get(position_id)
                if bet_position is None:
                    bet_position = nautilus_pyo3.BetPosition()
                    self._bet_positions[position_id] = bet_position
                    self._index_bet_positions[instrument.id].add(position_id)

                bet = nautilus_pyo3.Bet(
                    price=event.last_px.as_decimal(),
                    stake=event.last_qty.as_decimal(),
                    side=order_side_to_bet_side(order_side=event.order_side),
                )
                if self._debug:
                    self._log.debug(f"正在应用 {bet} 到 {bet_position}", LogColor.MAGENTA)

                bet_position.add_bet(bet)

                if self._debug:
                    self._log.debug(f"{bet_position}", LogColor.MAGENTA)

            # 计算并为此账户缓存未实现盈亏
            unrealized_pnl = self.unrealized_pnl(event.instrument_id, price=None, account_id=account.id)

        cdef list orders_open = self._cache.orders_open(
            venue=None,  # 更快的查询过滤
            instrument_id=event.instrument_id,
            strategy_id=None,
            side=OrderSide.NO_ORDER_SIDE,
            account_id=event.account_id,
        )

        cdef Order o
        cdef list passive_orders = [o for o in orders_open if o.is_passive_c()]
        cdef bint result = self._accounts.update_orders(
            account=account,
            instrument=instrument,
            orders_open=passive_orders,
            ts_event=event.ts_event,
        )
        if not result:
            self._log.debug(f"已为 {instrument.id} 添加挂起计算")
            self._pending_calcs.add(instrument.id)

        # 始终在非成交事件或 update_orders 成功时更新现货账户的账户状态
        if account.is_cash_account or not isinstance(event, OrderFilled):
            # 仅更新非成交事件的账户状态（成交事件将在持仓更新时更新）
            self._update_account(self._accounts.generate_account_state(account, event.ts_event))

        self._log.debug(f"已从 {event} 更新")

    cpdef void update_position(self, PositionEvent event):
        """
        使用给定的持仓事件更新投资组合。

        参数
        ----------
        event : PositionEvent
            要更新的事件。

        """
        Condition.not_none(event, "event")

        # 获取该工具的所有持仓以计算全局净头寸
        cdef list all_positions_open = self._cache.positions_open(
            venue=None,  # 更快的查询过滤
            instrument_id=event.instrument_id,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=None,  # 获取净头寸的所有账户
        )
        self._update_net_position(
            instrument_id=event.instrument_id,
            positions_open=all_positions_open,
        )

        # 失效该工具和账户的缓存盈亏
        # 对于已实现盈亏，还要检查这是否是新的持仓周期 (NETTING OMS)
        # 这种情况下会影响所有账户，而不只是当前这一个
        cdef:
            bint invalidate_all_accounts = False
            Position updated_position
            set[PositionId] snapshot_ids
            Money last_snapshot_pnl

        # 检查此持仓更新是否代表新周期（已实现盈亏的已平仓持仓）
        # 对于 NETTING OMS，如果持仓已关闭并具有快照，则它可能是新周期
        updated_position = self._cache.position(event.position_id)
        if updated_position is not None and updated_position.is_closed_c() and updated_position.realized_pnl is not None:
            # 确保快照数据已缓存，以便我们可以进行比较
            self._ensure_snapshot_pnls_cached_for(event.instrument_id)

            # 检查此持仓 ID 是否具有快照
            snapshot_ids = self._cache.position_snapshot_ids(event.instrument_id)
            if event.position_id in snapshot_ids:
                # 与最后一个快照盈亏比较——如果不同，则是新周期
                last_snapshot_pnl = self._snapshot_last_per_position.get(event.position_id)
                if last_snapshot_pnl is not None and updated_position.realized_pnl is not None:
                    if (
                        updated_position.realized_pnl.currency != last_snapshot_pnl.currency
                        or updated_position.realized_pnl != last_snapshot_pnl
                    ):
                        # 检测到新周期——失效该工具的所有账户
                        invalidate_all_accounts = True
                else:
                    # 持仓有快照，但我们尚未缓存 last_pnl
                    # 这可能是新周期——为保险起见进行失效
                    invalidate_all_accounts = True

        if invalidate_all_accounts:
            # 失效该工具的所有账户（新周期影响所有）
            self._realized_pnls.pop(event.instrument_id, None)
        else:
            # 仅失效此账户
            if event.instrument_id in self._realized_pnls:
                self._realized_pnls[event.instrument_id].pop(event.account_id, None)

        if event.instrument_id in self._unrealized_pnls:
            self._unrealized_pnls[event.instrument_id].pop(event.account_id, None)

        cdef:
            Account account
            Instrument instrument
        account, instrument = self._validate_event_account_and_instrument(event, "position")
        if account is None or instrument is None:
            return

        if account.type != AccountType.MARGIN or not account.calculate_account_state:
            return  # 无需计算

        # 获取按 account_id 过滤的持仓以便进行账户特定更新
        cdef list account_positions = self._cache.positions_open(
            venue=None,  # 更快的查询过滤
            instrument_id=event.instrument_id,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=event.account_id,  # 按 account_id 过滤
        )

        cdef AccountState account_state
        cdef bint result = self._accounts.update_positions(
            account=account,
            instrument=instrument,
            positions_open=account_positions,
            ts_event=event.ts_event,
        )
        if result:
            account_state = self._accounts.generate_account_state(account, event.ts_event)
            self._update_account(account_state)

    cpdef void on_order_event(self, OrderEvent event):
        """
        接收到订单事件时执行的操作。

        参数
        ----------
        event : OrderEvent
            接收到的事件。

        """
        Condition.not_none(event, "event")

        if event.account_id is None:
            return  # 事件未分配账户

        if not isinstance(event, _UPDATE_ORDER_EVENTS):
            return  # 账户状态无变化

        if isinstance(event, OrderFilled):
            return  # 当接收到持仓事件时将发布账户事件

        cdef Account account = self._cache.account(event.account_id)
        if account is None:
            return  # 未注册账户

        cdef AccountState account_state = account.last_event_c()

        self._msgbus.publish_c(
            topic=f"events.account.{account.id}",
            msg=account_state,
        )

    cpdef void on_position_event(self, PositionEvent event):
        """
        在接收到持仓事件时执行的操作。

        参数
        ----------
        event : PositionEvent
            接收到的事件。

        """
        Condition.not_none(event, "event")

        if event.account_id is None:
            return  # 事件未分配账户

        cdef Account account = self._cache.account(event.account_id)
        if account is None:
            return  # 未注册账户

        cdef AccountState account_state = account.last_event_c()

        self._msgbus.publish_c(
            topic=f"events.account.{account.id}",
            msg=account_state,
        )

    def _reset(self) -> None:
        self._net_positions.clear()
        self._bet_positions.clear()
        self._index_bet_positions.clear()
        self._realized_pnls.clear()
        self._unrealized_pnls.clear()
        self._pending_calcs.clear()
        self._snapshot_sum_per_position.clear()
        self._snapshot_last_per_position.clear()
        self._snapshot_processed_counts.clear()
        self._snapshot_account_ids.clear()
        self.analyzer.reset()

        self.initialized = False

    def reset(self) -> None:
        """
        重置投资组合。

        所有有状态字段都重置为初始值。

        """
        self._log.debug(f"RESETTING")
        self._reset()
        self._log.info("READY")

    def dispose(self) -> None:
        """
        销毁投资组合。

        所有有状态字段都重置为初始值。

        """
        self._log.debug(f"DISPOSING")
        self._reset()
        self._log.info("DISPOSED")

# -- QUERIES --------------------------------------------------------------------------------------

    cpdef Account account(self, Venue venue=None, AccountId account_id=None):
        """
        返回给定场地或账户 ID 的账户（如果找到）。

        参数
        ----------
        venue : Venue, 可选
            账户的场地。
        account_id : AccountId, 可选
            账户 ID（如果同时提供了 venue 和 account_id，则优先考虑此项）。

        返回
        -------
        Account 或 ``None``

        """
        return self._get_account(venue, account_id, "account", "venue or account_id must be provided")

    cpdef dict balances_locked(self, Venue venue=None, AccountId account_id=None):
        """
        返回给定场地或账户 ID 的锁定余额（如果找到）。

        参数
        ----------
        venue : Venue, 可选
            账户的场地。
        account_id : AccountId, 可选
            账户 ID（如果同时提供了 venue 和 account_id，则优先考虑此项）。

        返回
        -------
        dict[Currency, Money] 或 ``None``

        """
        cdef Account account = self._get_account(venue, account_id, "balances locked", "'venue' or 'account_id' must be provided")

        return account.balances_locked() if account is not None else None

    cpdef dict margins_init(self, Venue venue=None, AccountId account_id=None):
        """
        返回给定场地或账户 ID 的初始（订单）保证金（如果找到）。

        参数
        ----------
        venue : Venue, 可选
            账户的场地。
        account_id : AccountId, 可选
            账户 ID（如果同时提供了 venue 和 account_id，则优先考虑此项）。

        返回
        -------
        dict[Currency, Money] 或 ``None``

        """
        cdef Account account = self._get_account(venue, account_id, "initial (order) margins", "'venue' or 'account_id' must be provided")
        if account is None or account.is_cash_account:
            return None

        return account.margins_init()

    cpdef dict margins_maint(self, Venue venue=None, AccountId account_id=None):
        """
        返回给定场地或账户 ID 的维持（持仓）保证金（如果找到）。

        参数
        ----------
        venue : Venue, 可选
            账户的场地。
        account_id : AccountId, 可选
            账户 ID（如果同时提供了 venue 和 account_id，则优先考虑此项）。

        返回
        -------
        dict[Currency, Money] 或 ``None``

        """
        cdef Account account = self._get_account(venue, account_id, "maintenance (position) margins", "'venue' or 'account_id' must be provided")
        if account is None or account.is_cash_account:
            return None

        return account.margins_maint()

    cpdef dict realized_pnls(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定场地的已实现盈亏（如果找到）。

        如果场地不存在持仓或任何内部查找失败，则返回一个空字典。

        参数
        ----------
        venue : Venue, 可选
            已实现盈亏的场地。
        account_id : AccountId, 可选
            已实现盈亏的账户 ID。
        target_currency : Currency, 可选
            转换盈亏的目标货币。

        返回
        -------
        dict[Currency, Money]

        """
        cdef list positions = self._cache.positions(
            venue=venue,
            instrument_id=None,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=account_id,
        )

        return self._aggregate_pnls_by_instrument(positions, True, account_id, target_currency)

    cpdef dict unrealized_pnls(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定场地的未实现盈亏（如果找到）。

        参数
        ----------
        venue : Venue, 可选
            未实现盈亏的场地。
        account_id : AccountId, 可选
            未实现盈亏的账户 ID。
        target_currency : Currency, 可选
            转换盈亏的目标货币。

        返回
        -------
        dict[Currency, Money]

        """
        cdef list positions_open = self._cache.positions_open(
            venue=venue,
            instrument_id=None,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=account_id,
        )

        return self._aggregate_pnls_by_instrument(positions_open, False, account_id, target_currency)

    cdef dict _aggregate_pnls_by_instrument(self, list positions, bint is_realized, AccountId account_id, Currency target_currency):
        if not positions:
            return {}  # 无需计算

        cdef set[InstrumentId] instrument_ids = {p.instrument_id for p in positions}
        cdef dict[Currency, double] aggregated_pnls = {}
        cdef:
            InstrumentId instrument_id
            Money pnl
        for instrument_id in instrument_ids:
            if is_realized:
                pnl = self.realized_pnl(instrument_id, account_id, target_currency=target_currency)
            else:
                pnl = self.unrealized_pnl(instrument_id, price=None, account_id=account_id, target_currency=target_currency)

            if pnl is None:
                continue

            aggregated_pnls[pnl.currency] = aggregated_pnls.get(pnl.currency, 0.0) + pnl.as_f64_c()

        return {k: Money(v, k) for k, v in aggregated_pnls.items()}

    cpdef dict total_pnls(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定场地的总盈亏（如果找到）。

        参数
        ----------
        venue : Venue, 可选
            总盈亏的场地。
        account_id : AccountId, 可选
            总盈亏的账户 ID。
        target_currency : Currency, 可选
            转换盈亏的目标货币。

        返回
        -------
        dict[Currency, Money]

        """
        cdef dict realized = self.realized_pnls(venue, account_id, target_currency=target_currency)
        cdef dict unrealized = self.unrealized_pnls(venue, account_id, target_currency=target_currency)

        cdef:
            double total_amount = 0.0
            dict[Currency, double] total_pnls = {}
            Currency currency
            Money amount
        if target_currency is not None:
            # 当提供了 target_currency 时，两个字典都应仅包含该货币
            # 将它们相加。如果两个字典都不包含 target_currency（例如，所有工具的转换都失败
            # 或不存在持仓），total_amount 仍为 0.0，这是正确的。
            if target_currency in realized:
                total_amount += realized[target_currency].as_double()

            if target_currency in unrealized:
                total_amount += unrealized[target_currency].as_double()

            # 返回带有 target_currency 的字典（如果未找到盈亏，则为 0.0，代表零总盈亏）
            return {target_currency: Money(total_amount, target_currency)}

        # 无 target_currency：按货币汇总
        # 汇总已实现盈亏
        for currency, amount in realized.items():
            total_pnls[currency] = amount.as_double()

        # 添加未实现盈亏
        for currency, amount in unrealized.items():
            total_pnls[currency] = total_pnls.get(currency, 0.0) + amount.as_double()

        return {k: Money(v, k) for k, v in total_pnls.items()}

    cpdef dict net_exposures(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定场地的净风险敞口（如果找到）。

        参数
        ----------
        venue : Venue, 可选
            市场价值的场地。
        account_id : AccountId, 可选
            净风险敞口的账户 ID。
        target_currency : Currency, 可选
            转换风险敞口的目标货币。

        返回
        -------
        dict[Currency, Money] 或 ``None``

        """
        cdef list positions_open = self._cache.positions_open(
            venue=venue,
            instrument_id=None,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=account_id,
        )

        cdef Account account = None
        if not positions_open:
            # 当没有持仓时，尝试确定账户是否存在
            # （账户可以从 account_id 参数或场地确定）
            account = self._cache.account_for_venue(venue, account_id)
            if account is not None:
                # 账户存在但无持仓，返回空字典
                return {}
            else:
                # 无法确定账户，返回 None
                return None

        cdef set[AccountId] involved_accounts = set()
        cdef Position pos
        for pos in positions_open:
            if not pos.is_closed_c():
                involved_accounts.add(pos.account_id)

        # 防止分属不同基础货币账户的货币静默混合
        cdef:
            set[Currency] base_currencies = set()
            AccountId involved_account_id
            Account involved_account
            list[str] currency_strs
            Currency currency
            str currencies_str
        if target_currency is None and account_id is None and len(involved_accounts) > 1:
            for involved_account_id in involved_accounts:
                involved_account = self._cache.account(involved_account_id)
                if involved_account is not None and involved_account.base_currency is not None:
                    base_currencies.add(involved_account.base_currency)

            if len(base_currencies) > 1:
                currency_strs = []
                for currency in base_currencies:
                    currency_strs.append(str(currency))

                currencies_str = ", ".join(currency_strs)
                self._log.error(
                    f"Cannot calculate net exposures: multiple accounts with different base currencies "
                    f"({currencies_str}). "
                    f"Provide an explicit target_currency to aggregate across accounts."
                )
                return None

        cdef:
            dict[Currency, double] net_exposures = {}
            Position position
            Money exposure
            set[InstrumentId] processed_instruments = set()
        for position in positions_open:
            # 跳过已平仓持仓（它们不应贡献净敞口）
            if position.is_closed_c():
                continue

            if position.instrument_id in processed_instruments:
                continue

            processed_instruments.add(position.instrument_id)

            account = self._cache.account(position.account_id)
            current_target = target_currency or (account.base_currency if account else None)

            exposure = self.net_exposure(
                position.instrument_id,
                price=None,
                account_id=account_id,
                target_currency=current_target,
            )
            if exposure is None or exposure.as_f64_c() == 0.0:
                continue

            net_exposures[exposure.currency] = net_exposures.get(exposure.currency, 0.0) + exposure.as_f64_c()

        if not net_exposures:
            return {}

        # 如果提供了 target_currency，所有风险敞口都应使用该货币
        if target_currency is not None:
            if target_currency in net_exposures:
                return {target_currency: Money(net_exposures[target_currency], target_currency)}
            else:
                # 如果转换成功，这不应该发生，但会优雅处理
                return {}

        return {k: Money(v, k) for k, v in net_exposures.items()}

    cpdef Money realized_pnl(self, InstrumentId instrument_id, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定工具 ID 的已实现盈亏（如果找到）。

        参数
        ----------
        instrument_id : InstrumentId
            盈亏所属的工具。
        account_id : AccountId, 可选
            已实现盈亏的账户 ID。如果为 None，则汇总所有账户。
        target_currency : Currency, 可选
            转换盈亏的目标货币。

        返回
        -------
        Money 或 ``None``

        """
        Condition.not_none(instrument_id, "instrument_id")

        if account_id is not None:
            # 单个账户：检查缓存并在需要时进行计算
            native_pnl = self._calculate_realized_pnl(instrument_id, account_id)
            if native_pnl is None:
                return None

            return self._convert_money_if_needed(native_pnl, target_currency, venue=instrument_id.venue)
        else:
            # 尽可能使用缓存汇总所有账户
            return self._aggregate_pnl_from_cache(instrument_id, is_realized=True, target_currency=target_currency)

    cpdef Money unrealized_pnl(self, InstrumentId instrument_id, Price price=None, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定工具 ID 的未实现盈亏（如果找到）。

        - 如果提供了 `price`，则执行全新计算，而不使用或更新缓存。
        - 如果省略 `price`，则该方法返回缓存的盈亏（如果有），如果没有则计算并缓存盈亏。

        如果计算失败（例如找不到账户或工具），则返回 `None`；如果没有持仓，则返回零值的 `Money`。
        否则，它返回一个 `Money` 对象（通常以账户的基础货币或工具的结算货币表示）。

        参数
        ----------
        instrument_id : InstrumentId
            未实现盈亏所属的工具。
        price : Price, 可选
            计算的参考价格。这可以是最新价、中间价、买价、卖价、标记价格或任何其他具有代表性的值。
        account_id : AccountId, 可选
            未实现盈亏的账户 ID。如果为 None，则汇总所有账户。
        target_currency : Currency, 可选
            转换盈亏的目标货币。

        返回
        -------
        Money 或 ``None``
            未实现盈亏，如果无法计算则为 None。

        """
        Condition.not_none(instrument_id, "instrument_id")

        if price is not None or account_id is not None:
            # 新鲜计算或单个账户
            native_pnl = self._calculate_unrealized_pnl(instrument_id, price, account_id)
            if native_pnl is None:
                return None

            return self._convert_money_if_needed(native_pnl, target_currency, venue=instrument_id.venue)
        else:
            # 尽可能使用缓存汇总所有账户
            return self._aggregate_pnl_from_cache(instrument_id, is_realized=False, target_currency=target_currency)

    cpdef Money total_pnl(self, InstrumentId instrument_id, Price price=None, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定工具 ID 的总盈亏（如果找到）。

        参数
        ----------
        instrument_id : InstrumentId
            总盈亏所属的工具。
        price : Price, 可选
            计算的参考价格。这可以是最新价、中间价、买价、卖价、标记价格或任何其他具有代表性的值。
        account_id : AccountId, 可选
            总盈亏的账户 ID。
        target_currency : Currency, 可选
            转换盈亏的目标货币。

        返回
        -------
        Money 或 ``None``

        """
        Condition.not_none(instrument_id, "instrument_id")

        cdef Money realized = self.realized_pnl(instrument_id, account_id, target_currency=target_currency)
        cdef Money unrealized = self.unrealized_pnl(instrument_id, price, account_id, target_currency=target_currency)
        if realized is None and unrealized is None:
            return None

        if realized is None:
            return unrealized

        if unrealized is None:
            return realized

        # 当提供 target_currency 时，两者应使用相同的货币
        # 如果未提供，由于 convert_to_account_base_currency 逻辑，它们仍应匹配
        cdef:
            Money converted_realized
            Money converted_unrealized
        if realized.currency != unrealized.currency:
            # target_currency 存在时这种情况不应发生，但会优雅处理
            if target_currency is not None:
                # 尝试将两者都转换为 target_currency
                converted_realized = self._convert_money(realized, target_currency, venue=instrument_id.venue)
                converted_unrealized = self._convert_money(unrealized, target_currency, venue=instrument_id.venue)
                if converted_realized is not None and converted_unrealized is not None:
                    return Money.from_raw_c(converted_realized._mem.raw + converted_unrealized._mem.raw, target_currency)

                return None

            # 不带 target_currency 时，这是货币不匹配错误
            self._log.warning(
                f"total_pnl 中的货币不匹配：{realized.currency} 与 {unrealized.currency}。 "
                f"提供 target_currency 以将两者转换为通用货币。"
            )
            return None

        return Money.from_raw_c(realized._mem.raw + unrealized._mem.raw, realized.currency)

    cpdef Money net_exposure(self, InstrumentId instrument_id, Price price=None, AccountId account_id=None, Currency target_currency=None):
        """
        返回给定工具的净风险敞口（如果找到）。

        参数
        ----------
        instrument_id : InstrumentId
            计算所属的工具。
        price : Price, 可选
            计算的参考价格。这可以是最新价、中间价、买价、卖价、标记价格或任何其他具有代表性的值。
        account_id : AccountId, 可选
            净风险敞口的账户 ID。
        target_currency : Currency, 可选
            转换敞口的目标货币。

        返回
        -------
        Money 或 ``None``

        """
        cdef list positions = self._cache.positions_open(
            venue=None,
            instrument_id=instrument_id,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=account_id,
        )

        # 汇总时验证各账户的基础货币是否一致
        # （仅在未提供 target_currency 时需要，因为我们可以转换为 target_currency）
        cdef:
            set[AccountId] account_ids
            Currency first_base_currency = None
            Account account
            AccountId acc_id
            Position position

        if account_id is None and target_currency is None and positions:
            account_ids = set()

            # 从持仓中收集唯一账户 ID
            for position in positions:
                if position.account_id is not None:
                    account_ids.add(position.account_id)

            # 从缓存验证账户
            for acc_id in account_ids:
                account = self._cache.account(acc_id)
                if account is not None:
                    if first_base_currency is None:
                        first_base_currency = account.base_currency
                    elif account.base_currency is not None and account.base_currency != first_base_currency:
                        self._log.error(
                            f"无法计算净风险敞口： "
                            f"账户具有不同的基础货币 "
                            f"({first_base_currency} 与 {account.base_currency})； "
                            f"多账户汇总需要一致的基础货币",
                        )
                        return None

        cdef Instrument instrument_obj = self._cache.instrument(instrument_id)
        if instrument_obj is None:
            return None

        cdef:
            Currency settlement_currency = instrument_obj.get_cost_currency()
            bint is_betting = isinstance(instrument_obj, BettingInstrument)
            bint used_cross_notional = False
            double total_notional
            PriceType price_type
        if is_betting:
            total_notional = self._handle_betting_instrument_exposure(positions, instrument_obj, target_currency, settlement_currency)
            if total_notional is None:
                return None

            price_type = PriceType.MARK
        else:
            total_notional, price_type, used_cross_notional = self._calculate_non_betting_exposure(
                positions=positions,
                instrument_id=instrument_id,
                instrument_obj=instrument_obj,
                price=price,
                target_currency=target_currency,
            )
            if total_notional is None:
                return None

        # 最终完成具有货币转换的风险敞口结果
        if not is_betting:
            total_notional = abs(total_notional)

        # 如果我们使用了 cross_notional_value，结果已经是 target_currency
        if used_cross_notional and target_currency is not None:
            return Money(total_notional, target_currency)

        cdef Money exposure = Money(total_notional, settlement_currency)

        # 零敞口提前返回（始终可转换）
        if target_currency is not None and total_notional == 0.0:
            return Money(0.0, target_currency)

        return self._convert_money_if_needed(
            exposure,
            target_currency,
            venue=instrument_id.venue,
            price_type=price_type,
        )

    cdef double _handle_betting_instrument_exposure(
        self,
        list positions,
        Instrument instrument_obj,
        Currency target_currency,
        Currency settlement_currency,
    ):
        # 处理博彩工具的风险敞口计算
        cdef:
            double total_notional = 0.0
            Position position
            object bet_position

        if not positions:
            # 无持仓 - 返回零敞口
            # （已平仓持仓不应贡献净敞口）
            return 0.0

        # 汇总所有未平仓持仓的博彩持仓敞口
        for position in positions:
            bet_position = self._get_bet_position(position, instrument_obj)
            if bet_position is not None:
                # 直接使用敞口（已具有正确正负号）
                total_notional += float(bet_position.exposure)

        return total_notional

    cdef tuple _calculate_non_betting_exposure(
        self,
        list positions,
        InstrumentId instrument_id,
        Instrument instrument_obj,
        Price price,
        Currency target_currency,
    ):
        # Calculate exposure for non-betting instruments
        cdef:
            double total_notional = 0.0
            PriceType price_type = PriceType.MARK  # Default for conversion
            Position position
            double val
            bint used_cross_notional = False

        if not positions:
            if instrument_obj is not None:
                return (0.0, price_type, False)

            return (None, price_type, False)

        cdef bint is_currency_pair = isinstance(instrument_obj, CurrencyPair)
        cdef bint has_long = False
        cdef bint has_short = False

        for position in positions:
            val, used_cross = self._calculate_position_exposure_value(
                position=position,
                instrument_obj=instrument_obj,
                instrument_id=instrument_id,
                price=price,
                target_currency=target_currency,
                is_currency_pair=is_currency_pair,
                positions=positions,
            )
            if val is None:
                return (None, price_type, False)

            if used_cross:
                used_cross_notional = True

            if position.side == PositionSide.LONG:
                has_long = True
                total_notional += val

                if not price:  # 仅当我们使用了 _get_price 时覆盖
                    price_type = PriceType.BID
            elif position.side == PositionSide.SHORT:
                has_short = True
                total_notional -= val

                if not price:  # 仅当我们使用了 _get_price 时覆盖
                    price_type = PriceType.ASK

        # 当多空持仓混合时使用中性定价
        if has_long and has_short and not price:
            price_type = PriceType.MARK if self._use_mark_xrates else PriceType.MID

        return (total_notional, price_type, used_cross_notional)

    cdef tuple _calculate_position_exposure_value(
        self,
        Position position,
        Instrument instrument_obj,
        InstrumentId instrument_id,
        Price price,
        Currency target_currency,
        bint is_currency_pair,
        list positions,
    ):
        # 计算单个持仓的风险敞口价值
        cdef:
            Price p = price or self._get_price(position)
            object val_result
            double val
            Money exposure_money
            bint used_cross = False

        if p is None:
            self._log.debug(f"Cannot calculate net exposure: no price for {position.instrument_id}")
            return (None, False)

        # 对于带有 target_currency 的 CurrencyPair，使用 cross_notional_value 进行精确转换
        if is_currency_pair and target_currency is not None:
            val_result = self._calculate_currency_pair_exposure(
                position=position,
                instrument_obj=instrument_obj,
                instrument_id=instrument_id,
                price=p,
                target_currency=target_currency,
                positions=positions,
                price_param=price,
            )
            if val_result is not None:
                val = <double>val_result
                used_cross = True
            else:
                # 如果缺少所需的汇率，则回退到标准转换
                exposure_money = position.notional_value(p)
                val = exposure_money.as_f64_c()
        else:
            # 标准路径：获取名义价值并在需要时进行转换
            exposure_money = position.notional_value(p)
            val = exposure_money.as_f64_c()

        return (val, used_cross)

    cdef object _calculate_currency_pair_exposure(
        self,
        Position position,
        Instrument instrument_obj,
        InstrumentId instrument_id,
        Price price,
        Currency target_currency,
        list positions,
        Price price_param,
    ):
        # 尽可能使用 cross_notional_value 计算货币对的风险敞口
        cdef:
            PriceType conv_price_type = PriceType.MID
            CurrencyPair currency_pair = <CurrencyPair>instrument_obj
            object quote_xrate = None
            object base_xrate = None
            bint can_use_cross = False
            Money exposure_money

        # 确定转换查询的价格类型
        if not price_param:  # 仅当我们使用了 _get_price 时覆盖
            if position.side == PositionSide.LONG:
                conv_price_type = PriceType.BID
            elif position.side == PositionSide.SHORT:
                conv_price_type = PriceType.ASK

        # 如果持仓混合，MARK 可能更适合转换
        if len(positions) > 1 and not price_param:
            conv_price_type = PriceType.MARK if self._use_mark_prices else conv_price_type

        # 如果启用，先尝试标记汇率
        if self._use_mark_xrates:
            quote_xrate = self._cache.get_mark_xrate(currency_pair.quote_currency, target_currency)
            base_xrate = self._cache.get_mark_xrate(currency_pair.base_currency, target_currency)

        # 回退到标准汇率查询
        if quote_xrate is None:
            quote_xrate = self._cache.get_xrate(
                venue=instrument_id.venue,
                from_currency=currency_pair.quote_currency,
                to_currency=target_currency,
                price_type=conv_price_type,
            )

        if base_xrate is None:
            base_xrate = self._cache.get_xrate(
                venue=instrument_id.venue,
                from_currency=currency_pair.base_currency,
                to_currency=target_currency,
                price_type=conv_price_type,
            )

        # 对于非反向货币对，我们只需要 quote_price（base_price 被忽略）
        # 对于反向货币对，我们只需要 base_price（quote_price 被忽略）
        # 如果我们有所需的汇率，则使用 cross_notional_value
        if not position.is_inverse:
            # 非反向：需要 quote_price
            if quote_xrate is not None and quote_xrate > 0.0:
                can_use_cross = True

                # 使用哑元 base_price，因为它不会被使用
                if base_xrate is None or base_xrate <= 0.0:
                    base_xrate = 1.0
        else:
            # 反向：需要 base_price
            if base_xrate is not None and base_xrate > 0.0:
                can_use_cross = True

                # 使用哑元 quote_price，因为它不会被使用
                if quote_xrate is None or quote_xrate <= 0.0:
                    quote_xrate = 1.0

        if can_use_cross:
            # 使用 cross_notional_value 进行精确的汇率转换
            exposure_money = position.cross_notional_value(
                price=price,
                quote_price=Price(<double>quote_xrate, FIXED_PRECISION),
                base_price=Price(<double>base_xrate, FIXED_PRECISION),
                target_currency=target_currency,
            )
            return exposure_money.as_f64_c()

        return None  # Cannot use cross_notional, fall back to standard

    cpdef object net_position(self, InstrumentId instrument_id, AccountId account_id=None):
        """
        返回给定工具 ID 的净头寸。
        如果提供了 account_id，则返回该账户的净头寸。
        如果 account_id 为 None，则汇总所有账户。
        如果 instrument_id 没有持仓，则返回 `Decimal('0')`。

        参数
        ----------
        instrument_id : InstrumentId
            查询的工具。
        account_id : AccountId, 可选
            账户 ID。如果为 None，则汇总所有账户。

        返回
        -------
        Decimal

        """
        Condition.not_none(instrument_id, "instrument_id")

        return self._net_position(instrument_id, account_id)

    cpdef bint is_net_long(self, InstrumentId instrument_id, AccountId account_id=None):
        """
        返回一个值，指示投资组合是否净做多给定的工具 ID。

        参数
        ----------
        instrument_id : InstrumentId
            查询的工具。
        account_id : AccountId, 可选
            账户 ID。如果为 None，则汇总所有账户。

        返回
        -------
        bool
            如果净做多则为 True，否则为 False。

        """
        Condition.not_none(instrument_id, "instrument_id")

        return self._net_position(instrument_id, account_id) > 0.0

    cpdef bint is_net_short(self, InstrumentId instrument_id, AccountId account_id=None):
        """
        返回一个值，指示投资组合是否净做空给定的工具 ID。

        参数
        ----------
        instrument_id : InstrumentId
            查询的工具。
        account_id : AccountId, 可选
            账户 ID。如果为 None，则汇总所有账户。

        返回
        -------
        bool
            如果净做空则为 True，否则为 False。

        """
        Condition.not_none(instrument_id, "instrument_id")

        return self._net_position(instrument_id, account_id) < 0.0

    cpdef bint is_flat(self, InstrumentId instrument_id, AccountId account_id=None):
        """
        返回一个值，指示投资组合对于给定的工具 ID 是否为空仓（持平）。

        参数
        ----------
        instrument_id : InstrumentId
            工具查询过滤器。
        account_id : AccountId, 可选
            账户 ID。如果为 None，则汇总所有账户。

        返回
        -------
        bool
            如果净持平则为 True，否则为 False。

        """
        Condition.not_none(instrument_id, "instrument_id")

        return self._net_position(instrument_id, account_id) == 0.0

    cdef object _net_position(self, InstrumentId instrument_id, AccountId account_id=None):
        # 获取工具和账户的净头寸。如果 account_id 为 None，则汇总所有账户。
        cdef dict account_positions = self._net_positions.get(instrument_id)
        if account_positions is None:
            return Decimal(0)

        if account_id is not None:
            return account_positions.get(account_id, Decimal(0))

        # 汇总所有账户
        return sum(account_positions.values(), Decimal(0))

    cpdef bint is_completely_flat(self, AccountId account_id=None):
        """
        返回一个值，指示投资组合是否完全平仓。

        参数
        ----------
        account_id : AccountId, 可选
            账户 ID。如果为 None，则检查所有账户。

        返回
        -------
        bool
            如果所有工具都已结清则为 True，否则为 False。

        """
        cdef:
            InstrumentId instrument_id
            dict account_dict
            AccountId acc_id
            object net_position
        for instrument_id, account_dict in self._net_positions.items():
            for acc_id, net_position in account_dict.items():
                # 如果提供了 account_id，则按其过滤
                if account_id is not None and acc_id != account_id:
                    continue

                if net_position != Decimal(0):
                    return False

        return True

# -- INTERNAL -------------------------------------------------------------------------------------

    cdef tuple _validate_event_account_and_instrument(self, object event, str caller_name):
        if event.account_id is None:
            return None, None

        cdef Account account = self._cache.account(event.account_id)
        if account is None:
            self._log.error(
                f"无法更新 {caller_name}： "
                f"没有为 {event.account_id} 注册账户",
            )
            return None, None

        cdef Instrument instrument = self._cache.instrument(event.instrument_id)
        if instrument is None:
            self._log.error(
                f"无法更新 {caller_name}： "
                f"未找到 {event.instrument_id} 的工具",
            )
            return None, None

        return account, instrument

    cdef void _update_account(self, AccountState event):
        cdef Account account = self._cache.account(event.account_id)
        if account is None:
            # 生成账户
            account = AccountFactory.create_c(event)
            self._cache.add_account(account)
        else:
            account.apply(event)
            self._cache.update_account(account)

        cdef:
            bint should_log = True
            uint64_t ts_last_logged
        if self._min_account_state_logging_interval_ns:
            ts_last_logged = self._last_account_state_log_ts.get(event.account_id, 0)
            if (not ts_last_logged) or (event.ts_init - ts_last_logged) >= self._min_account_state_logging_interval_ns:
                self._last_account_state_log_ts[event.account_id] = event.ts_init
            else:
                should_log = False

        if should_log:
            self._log.info(f"已更新 {event}")

    cdef Account _get_account(self, Venue venue, AccountId account_id, str caller_name, str message=None):
        Condition.not_none(venue or account_id, message or "必须提供 'venue' 或 'account_id'")

        cdef Account account = self._cache.account_for_venue(venue, account_id)
        if account is None:
            self._log.error(
                f"无法获取 {caller_name}： "
                f"没有为 {venue=} 和 {account_id=} 注册账户",
            )

        return account

    cdef void _update_net_position(self, InstrumentId instrument_id, list positions_open):
        # 更新给定工具按账户划分的净头寸。
        cdef:
            dict[AccountId, Decimal] net_positions_by_account = {}
            Position position
            AccountId account_id
            object net_position  # Decimal
            set[AccountId] accounts_with_positions = set()
            list accounts_to_remove

        # 计算每个账户的净头寸
        for position in positions_open:
            account_id = position.account_id
            accounts_with_positions.add(account_id)
            net_positions_by_account[account_id] = (
                net_positions_by_account.get(account_id, Decimal(0)) + position.signed_decimal_qty()
            )

        # 确保存在工具项
        self._net_positions.setdefault(instrument_id, {})

        # 为每个有持仓的账户更新缓存
        for account_id, net_position in net_positions_by_account.items():
            existing_position = self._net_positions[instrument_id].get(account_id, Decimal(0))
            if existing_position != net_position:
                self._net_positions[instrument_id][account_id] = net_position
                self._log.info(f"{instrument_id} account={account_id} net_position={net_position}")

        # 清除不再有未平仓持仓的账户的缓存项
        if instrument_id in self._net_positions:
            accounts_to_remove = []
            for acc_id in self._net_positions[instrument_id].keys():
                if acc_id not in accounts_with_positions:
                    accounts_to_remove.append(acc_id)

            for acc_id in accounts_to_remove:
                self._net_positions[instrument_id].pop(acc_id, None)

            # 如果为空，则删除 instrument_id 项
            if not self._net_positions[instrument_id]:
                self._net_positions.pop(instrument_id, None)

    cdef void _update_instrument_id(self, InstrumentId instrument_id):
        # 失效此工具的缓存盈亏（所有账户）
        self._unrealized_pnls.pop(instrument_id, None)

        if self.initialized:
            return

        if instrument_id not in self._pending_calcs:
            return

        cdef list orders_open = self._cache.orders_open(
            venue=None,  # 更快的查询过滤
            instrument_id=instrument_id,
        )
        cdef dict orders_by_account = self._group_by_account_id(orders_open)

        cdef:
            AccountId account_id
            list account_orders
            Account account
            Instrument instrument
            Order o
            bint result_init
            bint result_maint
            list positions_open
            Money result_unrealized_pnl
            bint account_initialized
            list accounts_initialized = []
        for account_id, account_orders in orders_by_account.items():
            account = self._cache.account(account_id)
            if account is None:
                self._log.error(
                    f"无法更新：未找到为 {account_id=} 注册的账户",
                )
                return  # 未注册账户

            instrument = self._cache.instrument(instrument_id)
            if instrument is None:
                self._log.error(
                    f"无法更新：未找到 {instrument_id} 的工具",
                )
                return  # 未找到工具

            # 初始化初始（订单）保证金
            result_init = self._accounts.update_orders(
                account=account,
                instrument=instrument,
                orders_open=[o for o in account_orders if o.is_passive_c()],
                ts_event=account.last_event_c().ts_event,
            )

            result_maint = False
            if account.is_margin_account:
                positions_open = self._cache.positions_open(
                    venue=None,  # 更快的查询过滤
                    instrument_id=instrument_id,
                    strategy_id=None,
                    side=PositionSide.NO_POSITION_SIDE,
                    account_id=account_id,
                )

                # 初始化维持（持仓）保证金
                result_maint = self._accounts.update_positions(
                    account=account,
                    instrument=instrument,
                    positions_open=positions_open,
                    ts_event=account.last_event_c().ts_event,
                )

            # 计算未实现盈亏
            result_unrealized_pnl = self._calculate_unrealized_pnl(
                instrument_id=instrument_id,
                price=None,
                account_id=account_id
            )

            # 检查投资组合初始化
            account_initialized = result_init and (account.is_cash_account or (result_maint and result_unrealized_pnl is not None))
            accounts_initialized.append(account_initialized)

        if all(accounts_initialized):
            self._pending_calcs.discard(instrument_id)
            if not self._pending_calcs:
                self.initialized = True

    cdef dict _group_by_account_id(self, list items):
        # 注意：可以是在 rust 中的通用函数
        cdef:
            dict result = {}
            object item  # Order or Position
            AccountId account_id
        for item in items:
            account_id = item.account_id
            if account_id is not None:
                result.setdefault(account_id, []).append(item)

        return result

    cdef Money _aggregate_pnl_from_cache(self, InstrumentId instrument_id, bint is_realized, Currency target_currency=None):
        # 从缓存汇总给定工具在所有账户中的盈亏。
        # 如果缓存为空，则计算所有持仓账户的盈亏。
        cdef:
            dict pnl_cache = self._realized_pnls if is_realized else self._unrealized_pnls
            str pnl_type = "realized" if is_realized else "unrealized"
            Money total_pnl = None
            Money pnl
            dict account_pnls

        # 对于已实现盈亏，确保已处理快照（这也会在需要时使盈亏缓存失效）
        if is_realized:
            self._ensure_snapshot_pnls_cached_for(instrument_id)

        account_pnls = pnl_cache.get(instrument_id)

        # 如果此工具在缓存中没有任何内容，则计算盈亏
        if account_pnls is None or len(account_pnls) == 0:
            return self._aggregate_pnl_by_calculation(instrument_id, price=None, is_realized=is_realized, target_currency=target_currency)

        for pnl in account_pnls.values():
            if pnl is not None:
                total_pnl = self._add_pnl_to_total(total_pnl, pnl, pnl_type, venue=instrument_id.venue, target_currency=target_currency)
                if total_pnl is None:
                    return None  # 货币不匹配

        # 如果缓存有条目但 total_pnl 为 None，检查工具是否存在并返回零
        if total_pnl is None:
            return self._get_zero_or_none_for_instrument(instrument_id, target_currency=target_currency)

        return total_pnl

    cdef Money _aggregate_pnl_by_calculation(self, InstrumentId instrument_id, Price price, bint is_realized, Currency target_currency=None):
        # 通过查找所有持有头寸的账户并逐个计算来汇总盈亏。
        # 当提供价格（新鲜计算）或跨账户汇总时使用。
        cdef:
            set[AccountId] account_ids = set()
            list all_positions
            list all_positions_open
            Position position
            AccountId account_id
            Money account_pnl
            Money total_pnl = None
            Instrument inst
            set[PositionId] snapshot_ids
            PositionId position_id

        if is_realized:
            # 对于已实现盈亏，检查所有持仓（开仓和闭仓）和快照
            # 首先确保缓存了快照，以便可用 account_id
            self._ensure_snapshot_pnls_cached_for(instrument_id)

            all_positions = self._cache.positions(
                venue=None,  # 更快的查询过滤
                instrument_id=instrument_id,
                strategy_id=None,
                side=PositionSide.NO_POSITION_SIDE,
                account_id=None,
            )
            for position in all_positions:
                account_ids.add(position.account_id)

            # 同样检查快照以获取 account_id
            snapshot_ids = self._cache.position_snapshot_ids(instrument_id)
            for position_id in snapshot_ids:
                snapshot_account_id = self._snapshot_account_ids.get(position_id)
                if snapshot_account_id is not None:
                    account_ids.add(snapshot_account_id)
        else:
            # 对于未实现盈亏，仅检查开仓持仓
            all_positions_open = self._cache.positions_open(
                venue=None,  # 更快的查询过滤
                instrument_id=instrument_id,
                strategy_id=None,
                side=PositionSide.NO_POSITION_SIDE,
                account_id=None,
            )
            for position in all_positions_open:
                account_ids.add(position.account_id)

        if not account_ids:
            return self._get_zero_or_none_for_instrument(instrument_id, target_currency=target_currency)

        # 获取相应的缓存字典
        cdef dict pnl_cache = self._realized_pnls if is_realized else self._unrealized_pnls

        # 确定我们是否应该进行缓存：仅在价格为 None（使用当前市场价格）时进行缓存
        # 如果提供了价格，则是使用特定价格进行的新鲜计算，因此不进行缓存
        cdef bint should_cache = (price is None)

        # 为每个账户计算并求和
        # 始终先计算本位币以用于缓存，然后根据需要进行转换
        cdef bint attempted_calculation = False
        cdef bint any_conversion_failed = False
        cdef Money native_pnl
        for account_id in account_ids:
            # 以本位币计算以用于缓存
            if is_realized:
                native_pnl = self._calculate_realized_pnl(instrument_id, account_id)
            else:
                native_pnl = self._calculate_unrealized_pnl(instrument_id, price, account_id)

            attempted_calculation = True

            # 缓存本位币盈亏（仅当使用当前市场价，而非特定价时）
            if native_pnl is not None:
                if should_cache:
                    pnl_cache.setdefault(instrument_id, {})[account_id] = native_pnl

                # 如果需要汇总，则转换为 target_currency
                if target_currency is not None:
                    account_pnl = self._convert_money_if_needed(native_pnl, target_currency, venue=instrument_id.venue)
                    if account_pnl is None:
                        any_conversion_failed = True
                        self._log.error(
                            f"无法汇总盈亏：账户 {account_id} 从 {native_pnl.currency} 到 {target_currency} 的转换失败"
                        )
                else:
                    account_pnl = native_pnl

                if account_pnl is not None:
                    total_pnl = self._add_pnl_to_total(total_pnl, account_pnl, "unrealized" if not is_realized else "realized", venue=instrument_id.venue, target_currency=target_currency)

        # 如果任何转换失败，则返回 None（防止部分总额）
        if any_conversion_failed:
            return None

        if total_pnl is None:
            # 如果我们尝试了计算但全部返回 None（例如转换失败），且提供了 target_currency，则返回 None 以指示转换失败。
            # 否则，返回零（无持仓或无盈亏）。
            if attempted_calculation and target_currency is not None:
                return None

            return self._get_zero_or_none_for_instrument(instrument_id, target_currency=target_currency)

        return total_pnl

    cdef Money _add_pnl_to_total(self, Money total_pnl, Money pnl, str pnl_type, Venue venue=None, Currency target_currency=None):
        # 将盈亏添加到运行总额中，处理货币不匹配。
        # 返回新的总额，如果发生货币不匹配则返回 None。
        if total_pnl is None:
            return self._convert_money_if_needed(pnl, target_currency, venue=venue)
        elif total_pnl.currency == pnl.currency:
            return Money(total_pnl.as_double() + pnl.as_double(), total_pnl.currency)
        else:
            if target_currency is not None:
                # 如果盈亏已经转换，这种情况不应该发生，但预防万一
                if pnl.currency != target_currency:
                    pnl = self._convert_money(pnl, target_currency, venue=venue)

                if pnl is None:
                    return total_pnl

                return Money(total_pnl.as_double() + pnl.as_double(), target_currency)

            # 货币不匹配 - 需要转换，但目前返回 None
            self._log.warning(
                f"在汇总的 {pnl_type} 盈亏中存在货币不匹配： "
                f"{total_pnl.currency} 与 {pnl.currency}。 "
                f"改为按 account_id 计算盈亏 {pnl_type}"
            )
            return None

    cdef Money _get_zero_or_none_for_instrument(self, InstrumentId instrument_id, Currency target_currency=None):
        # 为缺失的工具返回适当的零或 None 的辅助方法
        cdef Instrument inst = self._cache.instrument(instrument_id)
        if inst is not None:
            # 如果提供了 target_currency，我们总能返回该货币的零值 Money 对象，
            # 因为 0 值与汇率无关。
            if target_currency is not None:
                return Money(0, target_currency)

            return Money(0, inst.get_cost_currency())
        else:
            self._log.warning(f"返回 None，因为在缓存中未找到工具 {instrument_id}")
            return None

    cdef Money _calculate_realized_pnl(self, InstrumentId instrument_id, AccountId account_id):
        # account_id 在这里是强制性的；汇总由 _aggregate_pnl_by_calculation 处理
        cdef:
            Account account
            Instrument instrument

        account, instrument = self._validate_account_and_instrument(instrument_id, account_id, "realized", is_error=False)
        if account is None or instrument is None:
            return None

        self._ensure_snapshot_pnls_cached_for(instrument_id)
        cdef list[Position] positions = self._cache.positions(
            venue=None,  # 更快的查询过滤
            instrument_id=instrument_id,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=account_id,
        )
        cdef Currency currency = self._determine_pnl_currency(account, instrument)

        if self._debug:
            self._log.debug(f"已找到 {instrument_id} 的 {len(positions)} 个持仓")

        cdef tuple snapshot_result = self._process_snapshot_pnl_contributions(
            instrument_id=instrument_id,
            account_id=account_id,
            positions=positions,
            currency=currency,
            account=account,
        )
        if snapshot_result is None:
            return None

        cdef:
            double total_pnl = 0.0
            set[PositionId] processed_ids

        total_pnl, processed_ids = snapshot_result

        cdef object active_pnl_result = self._process_active_position_realized_pnl(
            positions=positions,
            instrument_id=instrument_id,
            instrument=instrument,
            account=account,
            currency=currency,
            processed_ids=processed_ids,
        )
        if active_pnl_result is None:
            return None

        total_pnl += <double>active_pnl_result
        cdef Money result = Money(total_pnl, currency)

        return result

    cdef void _ensure_snapshot_pnls_cached_for(self, InstrumentId instrument_id):  # noqa: C901
        # 性能：此方法维护快照盈亏的增量缓存
        # 它仅反序列化尚未处理的新快照
        # 跟踪每个持仓的总和及最后盈亏，以实现高效的 NETTING OMS 支持

        # 获取该工具所有具有快照的持仓 ID
        cdef set[PositionId] snapshot_position_ids = self._cache.position_snapshot_ids(instrument_id)
        if not snapshot_position_ids:
            return  # 无需处理

        cdef bint rebuild = False
        cdef bint has_new_snapshots = False
        cdef bint has_purge = False

        cdef:
            PositionId position_id
            list position_id_snapshots
            int prev_count
            int curr_count
            dict[PositionId, list] snapshot_data = {}

        # 预取并检测变化
        for position_id in snapshot_position_ids:
            position_id_snapshots = self._cache.position_snapshot_bytes(position_id)
            curr_count = len(position_id_snapshots)
            snapshot_data[position_id] = position_id_snapshots

            prev_count = self._snapshot_processed_counts.get(position_id, 0)
            if prev_count > curr_count:
                rebuild = True
                has_purge = True
            elif curr_count > prev_count:
                has_new_snapshots = True

        cdef:
            Position snapshot
            Money sum_pnl
            Money last_pnl

        if rebuild:
            # 全量重建：从头处理所有快照
            for position_id in snapshot_position_ids:
                sum_pnl = None
                last_pnl = None
                position_id_snapshots = snapshot_data[position_id]
                curr_count = len(position_id_snapshots)

                if curr_count:
                    for s in position_id_snapshots:
                        snapshot = pickle.loads(s)

                        # 跟踪此持仓快照的 account_id
                        if snapshot.account_id is not None:
                            self._snapshot_account_ids[position_id] = snapshot.account_id

                        if snapshot.realized_pnl is not None:
                            if sum_pnl is None:
                                sum_pnl = snapshot.realized_pnl
                            elif sum_pnl.currency == snapshot.realized_pnl.currency:
                                # 累加所有快照盈亏
                                sum_pnl = Money(
                                    sum_pnl.as_double() + snapshot.realized_pnl.as_double(),
                                    sum_pnl.currency
                                )

                            # 始终将最后一次更新为最新快照
                            last_pnl = snapshot.realized_pnl

                # 更新跟踪结构
                if sum_pnl is not None:
                    self._snapshot_sum_per_position[position_id] = sum_pnl
                    self._snapshot_last_per_position[position_id] = last_pnl
                else:
                    self._snapshot_sum_per_position.pop(position_id, None)
                    self._snapshot_last_per_position.pop(position_id, None)

                self._snapshot_processed_counts[position_id] = curr_count
        else:
            # 增量路径：仅处理新快照
            for position_id in snapshot_position_ids:
                position_id_snapshots = snapshot_data[position_id]
                curr_count = len(position_id_snapshots)
                if curr_count == 0:
                    continue

                prev_count = self._snapshot_processed_counts.get(position_id, 0)
                if prev_count >= curr_count:
                    continue

                sum_pnl = self._snapshot_sum_per_position.get(position_id)
                last_pnl = self._snapshot_last_per_position.get(position_id)

                # 仅处理新快照
                for idx in range(prev_count, curr_count):
                    snapshot = pickle.loads(position_id_snapshots[idx])

                    # 跟踪此持仓快照的 account_id
                    if snapshot.account_id is not None:
                        self._snapshot_account_ids[position_id] = snapshot.account_id

                    if snapshot.realized_pnl is not None:
                        if sum_pnl is None:
                            sum_pnl = snapshot.realized_pnl
                        elif sum_pnl.currency == snapshot.realized_pnl.currency:
                            # 累加到运行总和
                            sum_pnl = Money(
                                sum_pnl.as_double() + snapshot.realized_pnl.as_double(),
                                sum_pnl.currency
                            )

                        # 更新最后一次为最新
                        last_pnl = snapshot.realized_pnl

                # 更新跟踪结构
                if sum_pnl is not None:
                    self._snapshot_sum_per_position[position_id] = sum_pnl
                    self._snapshot_last_per_position[position_id] = last_pnl
                else:
                    self._snapshot_sum_per_position.pop(position_id, None)
                    self._snapshot_last_per_position.pop(position_id, None)

                self._snapshot_processed_counts[position_id] = curr_count

        # 清除过时条目（不再有快照的持仓）
        cdef list[PositionId] stale_ids = []
        cdef PositionId stale_position_id
        for stale_position_id in self._snapshot_processed_counts:
            if stale_position_id not in snapshot_position_ids:
                stale_ids.append(stale_position_id)

        # 如果持仓被清除，则使盈亏缓存失效
        if stale_ids:
            has_purge = True

        for stale_position_id in stale_ids:
            self._snapshot_processed_counts.pop(stale_position_id, None)
            self._snapshot_sum_per_position.pop(stale_position_id, None)
            self._snapshot_last_per_position.pop(stale_position_id, None)
            self._snapshot_account_ids.pop(stale_position_id, None)

        # 当快照发生变化（新快照或降级/清除）时，使盈亏缓存失效
        if has_new_snapshots or has_purge:
            self._realized_pnls.pop(instrument_id, None)

    cdef tuple _process_snapshot_pnl_contributions(
        self,
        InstrumentId instrument_id,
        AccountId account_id,
        list positions,
        Currency currency,
        Account account,
    ):
        # 使用 3 种情况的组合规则处理快照盈亏贡献
        cdef:
            set[PositionId] active_position_ids = {p.id for p in positions}
            set[PositionId] snapshot_ids = self._cache.position_snapshot_ids(instrument_id)
            set[PositionId] processed_ids = set()
            double total_pnl = 0.0
            PositionId position_id
            Money sum_pnl
            Money last_pnl
            object contribution_result
            double contribution
            AccountId snapshot_account_id
            Position position
            double xrate
            PriceType conv_price_type
            Instrument instrument

        for position_id in snapshot_ids:
            # 仅处理所请求账户的快照
            snapshot_account_id = self._snapshot_account_ids.get(position_id)
            if snapshot_account_id != account_id:
                continue  # 跳过其他账户的快照

            sum_pnl = self._snapshot_sum_per_position.get(position_id)
            if sum_pnl is None:
                continue  # 该持仓无盈亏

            contribution_result = self._calculate_snapshot_contribution(
                position_id=position_id,
                active_position_ids=active_position_ids,
                positions=positions,
                sum_pnl=sum_pnl,
                processed_ids=processed_ids,
            )
            if contribution_result is None:
                continue

            contribution = <double>contribution_result

            # 根据需要进行货币转换并添加贡献
            if sum_pnl.currency == currency:
                total_pnl += contribution
            else:
                # 在快照转换中尊重 use_mark_xrates 配置
                instrument = self._cache.instrument(instrument_id)
                conv_price_type = PriceType.MARK if self._use_mark_xrates else PriceType.MID
                xrate = self._cache.get_xrate(
                    venue=instrument.id.venue,
                    from_currency=sum_pnl.currency,
                    to_currency=currency,
                    price_type=conv_price_type,
                )

                # 如果标记汇率不可用，则回退到中间价
                if xrate is None and conv_price_type == PriceType.MARK:
                    xrate = self._cache.get_xrate(
                        venue=instrument.id.venue,
                        from_currency=sum_pnl.currency,
                        to_currency=currency,
                        price_type=PriceType.MID,
                    )

                if xrate is None or xrate <= 0.0:
                    return None  # 无法转换货币

                total_pnl += contribution * xrate

        return (round(total_pnl, currency.get_precision()), processed_ids)

    cdef object _calculate_snapshot_contribution(
        self,
        PositionId position_id,
        set active_position_ids,
        list positions,
        Money sum_pnl,
        set processed_ids,
    ):
        # 使用 3 种情况组合规则计算来自快照的贡献
        cdef:
            double contribution = 0.0
            Position position
            Money last_pnl
        if position_id not in active_position_ids:
            # 情况 1: 持仓不在缓存中 - 添加所有快照的总和
            contribution = sum_pnl.as_double()

            # 因为持仓不存在，标记为已完全处理
            processed_ids.add(position_id)
        else:
            # 持仓在缓存中 - 找到它
            position = None
            for p in positions:
                if p.id == position_id:
                    position = p
                    break

            if position is None:
                return None  # 不应该发生

            if position.is_open_c():
                # 情况 2: 持仓开启 - 添加总和（先前周期）+ 持仓的已实现盈亏
                contribution = sum_pnl.as_double()

                # 持仓的盈亏将在下面的持仓循环中添加
                # 不要标记为已处理 - 我们仍然需要添加当前盈亏
            else:
                # 情况 3: 持仓关闭
                # 如果最后一个快照等于当前持仓已实现盈亏，在这里减去它；
                # 当我们在下面添加持仓已实现盈亏时，净效果是 `sum`。
                # 如果不相等（新关闭的周期尚未快照），则在这里包含完整的 `sum`
                # 并在下面添加持仓已实现盈亏（净额为 `sum + realized`）。
                last_pnl = self._snapshot_last_per_position.get(position_id)
                if (
                    last_pnl is not None
                    and position.realized_pnl is not None
                    and last_pnl.currency == position.realized_pnl.currency
                    and last_pnl == position.realized_pnl
                ):
                    contribution = sum_pnl.as_double() - last_pnl.as_double()
                else:
                    contribution = sum_pnl.as_double()

                # 持仓的盈亏将在下面的持仓循环中添加
                # 不要标记为已处理 - 我们仍然需要添加当前盈亏

        return contribution

    cdef object _process_active_position_realized_pnl(
        self,
        list positions,
        InstrumentId instrument_id,
        Instrument instrument,
        Account account,
        Currency currency,
        set processed_ids,
    ):
        # 处理来自活跃持仓的已实现盈亏
        cdef:
            double total_pnl = 0.0
            Position position
            double pnl
            object xrate_result
            double xrate
            object bet_position
        for position in positions:
            if position.instrument_id != instrument_id:
                continue  # 无需计算

            # 跳过已通过快照处理的持仓
            if position.id in processed_ids:
                continue  # 已在快照逻辑中处理

            if position.realized_pnl is None:
                continue  # 无需添加盈亏

            if self._debug:
                self._log.debug(f"正在为 {position} 添加已实现盈亏")

            # 添加持仓的已实现盈亏
            if isinstance(instrument, BettingInstrument):
                bet_position = self._get_bet_position(position, instrument)
                if bet_position is None:
                    self._log.debug(
                        f"无法计算已实现盈亏：没有 {position.id} 的 `BetPosition`",
                    )
                    return None  # 无法计算

                pnl = float(bet_position.realized_pnl)
            else:
                pnl = position.realized_pnl.as_f64_c()

            if self._convert_to_account_base_currency and account.base_currency is not None:
                xrate_result = self._get_xrate_to_account_base(
                    instrument=instrument,
                    account=account,
                    instrument_id=instrument_id,
                )
                if xrate_result is None or xrate_result == 0:
                    self._log.debug(
                        f"无法计算已实现盈亏： "
                        f"尚未有 {instrument.get_cost_currency()}/{account.base_currency} 的 {self._log_xrate} 汇率",
                    )
                    self._pending_calcs.add(instrument.id)
                    return None  # 无法计算

                xrate = <double>xrate_result
                pnl = pnl * xrate

            total_pnl += pnl

        return round(total_pnl, currency.get_precision())

    cdef Money _calculate_unrealized_pnl(self, InstrumentId instrument_id, Price price=None, AccountId account_id=None):
        # 在 _aggregate_pnl_by_calculation 中进行汇总时，account_id 可以为 None（该方法使用价格进行刷新计算）
        cdef:
            Account account
            Instrument instrument

        account, instrument = self._validate_account_and_instrument(instrument_id, account_id, "unrealized", is_error=True)
        if account is None or instrument is None:
            return None

        cdef Currency currency = self._determine_pnl_currency(account, instrument)
        cdef list positions_open = self._cache.positions_open(
            venue=None,  # 更快的查询过滤
            instrument_id=instrument_id,
            strategy_id=None,
            side=PositionSide.NO_POSITION_SIDE,
            account_id=account_id,
        )
        if not positions_open:
            return Money(0, currency)

        cdef object total_pnl = self._calculate_total_unrealized_pnl(
            positions_open=positions_open,
            instrument_id=instrument_id,
            instrument=instrument,
            account=account,
            currency=currency,
            price=price,
        )

        if total_pnl is None:
            return None

        cdef Money result = Money(<double>total_pnl, currency)

        return result

    cdef tuple _validate_account_and_instrument(self, InstrumentId instrument_id, AccountId account_id, str caller_name, bint is_error):
        cdef Account account = self._cache.account_for_venue(instrument_id.venue, account_id)
        if account is None:
            msg = f"无法计算 {caller_name} 盈亏：没有为 {instrument_id.venue} 和 {account_id} 注册账号"
            if is_error:
                self._log.error(msg)
            else:
                self._log.warning(msg)
            return None, None

        cdef Instrument instrument = self._cache.instrument(instrument_id)
        if instrument is None:
            msg = f"无法计算 {caller_name} 盈亏：找不到 {instrument_id} 的工具"
            if is_error:
                self._log.error(msg)
            else:
                self._log.warning(msg)
            return None, None

        if self._debug:
            self._log.debug(
                f"正在计算工具 {instrument_id} 在账户 {account} 中的 {caller_name} 盈亏", LogColor.MAGENTA,
            )

        return account, instrument

    cdef Currency _determine_pnl_currency(self, Account account, Instrument instrument):
        if self._convert_to_account_base_currency and account.base_currency is not None:
            return account.base_currency
        else:
            return instrument.get_cost_currency()

    cdef object _calculate_total_unrealized_pnl(
        self,
        list positions_open,
        InstrumentId instrument_id,
        Instrument instrument,
        Account account,
        Currency currency,
        Price price,
    ):
        # 计算所有未平仓持仓的总未实现盈亏
        cdef:
            double total_pnl = 0.0
            Position position
            object pnl_result
            double pnl
            object xrate_result
            double xrate
        for position in positions_open:
            if position.instrument_id != instrument_id:
                continue  # 无需计算

            if position.side == PositionSide.FLAT:
                continue  # 无需计算

            pnl_result = self._calculate_position_unrealized_pnl(
                position=position,
                instrument=instrument,
                account=account,
                currency=currency,
                instrument_id=instrument_id,
                price=price,
            )
            if pnl_result is None:
                return None  # 无法计算

            pnl = <double>pnl_result
            total_pnl += pnl

        return round(total_pnl, currency.get_precision())

    cdef object _calculate_position_unrealized_pnl(
        self,
        Position position,
        Instrument instrument,
        Account account,
        Currency currency,
        InstrumentId instrument_id,
        Price price,
    ):
        # 计算单个持仓的未实现盈亏
        cdef:
            Price p
            double pnl
            object bet_position
            object xrate_result
            double xrate

        p = price or self._get_price(position)
        if p is None:
            self._log.debug(
                f"无法计算未实现盈亏：没有 {instrument_id} 的 {self._log_price}",
            )
            self._pending_calcs.add(instrument.id)
            return None  # 无法计算

        if self._debug:
            self._log.debug(f"正在计算 {position} 的未实现盈亏")

        if isinstance(instrument, BettingInstrument):
            bet_position = self._get_bet_position(position, instrument)
            if bet_position is None:
                self._log.debug(
                    f"无法计算未实现盈亏：没有为 {position.id} 找到 `BetPosition`",
                )
                return None  # 无法计算

            pnl = float(bet_position.unrealized_pnl(p.as_decimal()))
        else:
            pnl = position.unrealized_pnl(p).as_f64_c()

        if self._debug:
            self._log.debug(
                f"{instrument.id} 的未实现盈亏：{pnl} {currency}", LogColor.MAGENTA,
            )

        if self._convert_to_account_base_currency and account.base_currency is not None:
            xrate_result = self._get_xrate_to_account_base(
                instrument=instrument,
                account=account,
                instrument_id=instrument_id,
            )
            if xrate_result is None or xrate_result == 0:
                self._log.debug(
                    f"无法计算未实现盈亏： "
                    f"没有 {instrument.get_cost_currency()}/{account.base_currency} 的 {self._log_xrate} 汇率",
                )
                self._pending_calcs.add(instrument.id)
                return None  # 无法计算

            xrate = <double>xrate_result
            pnl = pnl * xrate

        return pnl

    cdef object _get_bet_position(self, Position position, Instrument instrument):
        # 获取博彩持仓的辅助方法，对于净额结算持仓可回退至工具 ID
        cdef object bet_position = self._bet_positions.get(position.id)
        if bet_position is None:
            # 对于净额结算持仓尝试回退至工具 ID
            bet_position = self._bet_positions.get(PositionId(instrument.id.value))

        return bet_position

    cdef object _get_xrate_to_account_base(
        self,
        Instrument instrument,
        Account account,
        InstrumentId instrument_id,
    ):
        # 获取从工具结算货币到账户基础货币的汇率。
        # 如果启用，则使用标记汇率，否则回退至中间价汇率。
        if account.base_currency is None:
            return None

        cdef PriceType price_type = PriceType.MARK if self._use_mark_xrates else PriceType.MID
        # 使用工具的场地进行汇率查询，而不是账户所在的场地
        cdef Venue venue = instrument_id.venue

        cdef object xrate = self._cache.get_xrate(
            venue=venue,
            from_currency=instrument.get_cost_currency(),
            to_currency=account.base_currency,
            price_type=price_type,
        )

        # 如果标记价格不可用，则回退到中间价
        if xrate is None and price_type == PriceType.MARK:
            xrate = self._cache.get_xrate(
                venue=venue,
                from_currency=instrument.get_cost_currency(),
                to_currency=account.base_currency,
                price_type=PriceType.MID,
            )

        return xrate

    cdef Price _get_price(self, Position position):
        cdef PriceType price_type
        if self._use_mark_prices:
            price_type = PriceType.MARK
        elif position.side == PositionSide.FLAT:
            price_type = PriceType.LAST
        elif position.side == PositionSide.LONG:
            price_type = PriceType.BID
        elif position.side == PositionSide.SHORT:
            price_type = PriceType.ASK
        else:  # pragma: no cover (design-time error)
            raise RuntimeError(
                f"invalid `PositionSide`, was {position_side_to_str(position.side)}",
            )

        cdef InstrumentId instrument_id = position.instrument_id

        return self._cache.price(
            instrument_id=instrument_id,
            price_type=price_type,
        ) or self._cache.price(
            instrument_id=instrument_id,
            price_type=PriceType.LAST,
        ) or self._bar_close_prices.get(instrument_id)

    cdef Money _convert_money_if_needed(
        self,
        Money money,
        Currency target_currency,
        Venue venue=None,
        PriceType price_type=PriceType.MID,
    ):
        # 如果提供了 target_currency 且与金额的货币不同，则转换金额的辅助方法
        if target_currency is not None and money.currency != target_currency:
            return self._convert_money(money, target_currency, venue=venue, price_type=price_type)

        return money

    cdef Money _convert_money(
            self,
            Money money,
            Currency target_currency,
            Venue venue=None,
            PriceType price_type=PriceType.MID,
    ):
        if money.currency == target_currency:
            return money

        # 如果启用且 price_type 未明确设置为其他值，则使用标记汇率
        cdef PriceType effective_price_type = price_type
        if self._use_mark_xrates and price_type == PriceType.MID:
            effective_price_type = PriceType.MARK

        cdef object xrate = self._cache.get_xrate(
            venue=venue,
            from_currency=money.currency,
            to_currency=target_currency,
            price_type=effective_price_type,
        )

        if xrate is None and effective_price_type == PriceType.MARK:
            # 回退到标准汇率查询（如果提供，则使用场地）
            xrate = self._cache.get_xrate(
                venue=venue,
                from_currency=money.currency,
                to_currency=target_currency,
                price_type=PriceType.MID,
            )

        if xrate is None or xrate <= 0.0:
            self._log.error(f"无法将 {money} 转换为 {target_currency}：{'无' if xrate is None else '无效'} 汇率（使用 {price_type_to_str(effective_price_type)} 转换 {money.currency} 时）")
            return None

        return Money(round(money.as_f64_c() * (<double>xrate), target_currency.get_precision()), target_currency)
