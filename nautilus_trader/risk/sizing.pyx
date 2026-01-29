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

from nautilus_trader.core.correctness cimport Condition
from nautilus_trader.model.instruments.base cimport Instrument
from nautilus_trader.model.objects cimport Money
from nautilus_trader.model.objects cimport Price
from nautilus_trader.model.objects cimport Quantity


cdef class PositionSizer:
    """
    所有仓位计算器的基类。

    参数
    ----------
    instrument : Instrument
        用于仓位计算的标的。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。
    """

    def __init__(self, Instrument instrument not None):
        self.instrument = instrument

    cpdef void update_instrument(self, Instrument instrument):
        """
        使用给定的标的更新内部标的。

        参数
        ----------
        instrument : Instrument
            用于更新的标的。

        引发
        ------
        ValueError
            如果 `instrument` 与目前持有的标的不相等。

        """
        Condition.not_none(instrument, "instrument")
        Condition.equal(self.instrument.id, instrument.id, "instrument.id", "instrument.id")

        self.instrument = instrument

    cpdef Quantity calculate(
        self,
        Price entry,
        Price stop_loss,
        Money equity,
        risk: Decimal,
        commission_rate: Decimal = Decimal(0),
        exchange_rate: Decimal = Decimal(1),
        hard_limit: Decimal | None = None,
        unit_batch_size: Decimal = Decimal(1),
        int units=1,
    ):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `calculate` 必须在子类中实现")  # pragma: no cover

    cdef object _calculate_risk_ticks(self, Price entry, Price stop_loss):
        return abs(entry - stop_loss) / self.instrument.price_increment

    cdef object _calculate_riskable_money(
            self,
            equity: Decimal,
            risk: Decimal,
            commission_rate: Decimal,
    ):
        if equity <= 0:
            return Decimal(0)
        risk_money: Decimal = equity * risk
        commission: Decimal = risk_money * commission_rate * 2  # (round turn)

        return risk_money - commission


cdef class FixedRiskSizer(PositionSizer):
    """
    提供基于给定风险的仓位计算。

    参数
    ----------
    instrument : Instrument
        用于仓位计算的标的。
    """

    def __init__(self, Instrument instrument not None):
        super().__init__(instrument)

    cpdef Quantity calculate(
        self,
        Price entry,
        Price stop_loss,
        Money equity,
        risk: Decimal,
        commission_rate: Decimal = Decimal(0),
        exchange_rate: Decimal = Decimal(1),
        hard_limit: Decimal | None = None,
        unit_batch_size: Decimal=Decimal(1),
        int units=1,
    ):
        """
        计算仓位数量。

        参数
        ----------
        entry : Price
            入场价格。
        stop_loss : Price
            止损价格。
        equity : Money
            账户净值。
        risk : Decimal
            风险百分比。
        exchange_rate : Decimal
            标的报价币种与账户币种之间的汇率。
        commission_rate : Decimal
            佣金率 (>= 0)。
        hard_limit : Decimal, 可选
            总数量的硬限制 (>= 0)。
        unit_batch_size : Decimal
            单位批量大小 (> 0)。
        units : int
            将仓位分批的数量 (> 0)。

        引发
        ------
        ValueError
            如果 `risk` 不是正数 (> 0)。
        ValueError
            如果 `exchange_rate` 不是正数 (> 0)。
        ValueError
            如果 `commission_rate` 是负数 (< 0)。
        ValueError
            如果 `hard_limit` 不是 ``None`` 且不是正数 (> 0)。
        ValueError
            如果 `unit_batch_size` 不是正数 (> 0)。
        ValueError
            如果 `units` 不是正数 (> 0)。

        返回
        -------
        Quantity

        """
        Condition.not_none(equity, "equity")
        Condition.not_none(entry, "price_entry")
        Condition.not_none(stop_loss, "price_stop_loss")
        Condition.type(risk, Decimal, "risk")
        Condition.positive(risk, "risk")
        Condition.type(exchange_rate, Decimal, "exchange_rate")
        Condition.not_negative(exchange_rate, "xrate")
        Condition.type(commission_rate, Decimal, "commission_rate")
        Condition.not_negative(commission_rate, "commission_rate")
        if hard_limit is not None:
            Condition.positive(hard_limit, "hard_limit")
        Condition.type(unit_batch_size, Decimal, "unit_batch_size")
        Condition.not_negative(unit_batch_size, "unit_batch_size")
        Condition.positive_int(units, "units")

        if exchange_rate == 0:
            return self.instrument.make_qty(0)

        risk_points: Decimal = self._calculate_risk_ticks(entry, stop_loss)
        risk_money: Decimal = self._calculate_riskable_money(equity.as_decimal(), risk, commission_rate)

        if risk_points <= 0:
            # 除零保护
            return self.instrument.make_qty(0)

        # 计算仓位大小
        position_size: Decimal = ((risk_money / exchange_rate) / risk_points) / self.instrument.price_increment

        # 硬限制大小限制
        if hard_limit is not None:
            position_size = min(position_size, hard_limit)

        # 分成单位
        position_size_batched: Decimal = max(Decimal(0), position_size / units)

        if unit_batch_size > 0:
            # 将仓位大小舍入到最近的单位批量大小
            position_size_batched = (position_size_batched // unit_batch_size) * unit_batch_size

        # 最大交易量限制（如果已配置）
        if self.instrument.max_quantity is not None:
            final_size: Decimal = min(
                position_size_batched,
                self.instrument.max_quantity.as_decimal(),
            )
        else:
            final_size: Decimal = position_size_batched

        return self.instrument.make_qty(final_size)
