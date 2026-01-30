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
from typing import Final
from typing import Literal

from ibapi.const import UNSET_DECIMAL
from ibapi.contract import FundAssetType
from ibapi.contract import FundDistributionPolicyIndicator
from ibapi.tag_value import TagValue

from nautilus_trader.config import NautilusConfig
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import Venue


IB: Final[str] = "INTERACTIVE_BROKERS"
IB_VENUE: Final[Venue] = Venue(IB)
IB_CLIENT_ID: Final[ClientId] = ClientId(IB)


class ContractId(int):
    """
    ContractId（合约 ID）类型。
    """


# https://interactivebrokers.github.io/tws-api/tick_types.html
TickTypeMapping = {
    0: "Bid Size",
    1: "Bid Price",
    2: "Ask Price",
    3: "Ask Size",
    4: "Last Price",
    5: "Last Size",
    6: "High",
    7: "Low",
    8: "Volume",
    9: "Close Price",
}


class ComboLeg(NautilusConfig, frozen=True, omit_defaults=True, repr_omit_defaults=True):
    """
    表示组合订单（combo orders）中一条腿（leg）的类。
    """

    conId: int = 0
    ratio: int = 0
    action: str = ""  # Literal["BUY", "SELL"]
    exchange: str = ""
    openClose: int = 0  # LegOpenClose enum values
    # 用于做空时的股票腿
    shortSaleSlot: int = 0
    designatedLocation: str = ""
    exemptCode: int = -1


class DeltaNeutralContract(NautilusConfig, frozen=True, repr_omit_defaults=True):
    """
    Delta 中性（Delta-Neutral）合约。
    """

    conId: int = 0
    delta: float = 0.0
    price: float = 0.0


class IBContract(NautilusConfig, frozen=True, repr_omit_defaults=True):
    """
    描述工具定义的类，包含期权/期货的额外字段。

    参数
    ----------
    secType: str
        合约的安全类型（Security Type），例如 STK, OPT, FUT, CONTFUT。
    exchange: str
        工具交易的交易所。对于股票，通常为 SMART。
    primaryExchange: str
        工具注册的交易所。适用于股票。
    symbol: str
        在交易所注册的唯一代码。
    build_options_chain: bool (默认: None)
        是否搜索完整的期权链。
    build_futures_chain: bool (默认: None)
        是否搜索完整的期货链。
    options_chain_exchange: str (默认: None)
        期权链的可选交易所，用于替换底层工具的交易所。
    min_expiry_days: int (默认: None)
        过滤到期天数不少于指定天数的期权链和期货链。
    max_expiry_days: int (默认: None)
        过滤到期天数不多于指定天数的期权链和期货链。
    lastTradeDateOrContractMonth: str (%Y%m%d 或 %Y%m) (默认: '')
        过滤特定到期日期的期权链和期货链。
    lastTradeDate: str (默认: '')
        合约的最后交易日。

    """

    secType: Literal[
        "CASH",
        "STK",
        "OPT",
        "FUT",
        "FOP",
        "CONTFUT",
        "CRYPTO",
        "CFD",
        "CMDTY",
        "IND",
        "BAG",
        "",
    ] = ""
    conId: int = 0
    exchange: str = ""
    primaryExchange: str = ""
    symbol: str = ""
    localSymbol: str = ""
    currency: str = ""
    tradingClass: str = ""

    # 期权与期货 (Options and Futures)
    lastTradeDateOrContractMonth: str = ""
    lastTradeDate: str = ""
    multiplier: str = ""

    # 期权 (Options)
    strike: float | str = ""
    right: str = ""

    # 如果设置为 True，则可以进行与已过期期货合约相关的合约详情请求和历史数据查询。
    # 已过期的期权或其他工具类型不可用。
    includeExpired: bool = False

    # 通用 (Common)
    secIdType: str = ""
    secId: str = ""
    description: str = ""
    issuerId: str = ""

    # 组合 (Combos)
    comboLegsDescrip: str = ""
    comboLegs: list[ComboLeg] | None = None
    deltaNeutralContract: DeltaNeutralContract | None = None

    # Nautilus 特定参数 (Nautilus specific parameters)
    build_futures_chain: bool | None = None
    build_options_chain: bool | None = None
    options_chain_exchange: str | None = None
    min_expiry_days: int | None = None
    max_expiry_days: int | None = None


class IBOrderTags(NautilusConfig, frozen=True, repr_omit_defaults=True):
    """
    用于附加到 Nautilus 订单标签（Order Tags），以包含 IB 特定的订单参数。
    """

    # 包含佣金的盘前（Pre-order）和盘后（post-order）保证金分析
    whatIf: bool = False

    # 订单组（Order Group）条件 (One)
    ocaGroup: str = ""  # OCA（One Cancels All）组名
    ocaType: int = 0  # 1 = CANCEL_WITH_BLOCK, 2 = REDUCE_WITH_BLOCK, 3 = REDUCE_NON_BLOCK

    # 订单组条件 (All)
    allOrNone: bool = False

    # 时间条件
    activeStartTime: str = ""  # 用于 GTC 订单，格式："%Y%m%d %H:%M:%S %Z"
    activeStopTime: str = ""  # 用于 GTC 订单，格式："%Y%m%d %H:%M:%S %Z"
    goodAfterTime: str = ""  # 格式："%Y%m%d %H:%M:%S %Z"

    # 扩展订单字段
    blockOrder = False  # 如果设置为 True，指定该订单为 ISE Block 订单。
    sweepToFill = False
    outsideRth: bool = False

    # 如果设置为 True，在查看市场深度时，该订单将不可见。
    # 此选项仅适用于路由到 NASDAQ 交易所的订单。
    hidden: bool = False

    # 订单条件
    conditions: list[dict] = []  # 条件字典列表
    conditionsCancelOrder: bool = (
        False  # True = 当条件满足时取消订单, False = 传输订单
    )

    # 智能组合路由参数（用于组合订单）
    NonGuaranteed: bool = False  # True = 非保证组合订单, False = 保证组合订单

    @property
    def value(self):
        return f"IBOrderTags:{self.json().decode()}"

    def __str__(self):
        return self.value


class IBContractDetails(NautilusConfig, frozen=True, repr_omit_defaults=True):
    """
    ContractDetails 类，在 Nautilus 内部使用，以便于编码/解码。

    参考资料：https://ibkrcampus.com/campus/ibkr-api-page/twsapi-ref/#contract-pub-func

    """

    contract: IBContract | None = None
    marketName: str = ""
    minTick: float = 0
    orderTypes: str = ""
    validExchanges: str = ""
    priceMagnifier: int = 1
    underConId: int = 0
    longName: str = ""
    contractMonth: str = ""
    industry: str = ""
    category: str = ""
    subcategory: str = ""
    timeZoneId: str = ""
    tradingHours: str = ""
    liquidHours: str = ""
    evRule: str = ""
    evMultiplier: float = 0
    mdSizeMultiplier: int = 1  # 已废弃
    aggGroup: int = 0
    underSymbol: str = ""
    underSecType: str = ""
    marketRuleIds: str = ""
    secIdList: list[TagValue] | None = None
    realExpirationDate: str = ""
    lastTradeTime: str = ""
    stockType: str = ""
    minSize: Decimal = UNSET_DECIMAL
    sizeIncrement: Decimal = UNSET_DECIMAL
    suggestedSizeIncrement: Decimal = UNSET_DECIMAL

    # 债券 (BOND) 数值
    cusip: str = ""
    ratings: str = ""
    descAppend: str = ""
    bondType: str = ""
    couponType: str = ""
    callable: bool = False
    putable: bool = False
    coupon: float = 0
    convertible: bool = False
    maturity: str = ""
    issueDate: str = ""
    nextOptionDate: str = ""
    nextOptionType: str = ""
    nextOptionPartial: bool = False
    notes: str = ""

    # 基金 (FUND) 数值
    fundName: str = ""
    fundFamily: str = ""
    fundType: str = ""
    fundFrontLoad: str = ""
    fundBackLoad: str = ""
    fundBackLoadTimeInterval: str = ""
    fundManagementFee: str = ""
    fundClosed: bool = False
    fundClosedForNewInvestors: bool = False
    fundClosedForNewMoney: bool = False
    fundNotifyAmount: str = ""
    fundMinimumInitialPurchase: str = ""
    fundSubsequentMinimumPurchase: str = ""
    fundBlueSkyStates: str = ""
    fundBlueSkyTerritories: str = ""
    fundDistributionPolicyIndicator: FundDistributionPolicyIndicator = (
        FundDistributionPolicyIndicator.NoneItem
    )
    fundAssetType: FundAssetType = FundAssetType.NoneItem
    ineligibilityReasonList: list = None


def dict_to_contract_details(dict_details: dict) -> IBContractDetails:
    details_copy = dict_details.copy()

    if "contract" in details_copy and isinstance(details_copy["contract"], dict):
        details_copy["contract"] = IBContract(**details_copy["contract"])

    if details_copy.get("secIdList") and isinstance(details_copy["secIdList"], dict):
        tag_values = [
            TagValue(tag=tag, value=value) for tag, value in details_copy["secIdList"].items()
        ]
        details_copy["secIdList"] = tag_values

    # 将 Decimal 字段从字符串反序列化回 Decimal 对象。
    # 在 IBContractDetails 中已知这些字段为 Decimal 类型。
    decimal_fields = ["minSize", "sizeIncrement", "suggestedSizeIncrement"]
    for field in decimal_fields:
        if field in details_copy and isinstance(details_copy[field], str):
            try:
                decimal_value = Decimal(details_copy[field])

                # 检查这是否是 UNSET_DECIMAL 值
                if decimal_value == UNSET_DECIMAL:
                    details_copy[field] = UNSET_DECIMAL
                else:
                    details_copy[field] = decimal_value
            except (ValueError, TypeError):
                # 如果转换失败，保留原始值
                pass

    # 将 Enum 字段从它们的值反序列化回 Enum 成员。
    # 在 IBContractDetails 中已知这些字段为 Enum 类型。
    if "fundDistributionPolicyIndicator" in details_copy:
        details_copy["fundDistributionPolicyIndicator"] = _deserialize_enum_from_value(
            FundDistributionPolicyIndicator,
            details_copy["fundDistributionPolicyIndicator"],
        )

    if "fundAssetType" in details_copy:
        details_copy["fundAssetType"] = _deserialize_enum_from_value(
            FundAssetType,
            details_copy["fundAssetType"],
        )

    return IBContractDetails(**details_copy)


def _deserialize_enum_from_value(enum_class, value):
    """
    将枚举值（元组或字符串）转换回枚举成员。
    """
    if value is None:
        return None
 
    # 如果已经是枚举成员，则按原样返回
    if isinstance(value, enum_class):
        return value
 
    # 尝试通过匹配值来查找枚举成员
    for member in enum_class:
        if member.value == value:
            return member
 
    # 如果未找到，返回原始值（可能无效）
    return value
