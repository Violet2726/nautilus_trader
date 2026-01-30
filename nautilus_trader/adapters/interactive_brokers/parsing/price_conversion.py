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
Interactive Brokers 适配器的价格转换实用程序。
 
Interactive Brokers 在合约详情中使用价格乘数 (price magnifier) 字段来缩放价格。
所有从 IB 接收的价格都需要除以价格乘数才能得到真实价格。
所有发送到 IB 的价格都需要乘以价格乘数。
 
"""

from nautilus_trader.model.identifiers import InstrumentId


def ib_price_to_nautilus_price(ib_price: float, price_magnifier: int) -> float:
    """
    将 Interactive Brokers 的价格转换为 Nautilus 的价格。
 
    Parameters
    ----------
    ib_price : float
        从 Interactive Brokers 接收的价格。
    price_magnifier : int
        合约详情中的价格乘数。
 
    Returns
    -------
    float
        用于 Nautilus 的真实价格。
 
    """
    if price_magnifier <= 0:
        return ib_price

    return ib_price / price_magnifier


def nautilus_price_to_ib_price(nautilus_price: float, price_magnifier: int) -> float:
    """
    将 Nautilus 的价格转换为 Interactive Brokers 的价格。
 
    Parameters
    ----------
    nautilus_price : float
        要发送到 Interactive Brokers 的 Nautilus 价格。
    price_magnifier : int
        合约详情中的价格乘数。
 
    Returns
    -------
    float
        发送到 Interactive Brokers 的缩放后价格。
 
    """
    if price_magnifier <= 0:
        return nautilus_price

    return nautilus_price * price_magnifier


def get_price_magnifier_for_instrument(
    instrument_id: InstrumentId,
    instrument_provider,
) -> int:
    """
    获取工具的价格乘数。
 
    Parameters
    ----------
    instrument_id : InstrumentId
        工具标识符。
    instrument_provider : InteractiveBrokersInstrumentProvider | None
        用于获取合约详情的工具提供者。
 
    Returns
    -------
    int
        价格乘数，如果未找到或提供者为 None，则默认为 1。
 
    """
    if instrument_provider is None:
        return 1

    return instrument_provider.get_price_magnifier(instrument_id)
