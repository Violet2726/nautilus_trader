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

from decimal import Decimal

import pandas as pd

from nautilus_trader.risk.config import RiskEngineConfig

from libc.stdint cimport uint64_t

from nautilus_trader.accounting.accounts.base cimport Account
from nautilus_trader.accounting.accounts.cash cimport CashAccount
from nautilus_trader.cache.cache cimport Cache
from nautilus_trader.common.component cimport CMD
from nautilus_trader.common.component cimport EVT
from nautilus_trader.common.component cimport RECV
from nautilus_trader.common.component cimport Clock
from nautilus_trader.common.component cimport Component
from nautilus_trader.common.component cimport LogColor
from nautilus_trader.common.component cimport MessageBus
from nautilus_trader.common.component cimport Throttler
from nautilus_trader.common.messages cimport TradingStateChanged
from nautilus_trader.core.correctness cimport Condition
from nautilus_trader.core.datetime cimport unix_nanos_to_dt
from nautilus_trader.core.message cimport Command
from nautilus_trader.core.message cimport Event
from nautilus_trader.core.rust.model cimport AccountType
from nautilus_trader.core.rust.model cimport OrderSide
from nautilus_trader.core.rust.model cimport OrderStatus
from nautilus_trader.core.rust.model cimport OrderType
from nautilus_trader.core.rust.model cimport PositionSide
from nautilus_trader.core.rust.model cimport TimeInForce
from nautilus_trader.core.rust.model cimport TradingState
from nautilus_trader.core.rust.model cimport TrailingOffsetType
from nautilus_trader.core.rust.model cimport TriggerType
from nautilus_trader.core.uuid cimport UUID4
from nautilus_trader.core.nautilus_pyo3.markets import AShareSessionProvider
from nautilus_trader.core.nautilus_pyo3.markets import T1Ledger
from nautilus_trader.core.nautilus_pyo3.markets import compute_ashare_price_limits
from nautilus_trader.core.nautilus_pyo3.markets import compute_ashare_price_cage_violation_by_phase
from nautilus_trader.core.nautilus_pyo3.markets import compute_ashare_lot_size_violation
from nautilus_trader.core.nautilus_pyo3.markets import compute_ashare_price_limit_violation
from nautilus_trader.core.nautilus_pyo3.model import Price as PyO3Price
from nautilus_trader.core.nautilus_pyo3.markets import can_ashare_submit_order
from nautilus_trader.core.nautilus_pyo3.markets import can_ashare_cancel_order
from nautilus_trader.core.nautilus_pyo3.markets import is_ashare_trading_suspended
from nautilus_trader.core.nautilus_pyo3.markets import is_ashare_trading_resumed
from nautilus_trader.execution.messages cimport CancelAllOrders
from nautilus_trader.execution.messages cimport CancelOrder
from nautilus_trader.execution.messages cimport ModifyOrder
from nautilus_trader.execution.messages cimport SubmitOrder
from nautilus_trader.execution.messages cimport SubmitOrderList
from nautilus_trader.execution.messages cimport TradingCommand
from nautilus_trader.execution.trailing cimport TrailingStopCalculator
from nautilus_trader.model.data cimport InstrumentStatus
from nautilus_trader.model.data cimport QuoteTick
from nautilus_trader.model.data cimport TradeTick
from nautilus_trader.model.events.order cimport OrderCancelRejected
from nautilus_trader.model.events.order cimport OrderDenied
from nautilus_trader.model.events.order cimport OrderFilled
from nautilus_trader.model.events.order cimport OrderModifyRejected
from nautilus_trader.model.functions cimport order_type_to_str
from nautilus_trader.model.functions cimport trading_state_to_str
from nautilus_trader.model.functions cimport trailing_offset_type_to_str
from nautilus_trader.model.identifiers cimport AccountId
from nautilus_trader.model.identifiers cimport ComponentId
from nautilus_trader.model.identifiers cimport InstrumentId
from nautilus_trader.model.instruments.base cimport NEGATIVE_PRICE_INSTRUMENT_CLASSES
from nautilus_trader.model.instruments.base cimport Instrument
from nautilus_trader.model.objects cimport Currency
from nautilus_trader.model.objects cimport Money
from nautilus_trader.model.objects cimport Price
from nautilus_trader.model.objects cimport Quantity
from nautilus_trader.model.orders.base cimport Order
from nautilus_trader.model.orders.list cimport OrderList
from nautilus_trader.model.position cimport Position
from nautilus_trader.portfolio.base cimport PortfolioFacade


cdef class RiskEngine(Component):
    """
    提供高性能的风控引擎。

    `RiskEngine` 负责平台内的全局策略和投资组合风险。
    这包括盘前风控检查和盘后风险控制。

    可能的交易状态：
     - ``ACTIVE`` (交易已启用)。
     - ``REDUCING`` (仅允许执行减少现有头寸的新订单或更新)。
     - ``HALTED`` (除取消订单外的所有交易命令都将被拒绝)。

    参数
    ----------
    portfolio : PortfolioFacade
        引擎的投资组合。
    msgbus : MessageBus
        引擎的消息总线。
    cache : Cache
        引擎的缓存。
    clock : Clock
        引擎的时钟。
    config : RiskEngineConfig, 可选
        实例的配置。

    引发
    ------
    TypeError
        如果 `config` 不是 `RiskEngineConfig` 类型。
    """

    def __init__(
        self,
        PortfolioFacade portfolio not None,
        MessageBus msgbus not None,
        Cache cache not None,
        Clock clock not None,
        config: RiskEngineConfig | None = None,
    ) -> None:
        if config is None:
            config = RiskEngineConfig()

        Condition.type(config, RiskEngineConfig, "config")

        super().__init__(
            clock=clock,
            component_id=ComponentId("RiskEngine"),
            msgbus=msgbus,
            config=config,
        )

        self._portfolio = portfolio
        self._cache = cache

        # 配置
        self.trading_state = TradingState.ACTIVE  # 默认以活跃状态开始
        self.is_bypassed = config.bypass
        self.debug = config.debug
        self._log_state()

        # 计数器
        self.command_count = 0
        self.event_count = 0

        # 限流器
        pieces = config.max_order_submit_rate.split("/")
        order_submit_rate_limit = int(pieces[0])
        order_submit_rate_interval = pd.to_timedelta(pieces[1])
        self._order_submit_throttler = Throttler(
            name="ORDER_SUBMIT_THROTTLER",
            limit=order_submit_rate_limit,
            interval=order_submit_rate_interval,
            output_send=self._send_to_execution,
            output_drop=self._deny_new_order,
            clock=clock,
        )

        self._log.info(
            f"已设置最大订单提交速率: "
            f"{order_submit_rate_limit}/{str(order_submit_rate_interval).replace('0 days ', '')}",
            color=LogColor.BLUE,
        )

        pieces = config.max_order_modify_rate.split("/")
        order_modify_rate_limit = int(pieces[0])
        order_modify_rate_interval = pd.to_timedelta(pieces[1])
        self._order_modify_throttler = Throttler(
            name="ORDER_MODIFY_THROTTLER",
            limit=order_modify_rate_limit,
            interval=order_modify_rate_interval,
            output_send=self._send_to_execution,
            output_drop=self._deny_modify_order,
            clock=clock,
        )

        self._log.info(
            f"已设置最大订单修改速率: "
            f"{order_modify_rate_limit}/{str(order_modify_rate_interval).replace('0 days ', '')}",
            color=LogColor.BLUE,
        )

        if config.max_trade_command_rate:
            pieces = config.max_trade_command_rate.split("/")
            global_trade_rate_limit = int(pieces[0])
            global_trade_rate_interval = pd.to_timedelta(pieces[1])
            self._global_trade_throttler = Throttler(
                name="GLOBAL_TRADE_THROTTLER",
                limit=global_trade_rate_limit,
                interval=global_trade_rate_interval,
                output_send=self._direct_send_to_execution,
                output_drop=self._deny_global_trade_command,
                clock=clock,
            )
            self._log.info(
                f"已设置最大全局交易指令速率: "
                f"{global_trade_rate_limit}/{str(global_trade_rate_interval).replace('0 days ', '')}",
                color=LogColor.BLUE,
            )
        else:
            self._global_trade_throttler = None

        # 风险设置
        self._max_notional_per_order: dict[InstrumentId, Decimal] = {}

        self._init_ashare_extensions()

        # 配置
        self._initialize_risk_checks(config)

        # 注册端点
        self._msgbus.register(endpoint="RiskEngine.execute", handler=self.execute)
        self._msgbus.register(endpoint="RiskEngine.process", handler=self.process)

        # 必要订阅
        self._msgbus.subscribe(topic="events.order.*", handler=self._handle_event, priority=10)
        self._msgbus.subscribe(topic="events.position.*", handler=self._handle_event, priority=10)
        self._msgbus.subscribe(topic="events.status.*", handler=self._handle_event, priority=10)

    def _initialize_risk_checks(self, config: RiskEngineConfig):
        cdef dict max_notional_config = config.max_notional_per_order

        for instrument_id, value in max_notional_config.items():
            self.set_max_notional_per_order(InstrumentId.from_str_c(instrument_id), Decimal(value))

# -- 命令 -----------------------------------------------------------------------------------------

    cpdef void execute(self, Command command):
        """
        执行给定的命令。

        参数
        ----------
        command : Command
            要执行的命令。

        """
        Condition.not_none(command, "command")

        self._execute_command(command)

    cpdef void process(self, Event event):
        """
        处理给定的事件。

        参数
        ----------
        event : Event
            要处理的事件。

        """
        Condition.not_none(event, "event")

        self._handle_event(event)

    cpdef void set_trading_state(self, TradingState state):
        """
        为引擎设置交易状态。

        参数
        ----------
        state : TradingState
            要设置的状态。

        """
        if state == self.trading_state:
            self._log.warning(
                f"交易状态未改变：已设置为 {trading_state_to_str(self.trading_state)}",
            )
            return

        self.trading_state = state

        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef TradingStateChanged event = TradingStateChanged(
            trader_id=self.trader_id,
            state=self.trading_state,
            config=self._config,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )

        self._msgbus.publish_c(topic="events.risk", msg=event)
        self._log_state()

    cpdef void _log_state(self):
        cdef LogColor color = LogColor.BLUE

        if self.trading_state == TradingState.REDUCING:
            color = LogColor.YELLOW
        elif self.trading_state == TradingState.HALTED:
            color = LogColor.RED

        self._log.info(
            f"交易状态为 {trading_state_to_str(self.trading_state)}",
            color=color,
        )

        if self.is_bypassed:
            self._log.info(
                "盘前风控检查已跳过。实盘交易不建议这样做。",
                color=LogColor.RED,
            )

    cpdef void set_max_notional_per_order(self, InstrumentId instrument_id, new_value):
        """
        为给定的标的 ID 设置每笔订单的最大名义价值。

        将 `new_value` 设置为 `None` 将禁用盘前最大名义价值检查。

        参数
        ----------
        instrument_id : InstrumentId
            最大名义价值对应的标的 ID。
        new_value : 整数, 浮点数, 字符串 或 Decimal
            要设置的最大名义价值。

        引发
        ------
        decimal.InvalidOperation
            如果 `new_value` 不是 `decimal.Decimal` 的有效输入。
        ValueError
            如果 `new_value` 不是 `None` 且不为正数。

        """
        if new_value is not None:
            new_value = Decimal(new_value)
            Condition.type(new_value, Decimal, "new_value")
            Condition.positive(new_value, "new_value")

        old_value: Decimal = self._max_notional_per_order.get(instrument_id)
        self._max_notional_per_order[instrument_id] = new_value

        cdef str new_value_str = f"{new_value:,}" if new_value is not None else str(None)
        self._log.info(
            f"已设置最大订单名义价值: {instrument_id} {new_value_str}",
            color=LogColor.BLUE,
        )

# -- 风险设置 --------------------------------------------------------------------------------------

    cpdef tuple max_order_submit_rate(self):
        """
        返回当前最大订单提交速率限制设置。

        返回
        -------
        (int, timedelta)
            每个时间间隔内的限制数量。

        """
        return (
            self._order_submit_throttler.limit,
            self._order_submit_throttler.interval,
        )

    cpdef tuple max_order_modify_rate(self):
        """
        返回当前最大订单修改速率限制设置。

        返回
        -------
        (int, timedelta)
            每个时间间隔内的限制数量。

        """
        return (
            self._order_modify_throttler.limit,
            self._order_modify_throttler.interval,
        )

    cpdef dict max_notionals_per_order(self):
        """
        返回当前每笔订单最大名义价值设置。

        返回
        -------
        dict[InstrumentId, Decimal]

        """
        return self._max_notional_per_order.copy()

    cpdef object max_notional_per_order(self, InstrumentId instrument_id):
        """
        返回给定标的 ID 当前的每笔订单最大名义价值。

        返回
        -------
        Decimal 或 ``None``

        """
        return self._max_notional_per_order.get(instrument_id)

# -- 抽象方法 --------------------------------------------------------------------------------------

    cpdef void _on_start(self):
        pass  # 可在子类中重写

    cpdef void _on_stop(self):
        pass  # 可在子类中重写

# -- 动作实现 --------------------------------------------------------------------------------------

    cpdef void _start(self):
        # 暂时不进行其他操作
        self._on_start()

    cpdef void _stop(self):
        # 暂时不进行其他操作
        self._on_stop()

    cpdef void _reset(self):
        self.command_count = 0
        self.event_count = 0
        self._order_submit_throttler.reset()
        self._order_modify_throttler.reset()
        if self._global_trade_throttler is not None:
            self._global_trade_throttler.reset()

    cpdef void _dispose(self):
        pass
        # 暂时没有可销毁的内容

# -- 命令处理器 --------------------------------------------------------------------------------------

    cpdef void _execute_command(self, Command command):
        if self.debug:
            self._log.debug(f"{RECV}{CMD} {command}", LogColor.MAGENTA)

        self.command_count += 1

        if isinstance(command, SubmitOrder):
            self._handle_submit_order(command)
        elif isinstance(command, SubmitOrderList):
            self._handle_submit_order_list(command)
        elif isinstance(command, ModifyOrder):
            self._handle_modify_order(command)
        elif isinstance(command, (CancelOrder, CancelAllOrders)):
            self._handle_cancel_command(command)
        else:
            self._log.error(f"无法处理命令：{command}")

    cpdef void _handle_submit_order(self, SubmitOrder command):
        if self.is_bypassed:
            # 不再进行进一步的风控检查或限流
            self._send_to_execution(command)
            return

        cdef Order order = command.order

        # 检查只减仓 (reduce only)
        cdef Position position

        if command.position_id is not None:
            if order.is_reduce_only:
                position = self._cache.position(command.position_id)

                if position is None or not order.would_reduce_only(position.side, position.quantity):
                    self._deny_command(
                        command=command,
                        reason=f"只减仓订单将增加仓位规模 {command.position_id!r}",
                    )
                    return  # 拒绝

        # 获取订单的标的定义
        cdef Instrument instrument = self._cache.instrument(order.instrument_id)

        if instrument is None:
            self._deny_command(
                command=command,
                reason=f"未找到 {order.instrument_id} 的标的定义",
            )
            return  # 拒绝

        # A 股扩展: Session 检查
        cdef str submit_session_violation = self._check_ashare_submit_session()
        if submit_session_violation:
            self._deny_command(command=command, reason=submit_session_violation)
            return  # 拒绝

        ########################################################################
        # 盘前订单检查
        ########################################################################
        if not self._check_order(instrument, order):
            return  # 拒绝

        if not self._check_orders_risk(instrument, [order]):
            return  # 拒绝

        self._execution_gateway(instrument, command)

    cpdef void _handle_submit_order_list(self, SubmitOrderList command):
        if self.is_bypassed:
            # 不进行进一步的风控检查或限流
            self._send_to_execution(command)
            return

        # 获取订单的标的定义
        cdef Instrument instrument = self._cache.instrument(command.instrument_id)

        if instrument is None:
            self._deny_command(
                command=command,
                reason=f"未找到 {command.instrument_id} 的标的定义",
            )
            return  # 拒绝

        ########################################################################
        # 盘前订单检查
        ########################################################################
        for order in command.order_list.orders:
            if not self._check_order(instrument, order):
                return  # 拒绝

        if not self._check_orders_risk(instrument, command.order_list.orders):
            # 拒绝列表中的所有订单
            self._deny_order_list(command.order_list, f"订单列表 {command.order_list.id.to_str()} 已拒绝")
            return # 拒绝

        self._execution_gateway(instrument, command)

    cpdef void _handle_modify_order(self, ModifyOrder command):
        ########################################################################
        # 验证命令
        ########################################################################
        cdef Order order = self._cache.order(command.client_order_id)

        if order is None:
            self._log.error(
                f"ModifyOrder 已拒绝: 未找到 ID 为 {command.client_order_id!r} 的订单",
            )
            return  # 拒绝
        elif order.is_closed_c():
            self._reject_modify_order(
                order=order,
                reason=f"ID 为 {command.client_order_id!r} 的订单已收盘",
            )
            return  # 拒绝
        elif order.is_pending_cancel_c():
            self._reject_modify_order(
                order=order,
                reason=f"ID 为 {command.client_order_id!r} 的订单已在申请撤单中",
            )
            return  # 拒绝

        # 获取订单的标的定义
        cdef Instrument instrument = self._cache.instrument(command.instrument_id)

        if instrument is None:
            self._reject_modify_order(
                order=order,
                reason=f"未找到 {command.instrument_id} 的标的定义",
            )
            return  # 拒绝

        cdef str risk_msg = None

        # 检查价格
        risk_msg = self._check_price(instrument, command.price)

        if risk_msg:
            self._reject_modify_order(order=order, reason=risk_msg)
            return  # 拒绝

        # 检查触发价
        risk_msg = self._check_price(instrument, command.trigger_price)

        if risk_msg:
            self._reject_modify_order(order=order, reason=risk_msg)
            return  # 拒绝

        # 检查数量
        risk_msg = self._check_quantity(instrument, command.quantity, order.is_quote_quantity)

        if risk_msg:
            self._reject_modify_order(order=order, reason=risk_msg)
            return  # 拒绝

        # 检查 A 股标的状态 (Halt, Suspend, NotAvailableForTrading)
        status = self._cache.instrument_status(command.instrument_id)
        if status is not None:
             if is_ashare_trading_suspended(status.action):
                 self._reject_modify_order(
                     order=order,
                     reason=f"INSTRUMENT_SUSPENDED: status={status.action}",
                 )
                 return

        # 检查交易状态 (TradingState)
        if self.trading_state == TradingState.HALTED:
            self._reject_modify_order(
                order=order,
                reason="交易状态已熔断 (HALTED)",
            )
            return  # 拒绝
        elif self.trading_state == TradingState.REDUCING:
            if command.quantity and command.quantity > order.quantity:
                if order.is_buy_c() and self._portfolio.is_net_long(instrument.id):
                    self._reject_modify_order(
                        order=order,
                        reason="交易状态为 REDUCING，且更新将增加风险敞口",
                    )
                    return  # 拒绝
                elif order.is_sell_c() and self._portfolio.is_net_short(instrument.id):
                    self._reject_modify_order(
                        order=order,
                        reason="交易状态为 REDUCING，且更新将增加风险敞口",
                    )
                    return  # 拒绝

        self._order_modify_throttler.send(command)

    cpdef void _handle_cancel_command(self, TradingCommand command):
        if self.is_bypassed:
            self._send_to_execution(command)
            return

        # A 股扩展: Cancel session 检查
        cdef str cancel_session_violation = self._check_ashare_cancel_session()
        if cancel_session_violation:
            self._reject_cancel_command(command, reason=cancel_session_violation)
            return  # 拒绝拦截

        # 发送执行
        self._send_to_execution(command)

# -- 盘前检查 -----------------------------------------------------------------------------

    cpdef bint _check_order(self, Instrument instrument, Order order):
        ########################################################################
        # 验证检查
        ########################################################################

        if self.debug:
            self._log.debug(f"正在验证 {order}", LogColor.MAGENTA)

        if not self._check_order_price(instrument, order):
            return False  # 拒绝

        if not self._check_order_quantity(instrument, order):
            return False  # 拒绝

        if order.time_in_force == TimeInForce.GTD:
            if order.expire_time_ns <= self._clock.timestamp_ns():
                self._deny_order(
                    order=order,
                    reason=f"GTD 过期时间 {unix_nanos_to_dt(order.expire_time_ns)} 已过",
                )
                return False  # 拒绝

        return True  # 检查通过

    cpdef bint _check_order_price(self, Instrument instrument, Order order):
        ########################################################################
        # 检查价格
        ########################################################################
        cdef str risk_msg = None

        if order.has_price_c():
            risk_msg = self._check_price(instrument, order.price)

            if risk_msg:
                self._deny_order(order=order, reason=risk_msg)
                return False  # 拒绝

        ########################################################################
        # 检查触发价
        ########################################################################
        if order.has_trigger_price_c():
            risk_msg = self._check_price(instrument, order.trigger_price)

            if risk_msg:
                self._deny_order(order=order, reason=f"触发价 {risk_msg}")
                return False  # 拒绝

        cdef str price_cage_violation = self._check_ashare_price_cage(instrument, order)
        if price_cage_violation:
            self._deny_order(order=order, reason=price_cage_violation)
            return False

        return True  # 通过

    cpdef bint _check_order_quantity(self, Instrument instrument, Order order):
        cdef str risk_msg = self._check_quantity(instrument, order.quantity, order.is_quote_quantity)
        cdef uint64_t qty_raw
        cdef uint64_t lot_raw

        if risk_msg:
            self._deny_order(order=order, reason=risk_msg)
            return False  # 拒绝

        cdef str lot_size_violation = self._check_ashare_lot_size(instrument, order)
        if lot_size_violation:
            self._deny_order(order=order, reason=lot_size_violation)
            return False

        return True  # 通过

    cpdef bint _check_orders_risk(self, Instrument instrument, list orders):
        ########################################################################
        # 风控检查
        ########################################################################

        # 按 account_id 对订单进行分组，以处理每个标的多个账户的情况
        cdef dict orders_by_account = {}  # type: dict[AccountId, list]
        cdef:
            Order order
            AccountId account_id
        for order in orders:
            if order.account_id not in orders_by_account:
                orders_by_account[order.account_id] = []

            orders_by_account[order.account_id].append(order)

        # 分别检查每个账户组
        cdef list account_orders
        for account_id, account_orders in orders_by_account.items():
            if not self._check_orders_risk_for_account(instrument, account_orders, account_id):
                return False  # 拒绝

        return True  # 所有检查通过

    cpdef bint _check_orders_risk_for_account(self, Instrument instrument, list orders, AccountId account_id):
        # 检查特定账户的订单（如果 account_id 为 None，则基于场地进行查找）
        cdef QuoteTick last_quote = None
        cdef TradeTick last_trade = None
        cdef Price last_px = None
        cdef Money free

        # 确定最大名义价值
        cdef Money max_notional = None
        max_notional_setting: Decimal | None = self._max_notional_per_order.get(instrument.id)

        if max_notional_setting:
            # TODO: 优化此处的效率
            max_notional = Money(float(max_notional_setting), instrument.quote_currency)

        # 获取风控检查所需的账户
        cdef Account account = self._cache.account_for_venue(instrument.id.venue, account_id)

        if account is None:
            self._log.debug(
                f"无法找到场地 {instrument.id.venue} 的账户 "
                f"(account_id={account_id.get_issuer() if account_id is not None else None})"
            )
            return True  # TODO: 暂时提前返回，直到处理完路由/多场地情况
        
        if account.is_margin_account:
            return True  # TODO: 确定保证金账户的风控策略

        cdef bint allow_borrowing = isinstance(account, CashAccount) and account.allow_borrowing

        free = account.balance_free(instrument.quote_currency)

        if self.debug:
            self._log.debug(f"可用余额: {free!r}", LogColor.MAGENTA)

        # 获取该标的的净多头持仓数量（用于只减仓卖单检查），
        # 同时考虑已提交（但未成交）的卖单，以防止超卖。
        cdef list[Position] open_longs = self._cache.positions_open(
            None,
            instrument.id,
            None,
            PositionSide.LONG,
        )
        cdef Quantity net_long_qty = Quantity.zero_c(instrument.size_precision)
        cdef Position position
        for position in open_longs:
            net_long_qty = Quantity.from_raw_c(
                net_long_qty._mem.raw + position.quantity._mem.raw,
                instrument.size_precision,
            )

        # 获取该标的的待处理（未平仓）卖单
        cdef list open_sell_orders = self._cache.orders_open(
            None,
            instrument.id,
            None,
            OrderSide.SELL,
        )
        cdef Quantity submitted_sell_qty = Quantity.zero_c(instrument.size_precision)
        cdef Order open_order
        for open_order in open_sell_orders:
            submitted_sell_qty = Quantity.from_raw_c(
                submitted_sell_qty._mem.raw + open_order.leaves_qty._mem.raw,
                instrument.size_precision,
            )

        # 可用数量为多头持仓减去已提交的卖单
        cdef Quantity available_long_qty
        if submitted_sell_qty._mem.raw >= net_long_qty._mem.raw:
            available_long_qty = Quantity.zero_c(instrument.size_precision)
        else:
            available_long_qty = Quantity.from_raw_c(
                net_long_qty._mem.raw - submitted_sell_qty._mem.raw,
                instrument.size_precision,
            )

        if self.debug and net_long_qty._mem.raw > 0:
            self._log.debug(
                f"净多头数量: {net_long_qty}, 已提交卖单数量: {submitted_sell_qty}, 可用数量: {available_long_qty}",
                LogColor.MAGENTA,
            )

        # 跟踪累计卖出数量，以确定是平仓卖出还是开仓卖出
        cdef Quantity cum_sell_qty = Quantity.zero_c(instrument.size_precision)

        cdef:
            Order order
            Money notional
            Money cum_notional_buy = None
            Money cum_notional_sell = None
            Money order_balance_impact = None
            Money cash_value = None
            Currency base_currency = None
            double xrate
            Quantity effective_quantity
            Price effective_price
            bint is_position_reducing_sell
            Quantity pending_sell_qty
        for order in orders:
            if self.debug:
                self._log.debug(f"盘前风控检查: {order}", LogColor.MAGENTA)

            if order.order_type == OrderType.MARKET or order.order_type == OrderType.MARKET_TO_LIMIT:
                if last_px is None:
                    # 确定入场价格
                    last_quote = self._cache.quote_tick(instrument.id)

                    if last_quote is not None:
                        if order.side == OrderSide.BUY:
                            last_px = last_quote.ask_price
                        elif order.side == OrderSide.SELL:
                            last_px = last_quote.bid_price
                        else:  # pragma: no cover (设计时错误)
                            raise RuntimeError(f"无效的 `OrderSide`")
                    else:
                        last_trade = self._cache.trade_tick(instrument.id)

                        if last_trade is not None:
                            last_px = last_trade.price
                        else:
                            self._log.warning(
                                f"无法检查市价单风险：没有 {instrument.id} 的价格数据",
                            )
                            continue  # 无法检查订单风险
            elif order.order_type == OrderType.STOP_MARKET or order.order_type == OrderType.MARKET_IF_TOUCHED:
                last_px = order.trigger_price
            elif order.order_type == OrderType.TRAILING_STOP_MARKET or order.order_type == OrderType.TRAILING_STOP_LIMIT:
                if order.trigger_price is None:
                    # 验证追踪偏移类型是否受支持
                    if order.trailing_offset_type not in (TrailingOffsetType.PRICE, TrailingOffsetType.BASIS_POINTS, TrailingOffsetType.TICKS):
                        self._deny_order(
                            order=order,
                            reason=f"不支持的追踪偏移类型 (UNSUPPORTED_TRAILING_OFFSET_TYPE): {trailing_offset_type_to_str(order.trailing_offset_type)}",
                        )
                        return False

                    last_trade = None
                    last_quote = None

                    if order.trigger_type == TriggerType.BID_ASK:
                        last_quote = self._cache.quote_tick(instrument.id)
                        if last_quote is None:
                            self._log.warning(
                                f"无法检查 {order_type_to_str(order.order_type)} 订单风险：未设置触发价且没有 {instrument.id} 的买卖报价数据",
                            )
                            continue
                        last_px = TrailingStopCalculator.calculate_with_bid_ask(
                            price_increment=instrument.price_increment,
                            trailing_offset_type=order.trailing_offset_type,
                            side=order.side,
                            offset=float(order.trailing_offset),
                            bid=last_quote.bid_price,
                            ask=last_quote.ask_price,
                        )
                    else:
                        last_trade = self._cache.trade_tick(instrument.id)
                        if last_trade is not None:
                            last_px = TrailingStopCalculator.calculate_with_last(
                                price_increment=instrument.price_increment,
                                trailing_offset_type=order.trailing_offset_type,
                                side=order.side,
                                offset=float(order.trailing_offset),
                                last=last_trade.price,
                            )
                        elif order.trigger_type == TriggerType.LAST_OR_BID_ASK:
                            # 如果没有成交数据，则回退到买卖报价
                            last_quote = self._cache.quote_tick(instrument.id)
                            if last_quote is None:
                                self._log.warning(
                                    f"无法检查 {order_type_to_str(order.order_type)} 订单风险：未设置触发价且没有 {instrument.id} 的市场数据",
                                )
                                continue
                            last_px = TrailingStopCalculator.calculate_with_bid_ask(
                                price_increment=instrument.price_increment,
                                trailing_offset_type=order.trailing_offset_type,
                                side=order.side,
                                offset=float(order.trailing_offset),
                                bid=last_quote.bid_price,
                                ask=last_quote.ask_price,
                            )
                        else:
                            self._log.warning(
                                f"无法检查 {order_type_to_str(order.order_type)} 订单风险：未设置触发价且没有 {instrument.id} 的市场数据",
                            )
                            continue
                else:
                    last_px = order.trigger_price
            else:
                last_px = order.price

            # 对于以报价币种计算数量的限价单，使用最坏情况的执行价格
            if (
                order.is_quote_quantity
                and not instrument.is_inverse
                and (order.order_type == OrderType.LIMIT or order.order_type == OrderType.STOP_LIMIT)
            ):
                # 获取当前市场价格进行最坏情况执行评估
                last_quote = self._cache.quote_tick(instrument.id)
                if last_quote is not None:
                    if order.side == OrderSide.BUY:
                        # 买入：如果限价低于卖一价，则可能在卖一价执行（需更多数量）
                        effective_price = last_px if last_px < last_quote.ask_price else last_quote.ask_price
                    elif order.side == OrderSide.SELL:
                        # 卖出：如果限价高于买一价，则可能在买一价执行（但数量较少，因此使用限价）
                        effective_price = last_px if last_px > last_quote.bid_price else last_quote.bid_price
                    else:
                        effective_price = last_px
                else:
                    effective_price = last_px  # 无市场数据，使用限价
            else:
                effective_price = last_px

            # 如果需要计算余额影响，将报价币种数量转换为基础币种数量
            if order.is_quote_quantity and not instrument.is_inverse:
                effective_quantity = instrument.calculate_base_quantity(order.quantity, effective_price)

                if self.debug:
                    self._log.debug(f"已将报价币种数量 {order.quantity} 转换为基础币种数量 {effective_quantity}", LogColor.MAGENTA)
            else:
                effective_quantity = order.quantity

            # 根据有效数量检查最小/最大数量限制
            if instrument.max_quantity and effective_quantity > instrument.max_quantity:
                self._deny_order(
                    order=order,
                    reason=f"数量超过最大限制 (QUANTITY_EXCEEDS_MAXIMUM): 有效数量={effective_quantity}, 最大数量={instrument.max_quantity}",
                )
                return False  # 拒绝

            if instrument.min_quantity and effective_quantity < instrument.min_quantity:
                self._deny_order(
                    order=order,
                    reason=f"数量低于最小限制 (QUANTITY_BELOW_MINIMUM): 有效数量={effective_quantity}, 最小数量={instrument.min_quantity}",
                )
                return False  # 拒绝

            notional = instrument.notional_value(effective_quantity, last_px, use_quote_for_inverse=True)

            if self.debug:
                self._log.debug(f"名义价值: {notional!r}", LogColor.MAGENTA)

            if max_notional and notional._mem.raw > max_notional._mem.raw:
                self._deny_order(
                    order=order,
                    reason=f"名义价值超过每笔订单最大限制 (NOTIONAL_EXCEEDS_MAX_PER_ORDER): 最大名义价值={max_notional}, 当前名义价值={notional}",
                )
                return False  # 拒绝

            # 检查标的的最小名义价值限制
            if (
                instrument.min_notional is not None
                and instrument.min_notional.currency == notional.currency
                and notional._mem.raw < instrument.min_notional._mem.raw
            ):
                self._deny_order(
                    order=order,
                    reason=f"名义价值低于标的最小限制 (NOTIONAL_LESS_THAN_MIN_FOR_INSTRUMENT): 最小名义价值={instrument.min_notional} , 当前名义价值={notional}",
                )
                return False  # 拒绝

            # 检查标度的最大名义价值限制
            if (
                instrument.max_notional is not None
                and instrument.max_notional.currency == notional.currency
                and notional._mem.raw > instrument.max_notional._mem.raw
            ):
                self._deny_order(
                    order=order,
                    reason=f"名义价值超过标的最大限制 (NOTIONAL_GREATER_THAN_MAX_FOR_INSTRUMENT): 最大名义价值={instrument.max_notional}, 当前名义价值={notional}",
                )
                return False  # 拒绝

            order_balance_impact = account.balance_impact(instrument, effective_quantity, last_px, order.side)

            if self.debug:
                self._log.debug(f"余额影响: {order_balance_impact!r}", LogColor.MAGENTA)

            # 当允许借入时（例如现货杠杆交易），跳过余额检查
            if not allow_borrowing and free is not None and (free._mem.raw + order_balance_impact._mem.raw) < 0:
                self._deny_order(
                    order=order,
                    reason=f"名义价值超过可用余额 (NOTIONAL_EXCEEDS_FREE_BALANCE): 可用余额={free}, 余额影响={order_balance_impact}",
                )
                return False  # 拒绝

            if base_currency is None:
                base_currency = instrument.get_base_currency()

            if order.is_buy_c():
                if cum_notional_buy is None:
                    cum_notional_buy = Money(-order_balance_impact, order_balance_impact.currency)
                else:
                    cum_notional_buy._mem.raw += -order_balance_impact._mem.raw

                if self.debug:
                    self._log.debug(f"累计买入名义价值: {cum_notional_buy!r}")

                if not allow_borrowing and free is not None and cum_notional_buy._mem.raw > free._mem.raw:
                    self._deny_order(
                        order=order,
                        reason=f"累计名义价值超过可用余额 (CUM_NOTIONAL_EXCEEDS_FREE_BALANCE): 可用余额={free}, 累计名义价值={cum_notional_buy}",
                    )
                    return False  # 拒绝
            elif order.is_sell_c():
                pending_sell_qty = Quantity.from_raw_c(
                    cum_sell_qty._mem.raw + effective_quantity._mem.raw,
                    instrument.size_precision,
                )
                is_position_reducing_sell = (
                    order.is_reduce_only
                    or pending_sell_qty._mem.raw <= available_long_qty._mem.raw
                )
                cum_sell_qty = pending_sell_qty

                if is_position_reducing_sell:
                    if self.debug:
                        self._log.debug(
                            "平仓卖单跳过余额检查",
                            LogColor.MAGENTA,
                        )
                    continue

                if account.base_currency is not None:
                    if cum_notional_sell is None:
                        cum_notional_sell = Money(order_balance_impact, order_balance_impact.currency)
                    else:
                        cum_notional_sell._mem.raw += order_balance_impact._mem.raw

                    if self.debug:
                        self._log.debug(f"累计卖出名义价值: {cum_notional_sell!r}")
                    if not allow_borrowing and free is not None and cum_notional_sell._mem.raw > free._mem.raw:
                        self._deny_order(
                            order=order,
                            reason=f"累计名义价值超过可用余额 (CUM_NOTIONAL_EXCEEDS_FREE_BALANCE): 可用余额={free}, 累计名义价值={cum_notional_sell}",
                        )
                        return False  # 拒绝
                elif base_currency is not None and account.type == AccountType.CASH:
                    cash_value = Money(effective_quantity.as_f64_c(), base_currency)
                    free = account.balance_free(base_currency)

                    if self.debug:
                        total = account.balance_total(base_currency)
                        locked = account.balance_locked(base_currency)
                        self._log.debug(f"现货价值: {cash_value!r}", LogColor.MAGENTA)
                        self._log.debug(f"总额: {total!r}", LogColor.MAGENTA)
                        self._log.debug(f"锁定: {locked!r}", LogColor.MAGENTA)
                        self._log.debug(f"可用: {free!r}", LogColor.MAGENTA)

                    if cum_notional_sell is None:
                        cum_notional_sell = cash_value
                    else:
                        cum_notional_sell._mem.raw += cash_value._mem.raw

                    if self.debug:
                        self._log.debug(f"累计卖出名义价值: {cum_notional_sell!r}")
                    if not allow_borrowing and free is not None and cum_notional_sell._mem.raw > free._mem.raw:
                        self._deny_order(
                            order=order,
                            reason=f"累计名义价值超过可用余额 (CUM_NOTIONAL_EXCEEDS_FREE_BALANCE): 可用余额={free}, 累计名义价值={cum_notional_sell}",
                        )
                        return False  # 拒绝

        # 最后
        return True  # 通过

    cpdef str _check_price(self, Instrument instrument, Price price):
        if price is None:
            # 无需检查
            return None

        if price.precision > instrument.price_precision:
            # 检查失败
            return f"价格 {price} 无效 (精度 {price.precision} > {instrument.price_precision})"

        if instrument.instrument_class not in NEGATIVE_PRICE_INSTRUMENT_CLASSES:
            if price.raw_int_c() <= 0:
                # 检查失败
                return f"价格 {price} 无效 (非正数)"

        cdef str ashare_price_violation = self._check_ashare_price_rules(instrument, price)
        if ashare_price_violation:
            return ashare_price_violation


    cpdef str _check_quantity(self, Instrument instrument, Quantity quantity, bint is_quote_quantity=False):
        if quantity is None:
            # 无需检查
            return None

        if quantity._mem.precision > instrument.size_precision:
            # 检查失败
            return f"数量 {quantity} 无效 (精度 {quantity._mem.precision} > {instrument.size_precision})"

        # 对于以报价币种计算数量的订单，跳过最小/最大检查（这些将在 _check_orders_risk 中使用 effective_quantity 进行检查）
        if is_quote_quantity:
            return None

        if instrument.max_quantity and quantity > instrument.max_quantity:
            # 检查失败
            return f"数量 {quantity} 无效 (> 最大交易数量 {instrument.max_quantity})"

        if instrument.min_quantity and quantity < instrument.min_quantity:
            # 检查失败
            return f"数量 {quantity} 无效 (< 最小交易数量 {instrument.min_quantity})"

# -- 拒绝处理 --------------------------------------------------------------------------------------

    cpdef void _deny_command(self, TradingCommand command, str reason):
        if isinstance(command, SubmitOrder):
            self._deny_order(command.order, reason=reason)
        elif isinstance(command, SubmitOrderList):
            self._deny_order_list(command.order_list, reason=reason)
        else:  # pragma: no cover (设计时错误)
            raise RuntimeError(f"无法拒绝命令 {command}")  # pragma: no cover (设计时错误)

    # Needs to be `cpdef` due being called from throttler
    cpdef void _deny_new_order(self, TradingCommand command):
        if isinstance(command, SubmitOrder):
            self._deny_order(command.order, reason="超出最大订单提交速率 (MAX_ORDER_SUBMIT_RATE)")
        elif isinstance(command, SubmitOrderList):
            self._deny_order_list(command.order_list, reason="超出最大订单提交速率 (MAX_ORDER_SUBMIT_RATE)")

    cpdef void _deny_global_trade_command(self, TradingCommand command):
        cdef str reason = "超出最大全局交易指令速率 (MAX_TRADE_COMMAND_RATE)"
        cdef Order order

        if isinstance(command, SubmitOrder):
            self._deny_order(command.order, reason=reason)
        elif isinstance(command, SubmitOrderList):
            self._deny_order_list(command.order_list, reason=reason)
        elif isinstance(command, ModifyOrder):
            order = self._cache.order(command.client_order_id)
            if order is not None:
                self._reject_modify_order(order, reason=reason)
        elif isinstance(command, CancelOrder) or isinstance(command, CancelAllOrders):
            self._reject_cancel_command(command, reason=reason)

    # Needs to be `cpdef` due being called from throttler
    cpdef void _deny_modify_order(self, ModifyOrder command):
        cdef Order order = self._cache.order(command.client_order_id)

        if order is None:
            self._log.error(f"未找到 ID 为 {command.client_order_id!r} 的订单")
            return

        self._reject_modify_order(order, reason="超出最大订单修改速率 (MAX_ORDER_MODIFY_RATE)")

    cpdef void _deny_order(self, Order order, str reason):
        self._log.warning(f"订单提交 {order.client_order_id.to_str()} 已拒绝: {reason}")

        if order is None:
            # 无需执行拒绝操作
            return

        if order.status_c() != OrderStatus.INITIALIZED:
            # 已经处于拒绝状态或重复 (INITIALIZED -> DENIED 是唯一的有效状态转换)
            return

        if not self._cache.order_exists(order.client_order_id):
            self._cache.add_order(order)

        # 生成事件
        cdef OrderDenied denied = OrderDenied(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            reason=reason,
            event_id=UUID4(),
            ts_init=self._clock.timestamp_ns(),
        )

        self._msgbus.send(endpoint="ExecEngine.process", msg=denied)

    cpdef void _deny_order_list(self, OrderList order_list, str reason):
        cdef Order order
        for order in order_list.orders:
            if not order.is_closed_c():
                self._deny_order(order=order, reason=reason)

# -- 输出处理 --------------------------------------------------------------------------------------

    cpdef void _execution_gateway(self, Instrument instrument, TradingCommand command):
        # 检查交易状态 (TradingState)
        cdef Order order

        if self.trading_state == TradingState.HALTED:
            if isinstance(command, SubmitOrder):
                self._deny_command(
                    command=command,
                    reason=f"交易状态已熔断 (TradingState.HALTED)",
                )
                return  # 拒绝
            elif isinstance(command, SubmitOrderList):
                self._deny_order_list(
                    order_list=command.order_list,
                    reason="交易状态已熔断 (TradingState.HALTED)",
                )
                return  # 拒绝
        elif self.trading_state == TradingState.REDUCING:
            if isinstance(command, SubmitOrder):
                order = command.order

                if order.is_buy_c() and self._portfolio.is_net_long(instrument.id):
                    self._deny_command(
                        command=command,
                        reason=f"交易状态为 REDUCING 且持有 {instrument.id} 多头仓位时禁止买入",
                    )
                    return  # 拒绝
                elif order.is_sell_c() and self._portfolio.is_net_short(instrument.id):
                    self._deny_command(
                        command=command,
                        reason=f"交易状态为 REDUCING 且持有 {instrument.id} 空头仓位时禁止卖出",
                    )
                    return  # 拒绝
            elif isinstance(command, SubmitOrderList):
                for order in command.order_list.orders:
                    if order.is_buy_c() and self._portfolio.is_net_long(instrument.id):
                        self._deny_order_list(
                            order_list=command.order_list,
                            reason=f"订单列表中包含买单，而交易状态为 REDUCING 且持有 {instrument.id} 多头仓位",
                        )
                        return  # 拒绝
                    elif order.is_sell_c() and self._portfolio.is_net_short(instrument.id):
                        self._deny_order_list(
                            order_list=command.order_list,
                            reason=f"订单列表中包含卖单，而交易状态为 REDUCING 且持有 {instrument.id} 空头仓位",
                        )
                        return  # 拒绝

        # 所有检查通过：送入订单速率限流器 (ORDER_RATE throttler)
        self._order_submit_throttler.send(command)

    # Needs to be `cpdef` due being called from throttler
    cpdef void _send_to_execution(self, TradingCommand command):
        if self._global_trade_throttler is not None:
            if isinstance(command, (SubmitOrder, SubmitOrderList, ModifyOrder, CancelOrder)):
                self._global_trade_throttler.send(command)
                return
        self._direct_send_to_execution(command)

    cpdef void _direct_send_to_execution(self, TradingCommand command):
        self._msgbus.send(endpoint="ExecEngine.execute", msg=command)

    cpdef void _reject_modify_order(self, Order order, str reason):
        # 生成事件
        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderModifyRejected denied = OrderModifyRejected(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=order.venue_order_id,
            account_id=order.account_id,
            reason=reason,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )

        self._msgbus.send(endpoint="ExecEngine.process", msg=denied)

    cpdef void _reject_cancel_command(self, TradingCommand command, str reason):
        """A 股扩展: 撤单被阶段规则拒绝时，发回 OrderCancelRejected 事件通知策略。"""
        if not isinstance(command, CancelOrder):
            # CancelAllOrders 无对应的 OrderCancelRejected，仅记录日志
            self._log.warning(f"{reason}")
            return

        # 尝试从缓存和命令中获取订单元数据
        cdef CancelOrder cancel = command
        cdef Order order = self._cache.order(cancel.client_order_id)

        cdef uint64_t ts_now = self._clock.timestamp_ns()
        cdef OrderCancelRejected cancel_rejected = OrderCancelRejected(
            trader_id=cancel.trader_id,
            strategy_id=cancel.strategy_id,
            instrument_id=cancel.instrument_id,
            client_order_id=cancel.client_order_id,
            venue_order_id=order.venue_order_id if order is not None else cancel.venue_order_id,
            account_id=order.account_id if order is not None else None,
            reason=reason,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
        )

        self._log.warning(reason)
        self._msgbus.send(endpoint="ExecEngine.process", msg=cancel_rejected)

# -- 事件处理器 -----------------------------------------------------------------------------------

    cpdef void _handle_event(self, Event event):
        if self.debug:
            self._log.debug(f"{RECV}{EVT} {event}", LogColor.MAGENTA)

        self.event_count += 1

        # A 股扩展：监听成交事件更新 T+1 账本
        if self._ashare_t1_enabled and isinstance(event, OrderFilled):
            self._update_t1_ledger(event)

        # A 股扩展：监听状态变更事件
        if isinstance(event, InstrumentStatus):
            self._handle_instrument_status(event)

    cpdef void _handle_instrument_status(self, InstrumentStatus status):
        """处理 A 股标的状态变更。"""
        if is_ashare_trading_suspended(status.action):
            self._log.info(
                f"标的 {status.instrument_id} 已停牌/临停: action={status.action}, reason={status.reason}",
                color=LogColor.RED,
            )
            # 是否需要在这里自动撤单？按需扩展。目前仅通过 _check_order 进行盘前拦截。
        elif is_ashare_trading_resumed(status.action):
            self._log.info(
                f"标的 {status.instrument_id} 已复牌: action={status.action}",
                color=LogColor.GREEN,
            )

    cpdef void _update_t1_ledger(self, OrderFilled fill):
        """根据成交回报更新 T+1 可卖账本。"""
        if self._t1_ledger is None:
            return

        acct_key = str(fill.account_id) if fill.account_id is not None else None
        inst_key = str(fill.instrument_id)
        fill_qty = fill.last_qty.as_double()
        is_buy = fill.order_side == OrderSide.BUY
        self._t1_ledger.on_fill_opt(acct_key, inst_key, is_buy, fill_qty)

    def t1_load_position(
        self,
        str account_id,
        str instrument_id,
        double total_qty,
        double today_buy_qty,
    ):
        """实盘启动时从券商加载初始持仓到 T+1 账本。

        参数
        ----------
        account_id : str
            账户 ID。
        instrument_id : str
            标的 ID。
        total_qty : double
            总持仓。
        today_buy_qty : double
            今日已买入部分（不可卖）。
        """
        if self._t1_ledger is None:
            return
        self._t1_ledger.load_position_opt(account_id, instrument_id, total_qty, today_buy_qty)

    def t1_on_settlement(self):
        """日切结算：释放今日买入量，所有持仓变为可卖。"""
        if self._t1_ledger is None:
            return
        self._t1_ledger.on_settlement()

    # -- A 股扩展辅助函数 --------------------------------------------------------------------------

    def _init_ashare_extensions(self):
        self._ashare_t1_enabled = bool(self._config.get("t1_enabled", False))
        self._ashare_session_enabled = self._config.get("session_enabled")
        self._ashare_price_tick_enabled = self._config.get("price_tick_enabled")
        self._ashare_price_limit_enabled = self._config.get("price_limit_enabled")
        self._ashare_lot_size_enabled = self._config.get("lot_size_enabled")
        self._ashare_price_cage_enabled = bool(self._config.get("price_cage_enabled", False))

        if self._ashare_session_enabled is None:
            self._ashare_session_enabled = self._ashare_t1_enabled
        else:
            self._ashare_session_enabled = bool(self._ashare_session_enabled)

        if self._ashare_price_tick_enabled is None:
            self._ashare_price_tick_enabled = self._ashare_t1_enabled
        else:
            self._ashare_price_tick_enabled = bool(self._ashare_price_tick_enabled)

        if self._ashare_price_limit_enabled is None:
            self._ashare_price_limit_enabled = self._ashare_t1_enabled
        else:
            self._ashare_price_limit_enabled = bool(self._ashare_price_limit_enabled)

        if self._ashare_lot_size_enabled is None:
            self._ashare_lot_size_enabled = self._ashare_t1_enabled
        else:
            self._ashare_lot_size_enabled = bool(self._ashare_lot_size_enabled)

        self._ashare_session_provider = None
        if self._ashare_session_enabled or self._ashare_price_cage_enabled:
            self._ashare_session_provider = AShareSessionProvider()

        self._t1_ledger = None
        if self._ashare_t1_enabled:
            self._t1_ledger = T1Ledger()

    def _check_ashare_submit_session(self):
        if self._ashare_session_enabled and self._ashare_session_provider is not None:
            phase = self._ashare_session_provider.phase_at(self._clock.timestamp_ns())
            if not can_ashare_submit_order(phase):
                return f"OUT_OF_SESSION: phase={phase}"
        return None

    def _check_ashare_cancel_session(self):
        if self._ashare_session_enabled and self._ashare_session_provider is not None:
            phase = self._ashare_session_provider.phase_at(self._clock.timestamp_ns())
            if not can_ashare_cancel_order(phase):
                return f"CANCEL_DENIED: phase={phase} does not allow cancellation"
        return None

    def _check_ashare_price_cage(self, Instrument instrument, Order order):
        cdef object quote
        cdef object trade
        cdef object best_bid
        cdef object last_trade
        cdef object tick
        cdef object pyo3_best_bid
        cdef object pyo3_last_trade
        cdef object pyo3_tick
        cdef object bound
        cdef object pyo3_order_price
        cdef float pct
        cdef object phase

        if not self._ashare_price_cage_enabled:
            return None
        if self._ashare_session_provider is None or not order.has_price_c():
            return None

        phase = self._ashare_session_provider.phase_at(self._clock.timestamp_ns())
        quote = self._cache.quote_tick(instrument.id)
        trade = self._cache.trade_tick(instrument.id)

        best_bid = quote.bid_price if quote is not None else None
        last_trade = trade.price if trade is not None else None
        tick = instrument.price_increment if instrument.price_increment is not None else None
        pct = float(self._config.get("price_cage_pct", 0.02))

        pyo3_order_price = PyO3Price(order.price.as_double(), order.price.precision)
        pyo3_best_bid = PyO3Price(best_bid.as_double(), best_bid.precision) if best_bid is not None else None
        pyo3_last_trade = PyO3Price(last_trade.as_double(), last_trade.precision) if last_trade is not None else None
        pyo3_tick = PyO3Price(tick.as_double(), tick.precision) if tick is not None else None

        bound = compute_ashare_price_cage_violation_by_phase(
            phase,
            order.is_buy_c(),
            pyo3_order_price,
            pyo3_best_bid,
            pyo3_last_trade,
            pct,
            pyo3_tick,
        )
        if bound is not None:
            return f"PRICE_CAGE_VIOLATION: price={order.price}, cage={bound}"
        return None

    def _check_ashare_lot_size(self, Instrument instrument, Order order):
        cdef object sellable = None
        cdef str acct_key
        cdef str inst_key
        cdef object violation

        if not self._ashare_lot_size_enabled:
            return None
        if instrument.lot_size is None or instrument.lot_size.raw_uint_c() == 0:
            return None

        if self._ashare_t1_enabled and order.is_sell_c() and self._t1_ledger is not None:
            acct_key = str(order.account_id) if order.account_id is not None else None
            inst_key = str(order.instrument_id)
            sellable = self._t1_ledger.sellable_or_none_opt(acct_key, inst_key)

        violation = compute_ashare_lot_size_violation(
            instrument.id.symbol.value,
            order.is_buy_c(),
            order.quantity.as_double(),
            sellable,
        )
        return violation if violation is not None else None

    def _check_ashare_price_rules(self, Instrument instrument, Price price):
        cdef object pyo3_price
        cdef object pyo3_tick_size
        cdef object pyo3_max_price
        cdef object pyo3_min_price
        cdef object trade
        cdef object quote
        cdef object pyo3_prev_close
        cdef object pyo3_tick_for_limit
        cdef object computed_limit_up
        cdef object computed_limit_down
        cdef str stock_name
        cdef object violation

        if not (self._ashare_price_tick_enabled or self._ashare_price_limit_enabled):
            return None

        pyo3_price = PyO3Price(price.as_double(), price.precision)
        pyo3_tick_size = None
        if self._ashare_price_tick_enabled and instrument.price_increment is not None:
            pyo3_tick_size = PyO3Price(
                instrument.price_increment.as_double(),
                instrument.price_increment.precision,
            )

        pyo3_max_price = None
        pyo3_min_price = None
        if self._ashare_price_limit_enabled:
            pyo3_max_price = (
                PyO3Price(instrument.max_price.as_double(), instrument.max_price.precision)
                if instrument.max_price is not None
                else None
            )
            pyo3_min_price = (
                PyO3Price(instrument.min_price.as_double(), instrument.min_price.precision)
                if instrument.min_price is not None
                else None
            )

            if pyo3_max_price is None or pyo3_min_price is None:
                pyo3_prev_close = None
                trade = self._cache.trade_tick(instrument.id)
                quote = self._cache.quote_tick(instrument.id)

                if trade is not None:
                    pyo3_prev_close = PyO3Price(trade.price.as_double(), trade.price.precision)
                elif quote is not None:
                    pyo3_prev_close = PyO3Price(
                        (quote.bid_price.as_double() + quote.ask_price.as_double()) / 2.0,
                        quote.bid_price.precision,
                    )

                if pyo3_prev_close is not None:
                    if pyo3_tick_size is not None:
                        pyo3_tick_for_limit = pyo3_tick_size
                    elif instrument.price_increment is not None:
                        pyo3_tick_for_limit = PyO3Price(
                            instrument.price_increment.as_double(),
                            instrument.price_increment.precision,
                        )
                    else:
                        pyo3_tick_for_limit = PyO3Price(10.0 ** (-price.precision), price.precision)

                    stock_name = instrument.raw_symbol.value if instrument.raw_symbol is not None else instrument.id.symbol.value
                    computed_limit_up, computed_limit_down = compute_ashare_price_limits(
                        instrument.id.symbol.value,
                        stock_name,
                        pyo3_prev_close,
                        pyo3_tick_for_limit,
                    )

                    if pyo3_max_price is None:
                        pyo3_max_price = computed_limit_up
                    if pyo3_min_price is None:
                        pyo3_min_price = computed_limit_down

        violation = compute_ashare_price_limit_violation(
            pyo3_price,
            pyo3_tick_size,
            pyo3_max_price,
            pyo3_min_price,
        )
        return violation if violation is not None else None
