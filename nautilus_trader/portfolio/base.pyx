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

from nautilus_trader.accounting.accounts.base cimport Account
from nautilus_trader.model.identifiers cimport AccountId
from nautilus_trader.model.identifiers cimport InstrumentId
from nautilus_trader.model.identifiers cimport Venue
from nautilus_trader.model.objects cimport Money
from nautilus_trader.model.objects cimport Price


cdef class PortfolioFacade:
    """
    提供 `Portfolio` 的只读门面 (facade)。
    """

# -- 查询 --------------------------------------------------------------------------------------

    cpdef Account account(self, Venue venue=None, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `account` 必须在子类中实现")  # pragma: no cover

    cpdef dict balances_locked(self, Venue venue=None, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `balances_locked` 必须在子类中实现")  # pragma: no cover

    cpdef dict margins_init(self, Venue venue=None, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `margins_init` 必须在子类中实现")  # pragma: no cover

    cpdef dict margins_maint(self, Venue venue=None, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `margins_maint` 必须在子类中实现")  # pragma: no cover

    cpdef dict realized_pnls(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `realized_pnls` 必须在子类中实现")  # pragma: no cover

    cpdef dict unrealized_pnls(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `unrealized_pnls` 必须在子类中实现")  # pragma: no cover

    cpdef dict total_pnls(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `total_pnls` 必须在子类中实现")  # pragma: no cover

    cpdef dict net_exposures(self, Venue venue=None, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `net_exposures` 必须在子类中实现")  # pragma: no cover

    cpdef Money realized_pnl(self, InstrumentId instrument_id, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `realized_pnl` 必须在子类中实现")  # pragma: no cover

    cpdef Money unrealized_pnl(self, InstrumentId instrument_id, Price price=None, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `unrealized_pnl` 必须在子类中实现")  # pragma: no cover

    cpdef Money total_pnl(self, InstrumentId instrument_id, Price price=None, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `total_pnl` 必须在子类中实现")  # pragma: no cover

    cpdef Money net_exposure(self, InstrumentId instrument_id, Price price=None, AccountId account_id=None, Currency target_currency=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `net_exposure` 必须在子类中实现")  # pragma: no cover

    cpdef object net_position(self, InstrumentId instrument_id, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `net_position` 必须在子类中实现")  # pragma: no cover

    cpdef bint is_net_long(self, InstrumentId instrument_id, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `is_net_long` 必须在子类中实现")  # pragma: no cover

    cpdef bint is_net_short(self, InstrumentId instrument_id, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `is_net_short` 必须在子类中实现")  # pragma: no cover

    cpdef bint is_flat(self, InstrumentId instrument_id, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `is_flat` 必须在子类中实现")  # pragma: no cover

    cpdef bint is_completely_flat(self, AccountId account_id=None):
        """抽象方法（在子类中实现）。"""
        raise NotImplementedError("方法 `is_completely_flat` 必须在子类中实现")  # pragma: no cover
