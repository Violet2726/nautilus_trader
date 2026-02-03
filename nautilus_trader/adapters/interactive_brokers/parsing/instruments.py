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
import time
from decimal import Decimal
from enum import Enum
from typing import Any
from typing import cast

import pandas as pd
from ibapi.contract import ContractDetails

from nautilus_trader.adapters.interactive_brokers.common import ComboLeg
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.common import IBContractDetails
from nautilus_trader.adapters.interactive_brokers.config import SymbologyMethod
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.enums import OptionKind
from nautilus_trader.model.enums import asset_class_from_str
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.identifiers import generic_spread_id_to_list
from nautilus_trader.model.identifiers import is_generic_spread_id
from nautilus_trader.model.identifiers import new_generic_spread_id
from nautilus_trader.model.instruments import Cfd
from nautilus_trader.model.instruments import Commodity
from nautilus_trader.model.instruments import CryptoPerpetual
from nautilus_trader.model.instruments import CurrencyPair
from nautilus_trader.model.instruments import Equity
from nautilus_trader.model.instruments import FuturesContract
from nautilus_trader.model.instruments import FuturesSpread
from nautilus_trader.model.instruments import IndexInstrument
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.instruments import OptionContract
from nautilus_trader.model.instruments.option_spread import OptionSpread
from nautilus_trader.model.objects import Currency
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity


VENUE_MEMBERS: dict[str, list[str]] = {
    # ICE Endex 交易所
    "NDEX": ["ENDEX"],  # ICE Endex 交易所
    # CME Group Exchanges - Includes related index exchanges
    "XCME": [
        "CME",
    ],  # 芝加哥商品交易所 (芝商所) (场内/ClearPort 可能会使用此代码；用于 ES, RTY, NKD 期货等)
    "XCEC": ["CME"],  # CME 加密货币 (与 CME 相关)
    "XFXS": ["CME"],  # CME 外汇链接, 外汇现货 (与 CME 相关)
    # Chicago Board of Trade Segments
    "XCBT": [
        "CBOT",
    ],  # 芝加哥期货交易所 (场内/ClearPort 可能会使用此代码；用于 ZN, ZB, ZS 期货等)
    "CBCM": ["CBOT"],  # CBOT 大宗商品 (特定细分市场，与 CBOT 相关)
    # New York Mercantile Exchange Segments
    "XNYM": [
        "NYMEX",
    ],  # 纽约商业交易所 (场内/ClearPort 可能会使用此代码；用于 CL, NG 期货等)
    "NYUM": ["NYMEX"],  # NYMEX 金属 (特定细分市场，与 NYMEX 相关)
    # ICE 美国期货交易所 (前身为 NYBOT)
    "IFUS": ["NYBOT"],  # ICE 美国期货交易所 (IBKR 对此使用 NYBOT 代码；用于 CC, KC, SB 期货等)
    # GLBX，databento 使用的名称
    "GLBX": [
        "CBOT",
        "CME",
        "NYBOT",
        "NYMEX",
    ],  # CME 集团 Globex (这些交易所进行电子交易的父级 MIC)
    # US Major Exchanges & Index Venues
    "XNAS": ["NASDAQ"],  # 纳斯达克证券市场 (用于 IXIC, NDX 指数)
    "XNYS": ["NYSE"],  # 纽约证券交易所 (用于 NYA 指数)
    "ARCX": ["ARCA"],  # 纽约证券交易所 Arca (NYSE Arca)
    "BATS": ["BATS"],  # Cboe BZX 美国交易所 (前身为 BATS)
    "IEXG": ["IEX"],  # Investors Exchange (IEX 交易所)
    "XCBO": [
        "CBOE",
    ],  # Cboe 期权交易所 (用于 SPX, RUT 期权及如 SPX.IND, RUT.IND 等指数)
    "XCBF": ["CFE"],  # Cboe 期货交易所 (IBKR 使用 CFE，例如用于 VIX 期货)
    # 加拿大交易所
    "XTSE": ["TSX"],  # 多伦多证券交易所 (用于 GSPTSE 指数)
    # ICE 欧洲交易所
    "IFEU": [
        "ICEEU",
        "ICEEUSOFT",
        "IPE",
    ],  # ICE 欧洲期货交易所 (IBKR 对此交易所的产品使用以下代码：ICEEU (通用), ICEEUSOFT (软商品), IPE (能源))
    # 欧洲交易所
    "XLON": ["LSE"],  # 伦敦证券交易所 (用于 UKX, FTMC 指数)
    "XPAR": ["SBF"],  # 泛欧交易所巴黎分部 (IBKR 使用 SBF) (用于 FCHI 指数)
    "XETR": ["IBIS"],  # 德意志交易所 Xetra (IBKR 对 Xetra 使用 IBIS 代码)
    "XEUR": [
        "DTB",
        "EUREX",
        "SOFFEX",
    ],  # Eurex 交易所 (IBKR 使用 SOFFEX/DTB/EUREX 代码；SOFFEX 是前身。用于 STOXX50E, GDAXI 衍生品及指数参考)
    "XAMS": ["AEB"],  # 泛欧交易所阿姆斯特丹分部 (IBKR 使用 AEB) (用于 AEX 指数)
    "XBRU": ["EBS"],  # 泛欧交易所布鲁塞尔股票分部 (IBKR 使用 EBS)
    "XBRD": [
        "BELFOX",
    ],  # 泛欧交易所布鲁塞尔衍生品分部 (IBKR 使用 BELFOX) - XBRD 是 "泛欧交易所布鲁塞尔 - 衍生品市场" 的 MIC 代码
    "XLIS": ["BVLP"],  # 泛欧交易所里斯本分部 (IBKR 使用 BVLP)
    "XDUB": ["IRE"],  # 泛欧交易所都柏林分部 (IBKR 使用 IRE)
    "XOSL": ["OSL"],  # 泛欧交易所奥斯陆分部 (奥斯陆证券交易所) (IBKR 使用 OSL)
    "XSWX": [
        "EBS",
        "SIX",
        "SWX",
    ],  # 瑞士证券交易所 (IBKR 对股票使用 SWX 代码；EBS 是旧的 IBKR 代码。用于 SSMI 指数)
    "XSVX": [
        "VRTX",
    ],  # 瑞士证券交易所衍生品分部 (IBKR 使用 VRTX) - XSVX 是 "瑞士证券交易所 - 衍生品市场" 的 MIC 代码
    "XMIL": [
        "BIT",
        "BVME",
        "IDEM",
    ],  # 意大利证券交易所 (泛欧交易所米兰分部) (IBKR 使用 BIT 代码；BVME 用于股票，IDEM 用于衍生品。用于 FTMIB 指数)
    "XMAD": [
        "MDRD",
        "BME",
    ],  # 西班牙证券交易所 (BME) - 马德里 (IBKR 使用 MDRD 代码；也使用 BME。用于 IBEX 指数)
    "DXEX": [
        "BATEEN",
    ],  # Cboe 欧洲股票 - 荷兰 (IBKR 使用 BATEEN 代码) - DXEX 是 Cboe NL 的 MIC 代码 (英国脱欧后 Cboe 欧洲的主要交易场所)
    "XSWX": [
        "EBS",
        "SIX",
        "SWX",
    ],  # 瑞士证券交易所 (IBKR 对股票使用 SWX 代码；EBS 是旧的 IBKR 代码。用于 SSMI 指数)
    "XSVX": [
        "VRTX",
    ],  # 瑞士证券交易所衍生品分部 (IBKR 使用 VRTX) - XSVX 是 "瑞士证券交易所 - 衍生品市场" 的 MIC 代码
    "XMIL": [
        "BIT",
        "BVME",
        "IDEM",
    ],  # 意大利证券交易所 (泛欧交易所米兰分部) (IBKR 使用 BIT 代码；BVME 用于股票，IDEM 用于衍生品。用于 FTMIB 指数)
    "XMAD": [
        "MDRD",
        "BME",
    ],  # 西班牙证券交易所 (BME) - 马德里 (IBKR 使用 MDRD 代码；也使用 BME。用于 IBEX 指数)
    "DXEX": [
        "BATEEN",
    ],  # Cboe 欧洲股票 - 荷兰 (IBKR 使用 BATEEN 代码) - DXEX 是 Cboe NL 的 MIC 代码 (英国脱欧后 Cboe 欧洲的主要交易场所)
    "XWBO": ["WBAG"],  # 维也纳证券交易所 (IBKR 使用 WBAG)
    "XBUD": ["BUX"],  # 布达佩斯证券交易所 (IBKR 使用 BUX)
    "XPRA": ["PRA"],  # 布拉格证券交易所 (IBKR 使用 PRA)
    "XWAR": ["WSE"],  # 华沙证券交易所 (IBKR 使用 WSE)
    "XIST": ["ISE"],  # 伊斯坦布尔证券交易所 (IBKR 通常对伊斯坦布尔证券交易所的股票使用 ISE 代码)
    # Nasdaq Nordic Exchanges
    "XSTO": ["SFB"],  # 纳斯达克斯德哥尔摩交易所 (IBKR 使用 SFB)
    "XCSE": ["KFB"],  # 纳斯达克哥本哈根交易所 (IBKR 使用 KFB)
    "XHEL": ["HMB"],  # 纳斯达克赫尔辛基交易所 (IBKR 使用 HMB)
    "XICE": ["ISB"],  # 纳斯达克冰岛交易所 (IBKR 使用 ISB)
    # Asia-Pacific Exchanges
    "XASX": ["ASX"],  # 澳大利亚证券交易所 (用于 S&P/ASX 200 - AXJO 指数)
    "XHKG": ["SEHK"],  # 香港证券交易所 (股票) (IBKR 使用 SEHK)
    "XHKF": ["HKFE"],  # 香港期货交易所 (用于恒生指数衍生品及指数参考)
    "XSES": ["SGX"],  # 新加坡交易所 (用于 STI 指数和一些国际衍生品)
    "XOSE": [
        "OSE.JPN",
    ],  # 大阪交易所 (IBKR 使用 OSE.JPN) (用于 N225 衍生品及指数参考)
    "XTKS": [
        "TSEJ",
        "TSE.JPN",
    ],  # 东京证券交易所 (IBKR 对股票使用 TSEJ 代码；对 TOPX 指数使用 TSE.JPN)
    "XKRX": [
        "KSE",
        "KRX",
    ],  # 韩国交易所 (IBKR 对股票使用 KSE 代码；KRX 与 KOSPI - KS11 指数相关)
    "XTAI": [
        "TASE",
        "TWSE",
    ],  # 台湾证券交易所 (MIC XTAI。IBKR 对台湾股票使用 TASE 代码；TWSE 也与
    # TAIEX - TWII 指数相关。注意：XTAE 是特拉维夫的 MIC 代码)
    "XSHG": [
        "SEHKNTL",
        "SSE",
    ],  # 上海证券交易所 (IBKR 对沪股通使用 SEHKNTL；对上证综指 SSEC 指数直接参考使用 SSE)
    "XSHE": ["SEHKSZSE"],  # 深圳证券交易所 (深股通) (IBKR 使用 SEHKSZSE)
    "XNSE": ["NSE"],  # 印度国家证券交易所 (IBKR 使用 NSE) (用于 NIFTY 50 - NSEI 指数)
    "XBOM": ["BSE"],  # 孟买证券交易所 (IBKR 使用 BSE) (用于 SENSEX - BSESN 指数)
    # 其他衍生品交易所
    "XSFE": ["SNFE"],  # 悉尼期货交易所 (现为 ASX 24，IBKR 使用 SNFE)
    "XMEX": ["MEXDER"],  # 墨西哥衍生品交易所
    # 非洲、中东、南美交易所
    "XJSE": [
        "JSE",
    ],  # 约翰内斯堡证券交易所 (IBKR 使用 JSE) (用于 FTSE/JSE All Share - JALSH 指数)
    "XBOG": ["BVC"],  # 哥伦比亚证券交易所 (IBKR 使用 BVC)
    "XTAE": [
        "TASE",  # 特拉维夫证券交易所 (MIC XTAE。IBKR 对特拉维夫股票使用 TASE 代码；注意 XTAI 是台湾)
    ],
    "BVMF": [
        "BVMF",
    ],  # B3 - 巴西证券交易所 (IBKR 使用 BVMF；用于 IBOVESPA - BVSP 指数。BVMF 也是 MIC 代码)
}

FUTURES_MONTH_TO_CODE: dict[str, str] = {
    "JAN": "F",
    "FEB": "G",
    "MAR": "H",
    "APR": "J",
    "MAY": "K",
    "JUN": "M",
    "JUL": "N",
    "AUG": "Q",
    "SEP": "U",
    "OCT": "V",
    "NOV": "X",
    "DEC": "Z",
}
FUTURES_CODE_TO_MONTH = dict(
    zip(FUTURES_MONTH_TO_CODE.values(), FUTURES_MONTH_TO_CODE.keys(), strict=False),
)

VENUES_FUT = [
    "BELFOX",  # BE
    "CBOT",  # US
    "CFE",  # US
    "CME",  # US
    "COMEX",  # US
    "DTB",  # DE
    "EUREX",  # EU
    "HKFE",  # HK
    "ICEEU",  # UK
    "ICEEUSOFT",  # UK
    "IDEM",  # IT
    "IPE",  # UK
    "KCBT",  # US
    "MEXDER",  # MX
    "MGE",  # US
    "NYBOT",  # US
    "NYMEX",  # US
    "OSE.JPN",  # JP
    "SNFE",  # AU
    "SOFFEX",  # CH
    "VRTX",  # 全球 (Global)
]
VENUES_CASH = ["IDEALPRO"]
VENUES_CRYPTO = ["PAXOS"]
VENUES_OPT = ["SMART"]
VENUES_CFD = ["IBCFD"]  # 自定义名称，解析时实际上映射到 "SMART"
VENUES_CMDTY = ["IBCMDTY"]  # 自定义名称，解析时实际上映射到 "SMART"

RE_CASH = re.compile(r"^(?P<symbol>[A-Z]{3})\/(?P<currency>[A-Z]{3})$")  # "EUR/USD"
RE_CFD_CASH = re.compile(r"^(?P<symbol>[A-Z]{3})\.(?P<currency>[A-Z]{3})$")  # "EUR.USD"
RE_OPT = re.compile(
    r"^(?P<symbol>[A-Z.]{1,6}) *(?P<expiry>\d{6})(?P<right>[CP])(?P<strike>\d{5})(?P<decimal>\d{3})$",
)  # "AAPL220617C00155000" or "SPXW  260120P06835000" (OCC format with padding)
RE_FUT_UNDERLYING = re.compile(r"^(?P<symbol>\w{1,3})$")  # "ES"
RE_FUT = re.compile(r"^(?P<symbol>\w{1,3})(?P<month>[FGHJKMNQUVXZ])(?P<year>\d{2})$")  # "ESM23"
RE_FUT_ORIGINAL = re.compile(
    r"^(?P<symbol>\w{1,3})(?P<month>[FGHJKMNQUVXZ])(?P<year>\d)$",
)  # "ESM3"
RE_FUT2 = re.compile(
    r"^(?P<symbol>\w{1,4})(?P<month>(JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC))(?P<year>\d{2})$",
)  # "ESMAR23"
RE_FUT2_ORIGINAL = re.compile(
    r"^(?P<symbol>\w{1,4}) *(?P<month>(JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC)) (?P<year>\d{2})$",
)  # "ES MAR 23"
RE_FUT3_ORIGINAL = re.compile(
    r"^(?P<symbol>[A-Z]+)(?P<year>\d{2})(?P<month>(JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC))FUT$",
)  # "NIFTY25MARFUT"
RE_FOP = re.compile(
    r"^(?P<symbol>\w{1,3})(?P<month>[FGHJKMNQUVXZ])(?P<year>\d{2})(?P<right>[CP])(?P<strike>.{4,6})$",
)  # "ESM23C4200"
RE_FOP_ORIGINAL = re.compile(
    r"^(?P<symbol>\w{1,3})(?P<month>[FGHJKMNQUVXZ])(?P<year>\d)\s(?P<right>[CP])(?P<strike>\d{1,6}(?:\.\d+)?)$",
)  # "ESM3 C4420"
RE_CRYPTO = re.compile(r"^(?P<symbol>[A-Z]*)\/(?P<currency>[A-Z]{3})$")  # "BTC/USD"


def sec_type_to_asset_class(sec_type: str) -> AssetClass:
    mapping = {
        "STK": "EQUITY",
        "IND": "INDEX",
        "CASH": "FX",
        "BOND": "DEBT",
        "CMDTY": "COMMODITY",
        "FUT": "INDEX",
    }

    # 处理空或 None 的 sec_type
    if not sec_type:
        return AssetClass.EQUITY  # 默认为 EQUITY (股票)

    mapped_value = mapping.get(sec_type, sec_type)
    # 如果映射后的值仍不是有效的 AssetClass，则默认为 EQUITY
    try:
        return asset_class_from_str(mapped_value)
    except Exception:
        return AssetClass.EQUITY


def contract_details_to_ib_contract_details(contract_details: ContractDetails) -> IBContractDetails:
    contract_details.contract = IBContract(**contract_details.contract.__dict__)
    contract_details = IBContractDetails(**contract_details.__dict__)

    return contract_details


def parse_instrument(  # noqa: C901
    contract_details: IBContractDetails,
    venue: str,
    symbology_method: SymbologyMethod = SymbologyMethod.IB_SIMPLIFIED,
    contract_details_map: dict[int, IBContractDetails] | None = None,
) -> Instrument:
    security_type = contract_details.contract.secType
    instrument_id = ib_contract_to_instrument_id(
        contract_details.contract,
        venue,
        symbology_method,
        contract_details_map,
    )

    if security_type == "STK":
        return parse_equity_contract(contract_details=contract_details, instrument_id=instrument_id)
    elif security_type == "IND":
        return parse_index_contract(contract_details=contract_details, instrument_id=instrument_id)
    elif security_type in ("FUT", "CONTFUT"):
        return parse_futures_contract(
            contract_details=contract_details,
            instrument_id=instrument_id,
        )
    elif security_type in ("OPT", "FOP"):
        return parse_option_contract(contract_details=contract_details, instrument_id=instrument_id)
    elif security_type == "CASH":
        return parse_forex_contract(contract_details=contract_details, instrument_id=instrument_id)
    elif security_type == "CRYPTO":
        return parse_crypto_contract(contract_details=contract_details, instrument_id=instrument_id)
    elif security_type == "CFD":
        return parse_cfd_contract(contract_details=contract_details, instrument_id=instrument_id)
    elif security_type == "CMDTY":
        return parse_commodity_contract(
            contract_details=contract_details,
            instrument_id=instrument_id,
        )
    elif security_type == "BAG":
        if _has_futures(contract_details.contract, contract_details_map):
            return parse_futures_spread(
                contract_details=contract_details,
                instrument_id=instrument_id,
            )
        else:
            return parse_option_spread(
                contract_details=contract_details,
                instrument_id=instrument_id,
            )
    else:
        raise ValueError(f"未知的 {security_type=}")


def parse_equity_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> Equity:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    timestamp = time.time_ns()

    return Equity(
        instrument_id=instrument_id,
        raw_symbol=Symbol(contract_details.contract.localSymbol),
        currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        lot_size=Quantity.from_int(100),
        isin=_extract_isin(contract_details),
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def _extract_isin(contract_details: IBContractDetails) -> int:
    if contract_details.secIdList:
        for tag_value in contract_details.secIdList:
            if tag_value.tag == "ISIN":
                return tag_value.value

    raise ValueError("未找到 ISIN")


def parse_index_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> IndexInstrument:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    size_precision: int = _tick_size_to_precision(contract_details.minSize)
    timestamp = time.time_ns()

    return IndexInstrument(
        instrument_id=instrument_id,
        raw_symbol=Symbol(contract_details.contract.localSymbol),
        currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        size_precision=size_precision,
        size_increment=Quantity(contract_details.sizeIncrement, size_precision),
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def parse_futures_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> FuturesContract:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    timestamp = time.time_ns()
    expiration = expiry_timestring_to_datetime(contract_details)
    activation = expiration - pd.Timedelta(days=90)  # 待办：使其更准确
    raw_symbol = (
        contract_details.contract.localSymbol
        if contract_details.contract.secType == "FUT"
        else contract_details.contract.symbol
    )  # 对于 CONTFUT 的证券代码

    return FuturesContract(
        instrument_id=instrument_id,
        raw_symbol=Symbol(raw_symbol),
        asset_class=sec_type_to_asset_class(contract_details.underSecType),
        currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        multiplier=Quantity.from_str(contract_details.contract.multiplier),
        lot_size=Quantity.from_int(1),
        underlying=contract_details.underSymbol,
        activation_ns=activation.value,
        expiration_ns=expiration.value,
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def parse_option_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> OptionContract:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    timestamp = time.time_ns()
    asset_class = sec_type_to_asset_class(contract_details.underSecType)
    option_kind = {
        "C": OptionKind.CALL,
        "P": OptionKind.PUT,
    }[contract_details.contract.right]
    expiration = expiry_timestring_to_datetime(contract_details)
    activation = expiration - pd.Timedelta(days=90)  # 待办：使其更准确

    # 对于期权，乘数代表手数 (例如，每张合约 100 股)
    multiplier = Quantity.from_str(contract_details.contract.multiplier)

    return OptionContract(
        instrument_id=instrument_id,
        raw_symbol=Symbol(contract_details.contract.localSymbol),
        asset_class=asset_class,
        currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        multiplier=multiplier,
        lot_size=multiplier,  # 对于期权，手数等于乘数
        underlying=contract_details.underSymbol,
        strike_price=Price(contract_details.contract.strike, price_precision),
        activation_ns=activation.value,
        expiration_ns=expiration.value,
        option_kind=option_kind,
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def expiry_timestring_to_datetime(contract_details: IBContractDetails) -> pd.Timestamp:
    last_trade_date = contract_details.contract.lastTradeDateOrContractMonth
    trading_hours = contract_details.tradingHours
    tz_id = contract_details.timeZoneId

    try:
        closing_time = trading_hours.split(";")[-1].split("-")[-1].split(":")[-1]
        local_ts = pd.to_datetime(f"{last_trade_date} {closing_time}", format="%Y%m%d %H%M")
        utc_ts = local_ts.tz_localize(tz_id).tz_convert("UTC")

        return utc_ts
    except (IndexError, ValueError):
        return pd.Timestamp(contract_details.contract.lastTradeDateOrContractMonth, tz="UTC")


def parse_forex_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> CurrencyPair:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    size_precision: int = _tick_size_to_precision(contract_details.minSize)
    timestamp = time.time_ns()

    return CurrencyPair(
        instrument_id=instrument_id,
        raw_symbol=Symbol(contract_details.contract.localSymbol),
        base_currency=Currency.from_str(contract_details.contract.symbol),
        quote_currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        size_precision=size_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        size_increment=Quantity(contract_details.sizeIncrement, size_precision),
        lot_size=None,
        max_quantity=None,
        min_quantity=None,
        max_notional=None,
        min_notional=None,
        max_price=None,
        min_price=None,
        margin_init=Decimal(0),
        margin_maint=Decimal(0),
        maker_fee=Decimal(0),
        taker_fee=Decimal(0),
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def parse_crypto_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> CryptoPerpetual:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    size_precision: int = _tick_size_to_precision(contract_details.minSize)
    timestamp = time.time_ns()

    return CryptoPerpetual(
        instrument_id=instrument_id,
        raw_symbol=Symbol(contract_details.contract.localSymbol),
        base_currency=Currency.from_str(contract_details.contract.symbol),
        quote_currency=Currency.from_str(contract_details.contract.currency),
        settlement_currency=Currency.from_str(contract_details.contract.currency),
        is_inverse=True,
        price_precision=price_precision,
        size_precision=size_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        size_increment=Quantity(contract_details.sizeIncrement, size_precision),
        max_quantity=None,
        min_quantity=Quantity(contract_details.minSize, size_precision),
        max_notional=None,
        min_notional=None,
        max_price=None,
        min_price=None,
        margin_init=Decimal(0),
        margin_maint=Decimal(0),
        maker_fee=Decimal(0),
        taker_fee=Decimal(0),
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def parse_cfd_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> Cfd:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    size_precision: int = _tick_size_to_precision(contract_details.minSize)
    timestamp = time.time_ns()

    if RE_CFD_CASH.match(contract_details.contract.localSymbol):
        return Cfd(
            instrument_id=instrument_id,
            raw_symbol=Symbol(contract_details.contract.localSymbol),
            asset_class=sec_type_to_asset_class(contract_details.underSecType),
            base_currency=Currency.from_str(contract_details.contract.symbol),
            quote_currency=Currency.from_str(contract_details.contract.currency),
            price_precision=price_precision,
            size_precision=size_precision,
            price_increment=Price(contract_details.minTick, price_precision),
            size_increment=Quantity(contract_details.sizeIncrement, size_precision),
            lot_size=None,
            max_quantity=None,
            min_quantity=None,
            max_notional=None,
            min_notional=None,
            max_price=None,
            min_price=None,
            margin_init=Decimal(0),
            margin_maint=Decimal(0),
            maker_fee=Decimal(0),
            taker_fee=Decimal(0),
            ts_event=timestamp,
            ts_init=timestamp,
            info=contract_details_to_dict(contract_details),
        )
    else:
        return Cfd(
            instrument_id=instrument_id,
            raw_symbol=Symbol(contract_details.contract.localSymbol),
            asset_class=sec_type_to_asset_class(contract_details.underSecType),
            quote_currency=Currency.from_str(contract_details.contract.currency),
            price_precision=price_precision,
            size_precision=size_precision,
            price_increment=Price(contract_details.minTick, price_precision),
            size_increment=Quantity(contract_details.sizeIncrement, size_precision),
            lot_size=None,
            max_quantity=None,
            min_quantity=None,
            max_notional=None,
            min_notional=None,
            max_price=None,
            min_price=None,
            margin_init=Decimal(0),
            margin_maint=Decimal(0),
            maker_fee=Decimal(0),
            taker_fee=Decimal(0),
            ts_event=timestamp,
            ts_init=timestamp,
            info=contract_details_to_dict(contract_details),
        )


def parse_commodity_contract(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> Commodity:
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    size_precision: int = _tick_size_to_precision(contract_details.minSize)
    timestamp = time.time_ns()

    return Commodity(
        instrument_id=instrument_id,
        raw_symbol=Symbol(contract_details.contract.localSymbol),
        asset_class=AssetClass.COMMODITY,
        quote_currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        size_precision=size_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        size_increment=Quantity(contract_details.sizeIncrement, size_precision),
        lot_size=None,
        max_quantity=None,
        min_quantity=None,
        max_notional=None,
        min_notional=None,
        max_price=None,
        min_price=None,
        margin_init=Decimal(0),
        margin_maint=Decimal(0),
        maker_fee=Decimal(0),
        taker_fee=Decimal(0),
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def parse_option_spread(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> OptionSpread:
    """
    根据 IB BAG 合约详情解析期权组合。
 
    仅使用合约详情中可用的信息。对于资产类别和其他属性，
    使用与单个期权腿相同的信息。
 
    """
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    timestamp = time.time_ns()

    # 从合约详情中提取标的证券代码
    underlying = contract_details.underSymbol or contract_details.contract.symbol or "UNKNOWN"

    # 根据标的证券类型确定资产类别
    asset_class = (
        sec_type_to_asset_class(contract_details.underSecType)
        if contract_details.underSecType
        else AssetClass.EQUITY
    )

    # 对于期权，乘数代表手数 (例如，每张合约 100 股)
    multiplier = Quantity.from_str(contract_details.contract.multiplier or "100")

    return OptionSpread(
        instrument_id=instrument_id,
        raw_symbol=Symbol(
            contract_details.contract.localSymbol or contract_details.contract.symbol,
        ),
        asset_class=asset_class,
        currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        multiplier=multiplier,
        lot_size=multiplier,  # 对于期权，手数等于乘数
        underlying=underlying,
        strategy_type="SPREAD",
        activation_ns=0,  # BAG 合约没有单一的激活日期
        expiration_ns=0,  # BAG 合约没有单一的到期日期
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def parse_option_spread_instrument_id(
    instrument_id: InstrumentId,
    leg_contract_details: list[tuple[IBContractDetails, int]],
    clock_timestamp_ns: int | None = None,
) -> OptionSpread:
    """
    将组合工具 ID 解析为 OptionSpread (期权组合) 工具。
 
    使用第一条腿的合约详情来确定组合属性。
    这确保了与单个期权合约处理方式的一致性。
 
    Parameters
    ----------
    instrument_id : InstrumentId
        要解析的组合工具 ID。
    leg_contract_details : list[tuple[IBContractDetails, int]]
        组合腿的 (contract_details, ratio) 元组列表。
        合约详情将用于确定工具属性。
    clock_timestamp_ns : int | None, 可选
        以纳秒为单位的时钟时间戳。如果不提供，则使用当前时间。
 
    Returns
    -------
    OptionSpread
        解析后的期权组合工具。
 
    Raises
    ------
    ValueError
        如果工具 ID 无法被解析为组合，或者未提供腿合约详情。
 
    """
    try:
        if not leg_contract_details:
            raise ValueError("必须提供 leg_contract_details")

        # 使用第一条腿的合约详情
        first_details, _ = leg_contract_details[0]
        first_contract = first_details.contract

        # 从第一条腿的合约详情中提取所有属性
        currency = Currency.from_str(first_contract.currency)
        underlying = first_details.underSymbol or first_contract.symbol

        # 使用合约乘数
        multiplier = Quantity.from_str(str(first_contract.multiplier))

        # 根据证券类型确定资产类别
        if first_contract.secType == "FOP":
            asset_class = AssetClass.INDEX  # 期货期权
        else:  # OPT
            asset_class = AssetClass.EQUITY  # 股票期权

        # 从合约详情中读取价格增量
        min_tick = min(leg_details.minTick for leg_details, _ in leg_contract_details)
        price_increment = Price(
            min_tick,
            _tick_size_to_precision(min_tick),
        )
        price_precision = _tick_size_to_precision(min_tick)

        # 使用提供的时间戳或当前时间
        timestamp = clock_timestamp_ns if clock_timestamp_ns is not None else time.time_ns()

        # 对于期权组合，手数等于乘数 (与单个期权合约相同)
        lot_size = multiplier

        # 为第一条腿创建包含合约详情的 info 字典
        # 这对于数据客户端创建订阅合约是必需的
        info = {
            "contract": {
                "secType": first_contract.secType,
                "symbol": first_contract.symbol,
                "currency": first_contract.currency,
                "multiplier": first_contract.multiplier,
            },
        }

        return OptionSpread(
            instrument_id=instrument_id,
            raw_symbol=Symbol(instrument_id.symbol.value),
            asset_class=asset_class,
            currency=currency,
            price_precision=price_precision,
            price_increment=price_increment,
            multiplier=multiplier,
            lot_size=lot_size,
            underlying=underlying,
            strategy_type="SPREAD",
            activation_ns=0,  # 组合没有单一的激活日期
            expiration_ns=0,  # 组合没有单一的到期日期
            ts_event=timestamp,
            ts_init=timestamp,
            info=info,
        )
    except Exception as e:
        raise ValueError(f"解析组合工具 ID {instrument_id} 失败: {e}") from e


def _has_futures(
    contract: IBContract,
    contract_details_map: dict[int, IBContractDetails] | None = None,
) -> bool:
    # 检查 BAG 合约是否包含至少一个期货腿。
    if not contract.comboLegs or not contract_details_map:
        return False

    for combo_leg in contract.comboLegs:
        if combo_leg.conId in contract_details_map:
            leg_details = contract_details_map[combo_leg.conId]
            if leg_details.contract.secType in ("FUT", "CONTFUT"):
                return True

    return False


def parse_futures_spread(
    contract_details: IBContractDetails,
    instrument_id: InstrumentId,
) -> FuturesSpread:
    """
    根据 IB BAG 合约详情解析期货组合。
 
    仅使用合约详情中可用的信息。对于资产类别和其他属性，
    使用与单个期货腿相同的信息。
 
    """
    price_precision: int = _tick_size_to_precision(contract_details.minTick)
    timestamp = time.time_ns()

    # 从合约详情中提取标的证券代码
    underlying = contract_details.underSymbol or contract_details.contract.symbol or "UNKNOWN"

    # 根据标的证券类型确定资产类别
    asset_class = (
        sec_type_to_asset_class(contract_details.underSecType)
        if contract_details.underSecType
        else AssetClass.INDEX
    )

    # 对于期货，乘数通常为 1 或合约乘数
    multiplier = Quantity.from_str(contract_details.contract.multiplier or "1")

    return FuturesSpread(
        instrument_id=instrument_id,
        raw_symbol=Symbol(
            contract_details.contract.localSymbol or contract_details.contract.symbol,
        ),
        asset_class=asset_class,
        currency=Currency.from_str(contract_details.contract.currency),
        price_precision=price_precision,
        price_increment=Price(contract_details.minTick, price_precision),
        multiplier=multiplier,
        lot_size=Quantity.from_int(1),  # 对于期货，手数通常为 1
        underlying=underlying,
        strategy_type="SPREAD",
        activation_ns=0,  # BAG 合约没有单一的激活日期
        expiration_ns=0,  # BAG 合约没有单一的到期日期
        ts_event=timestamp,
        ts_init=timestamp,
        info=contract_details_to_dict(contract_details),
    )


def parse_futures_spread_instrument_id(
    instrument_id: InstrumentId,
    leg_contract_details: list[tuple[IBContractDetails, int]],
    clock_timestamp_ns: int | None = None,
) -> FuturesSpread:
    """
    将组合工具 ID 解析为 FuturesSpread (期货组合) 工具。
 
    使用第一条腿的合约详情来确定组合属性。
    这确保了与单个期货合约处理方式的一致性。
 
    Parameters
    ----------
    instrument_id : InstrumentId
        要解析的组合工具 ID。
    leg_contract_details : list[tuple[IBContractDetails, int]]
        组合腿的 (contract_details, ratio) 元组列表。
        合约详情将用于确定工具属性。
    clock_timestamp_ns : int | None, 可选
        以纳秒为单位的时钟时间戳。如果不提供，则使用当前时间。
 
    Returns
    -------
    FuturesSpread
        解析后的期货组合工具。
 
    Raises
    ------
    ValueError
        如果工具 ID 无法被解析为组合，或者未提供腿合约详情。
 
    """
    try:
        if not leg_contract_details:
            raise ValueError("必须提供 leg_contract_details")

        # 使用第一条腿的合约详情
        first_details, _ = leg_contract_details[0]
        first_contract = first_details.contract

        # 从第一条腿的合约详情中提取所有属性
        currency = Currency.from_str(first_contract.currency)
        underlying = first_details.underSymbol or first_contract.symbol

        # 使用合约乘数
        multiplier = Quantity.from_str(str(first_contract.multiplier))

        # 根据证券类型确定资产类别
        asset_class = sec_type_to_asset_class(first_contract.secType)

        # 从合约详情中读取价格增量
        min_tick = min(leg_details.minTick for leg_details, _ in leg_contract_details)
        price_increment = Price(
            min_tick,
            _tick_size_to_precision(min_tick),
        )
        price_precision = _tick_size_to_precision(min_tick)

        # 使用提供的时间戳或当前时间
        timestamp = clock_timestamp_ns if clock_timestamp_ns is not None else time.time_ns()

        # 对于期货组合，手数通常为 1
        lot_size = Quantity.from_int(1)

        # 为第一条腿创建包含合约详情的 info 字典
        # 这对于数据客户端创建订阅合约是必需的
        info = {
            "contract": {
                "secType": first_contract.secType,
                "symbol": first_contract.symbol,
                "currency": first_contract.currency,
                "multiplier": first_contract.multiplier,
            },
        }

        return FuturesSpread(
            instrument_id=instrument_id,
            raw_symbol=Symbol(instrument_id.symbol.value),
            asset_class=asset_class,
            currency=currency,
            price_precision=price_precision,
            price_increment=price_increment,
            multiplier=multiplier,
            lot_size=lot_size,
            underlying=underlying,
            strategy_type="SPREAD",
            activation_ns=0,  # 组合没有单一的激活日期
            expiration_ns=0,  # 组合没有单一的到期日期
            ts_event=timestamp,
            ts_init=timestamp,
            info=info,
        )
    except Exception as e:
        raise ValueError(
            f"解析期货组合工具 ID {instrument_id} 失败: {e}",
        ) from e


def contract_details_to_dict(contract_details: IBContractDetails) -> dict:
    dict_details = contract_details.dict().copy()
    dict_details["contract"] = contract_details.contract.dict().copy()

    if dict_details.get("secIdList"):
        dict_details["secIdList"] = {
            tag_value.tag: tag_value.value for tag_value in dict_details["secIdList"]
        }

    # 为 JSON 兼容性序列化 Decimal 和 Enum 对象
    result = _serialize_for_json(dict_details)

    # 类型转换：我们知道这是一个字典，因为我们传入了一个字典
    return cast(dict[str, Any], result)


def _serialize_for_json(obj: object) -> object:
    """
    递归地将 Decimal 对象和 Enum 对象转换为可 JSON 序列化的类型。
    """
    if obj is None:
        return None

    if isinstance(obj, Decimal):
        return str(obj)

    if isinstance(obj, Enum):
        return obj.value if hasattr(obj, "value") else str(obj)

    if isinstance(obj, dict):
        return {k: _serialize_for_json(v) for k, v in obj.items()}

    if isinstance(obj, (list, tuple)):
        return [_serialize_for_json(item) for item in obj]

    return obj


def _tick_size_to_precision(tick_size: float | Decimal) -> int:
    tick_size_str = f"{tick_size:.10f}"

    return len(tick_size_str.partition(".")[2].rstrip("0"))


def decade_digit(last_digit: str, contract: IBContract) -> int:
    if year := contract.lastTradeDateOrContractMonth[:4]:
        return int(year[2:3])
    elif int(last_digit) > int(repr(datetime.datetime.now(tz=datetime.UTC).year)[-1]):
        return int(repr(datetime.datetime.now(tz=datetime.UTC).year)[-2]) - 1
    else:
        return int(repr(datetime.datetime.now(tz=datetime.UTC).year)[-2])


def ib_contract_to_instrument_id(
    contract: IBContract,
    venue: str,
    symbology_method: SymbologyMethod = SymbologyMethod.IB_SIMPLIFIED,
    contract_details_map: dict[int, IBContractDetails] | None = None,
) -> InstrumentId:
    PyCondition.type(contract, IBContract, "IBContract")

    if symbology_method == SymbologyMethod.IB_SIMPLIFIED:
        return ib_contract_to_instrument_id_simplified_symbology(
            contract,
            venue,
            contract_details_map,
        )
    elif symbology_method == SymbologyMethod.IB_RAW:
        return ib_contract_to_instrument_id_raw_symbology(contract, venue)
    else:
        raise NotImplementedError(f"{symbology_method} 尚未实现")


def ib_contract_to_instrument_id_simplified_symbology(  # noqa: C901 (too complex)
    contract: IBContract,
    venue: str,
    contract_details_map: dict[int, IBContractDetails] | None = None,
) -> InstrumentId:
    security_type = contract.secType

    if security_type == "BAG":
        return bag_contract_to_instrument_id(contract, venue, contract_details_map)
    elif security_type == "STK":
        symbol = (contract.localSymbol or contract.symbol).replace(" ", "-")
    elif security_type == "IND":
        symbol = f"^{(contract.localSymbol or contract.symbol)}"
    elif security_type == "OPT":
        symbol = contract.localSymbol
    elif security_type == "CONTFUT":
        symbol = contract.symbol
    elif security_type == "FUT" and (m := RE_FUT_ORIGINAL.match(contract.localSymbol)):
        symbol = f"{m['symbol']}{m['month']}{m['year']}"
    elif (security_type == "FUT" and (m := RE_FUT2_ORIGINAL.match(contract.localSymbol))) or (
        security_type == "FUT" and (m := RE_FUT3_ORIGINAL.match(contract.localSymbol))
    ):
        symbol = f"{m['symbol']}{FUTURES_MONTH_TO_CODE[m['month']]}{m['year'][-1]}"
    elif security_type == "FOP" and (m := RE_FOP_ORIGINAL.match(contract.localSymbol)):
        symbol = f"{m['symbol']}{m['month']}{m['year']} {m['right']}{m['strike']}"
    elif security_type in ["CASH", "CRYPTO"]:
        symbol = (
            f"{contract.localSymbol}".replace(".", "/") or f"{contract.symbol}/{contract.currency}"
        )
    elif security_type == "CFD":
        if m := RE_CFD_CASH.match(contract.localSymbol):
            symbol = (
                f"{contract.localSymbol}".replace(".", "/")
                or f"{contract.symbol}/{contract.currency}"
            )
        else:
            symbol = (contract.symbol).replace(" ", "-")
    elif security_type == "CMDTY":
        symbol = (contract.symbol).replace(" ", "-")
    else:
        symbol = None

    if symbol:
        return InstrumentId(Symbol(symbol), Venue(venue))

    raise ValueError(f"未知的 {contract=}")


def bag_contract_to_instrument_id(
    contract: IBContract,
    venue: str,
    contract_details_map: dict[int, IBContractDetails] | None = None,
) -> InstrumentId:
    """
    从 BAG 合约创建组合工具 ID。
 
    这是 _create_bag_contract_from_spread 的反向操作。
    它将 IB BAG 合约转换回 Nautilus 期权组合工具 ID。
 
    Parameters
    ----------
    contract : IBContract
        包含代表组合的 comboLegs 的 BAG 合约
    venue : str
        工具 ID 的交易场所
    contract_details_map : dict[int, IBContractDetails] | None
        合约 ID (conIds) 到其合约详情的映射，用于腿的解析
 
    Returns
    -------
    InstrumentId
        使用 new_generic_spread_id() 创建的组合工具 ID
 
    """
    try:
        if not contract.comboLegs:
            raise ValueError("BAG 合约没有组合腿 (combo legs)")

        # 将组合腿转换为工具 ID 元组
        leg_tuples = []

        for combo_leg in contract.comboLegs:
            # 使用 conId 获取这条腿的合约详情
            if contract_details_map and combo_leg.conId in contract_details_map:
                leg_contract_details = contract_details_map[combo_leg.conId]
                leg_contract = leg_contract_details.contract

                # 从腿合约创建工具 ID
                leg_instrument_id = ib_contract_to_instrument_id_simplified_symbology(
                    leg_contract,
                    venue,
                )
            else:
                raise ValueError(
                    f"无法解析 conId {combo_leg.conId} 的腿工具 ID。"
                    f"未提供合约详情映射或映射不完整。",
                )

            # 确定比例 (BUY 为正, SELL 为负)
            ratio = combo_leg.ratio if combo_leg.action == "BUY" else -combo_leg.ratio

            leg_tuples.append((leg_instrument_id, ratio))

        # 创建组合工具 ID
        return new_generic_spread_id(leg_tuples)

    except Exception as e:
        raise ValueError(f"从 BAG 合约 {contract} 创建组合工具 ID 失败: {e}")


def ib_contract_to_instrument_id_raw_symbology(
    contract: IBContract,
    venue: str,
) -> InstrumentId:
    if contract.secType == "CFD" or contract.secType == "CMDTY":
        symbol = f"{contract.localSymbol}={contract.secType}"
    else:
        symbol = f"{contract.localSymbol}={contract.secType}"

    return InstrumentId.from_str(f"{symbol}.{venue}")


def instrument_id_to_ib_contract(
    instrument_id: InstrumentId,
    exchange: str,
    symbology_method: SymbologyMethod = SymbologyMethod.IB_SIMPLIFIED,
    contract_details_map: dict[InstrumentId, IBContractDetails] | None = None,
) -> IBContract:
    PyCondition.type(instrument_id, InstrumentId, "InstrumentId")

    if symbology_method == SymbologyMethod.IB_SIMPLIFIED:
        return instrument_id_to_ib_contract_simplified_symbology(
            instrument_id,
            exchange,
            contract_details_map,
        )
    elif symbology_method == SymbologyMethod.IB_RAW:
        return instrument_id_to_ib_contract_raw_symbology(instrument_id)
    else:
        raise NotImplementedError(f"{symbology_method} 尚未实现")


def instrument_id_to_ib_contract_simplified_symbology(  # noqa: C901 (too complex)
    instrument_id: InstrumentId,
    exchange: str,
    contract_details_map: dict[InstrumentId, IBContractDetails] | None = None,
) -> IBContract:
    if is_generic_spread_id(instrument_id):
        return instrument_id_to_bag_contract(instrument_id, exchange, contract_details_map)
    elif exchange in VENUES_CASH and (m := RE_CASH.match(instrument_id.symbol.value)):
        return IBContract(
            secType="CASH",
            exchange=exchange,
            localSymbol=f"{m['symbol']}.{m['currency']}",
        )
    elif exchange in VENUES_CRYPTO and (m := RE_CRYPTO.match(instrument_id.symbol.value)):
        return IBContract(
            secType="CRYPTO",
            exchange=exchange,
            localSymbol=f"{m['symbol']}.{m['currency']}",
        )
    elif exchange in VENUES_OPT and (m := RE_OPT.match(instrument_id.symbol.value)):
        return IBContract(
            secType="OPT",
            exchange=exchange,
            localSymbol=f"{m['symbol'].ljust(6)}{m['expiry']}{m['right']}{m['strike']}{m['decimal']}",
        )
    elif exchange in VENUES_FUT:
        if m := RE_FUT_ORIGINAL.match(instrument_id.symbol.value):
            return IBContract(
                secType="FUT",
                exchange=exchange,
                localSymbol=f"{m['symbol']}{m['month']}{m['year']}",
            )
        elif m := RE_FUT_UNDERLYING.match(instrument_id.symbol.value):
            return IBContract(
                secType="CONTFUT",
                exchange=exchange,
                symbol=m["symbol"],
            )
        elif m := RE_FOP_ORIGINAL.match(instrument_id.symbol.value):
            return IBContract(
                secType="FOP",
                exchange=exchange,
                localSymbol=f"{m['symbol']}{m['month']}{m['year']} {m['right']}{m['strike']}",
            )
        else:
            raise ValueError(f"无法解析 {instrument_id}，FUT 和 FOP 请使用 2 位年份代码")
    elif exchange in VENUES_CFD:
        if m := RE_CASH.match(instrument_id.symbol.value):
            return IBContract(
                secType="CFD",
                exchange="SMART",
                symbol=m["symbol"],
                localSymbol=f"{m['symbol']}.{m['currency']}",
            )
        else:
            return IBContract(
                secType="CFD",
                exchange="SMART",
                symbol=f"{instrument_id.symbol.value}".replace("-", " "),
            )
    elif exchange in VENUES_CMDTY:
        return IBContract(
            secType="CMDTY",
            exchange="SMART",
            symbol=f"{instrument_id.symbol.value}".replace("-", " "),
        )
    elif str(instrument_id.symbol).startswith("^"):
        return IBContract(
            secType="IND",
            exchange=exchange,
            localSymbol=instrument_id.symbol.value[1:],
        )

    # 默认为股票 (Stock)
    return IBContract(
        secType="STK",
        exchange="SMART",
        primaryExchange=exchange,
        localSymbol=f"{instrument_id.symbol.value}".replace("-", " "),
    )


def instrument_id_to_bag_contract(
    instrument_id: InstrumentId,
    exchange: str,
    contract_details_map: dict[InstrumentId, IBContractDetails] | None = None,
) -> IBContract:
    try:
        # 将组合 ID 解析回单个腿
        leg_tuples = generic_spread_id_to_list(instrument_id)

        if not leg_tuples:
            raise ValueError("组合工具 ID 没有腿")

        # 为 BAG 合约创建组合腿
        combo_legs = []

        for leg_instrument_id, ratio in leg_tuples:
            # 获取这条腿的合约详情以提取 conId
            if contract_details_map and leg_instrument_id in contract_details_map:
                contract_details = contract_details_map[leg_instrument_id]
                con_id = contract_details.contract.conId
                currency = contract_details.contract.currency
            else:
                # 如果我们没有合约详情，就无法创建一个有效的 BAG 合约
                raise ValueError(
                    f"未找到腿 {leg_instrument_id} 的合约详情。"
                    f"在创建组合之前，请确保所有腿都已加载到工具提供者中。",
                )

            # 根据比例确定动作 (正数 = BUY, 负数 = SELL)
            action = "BUY" if ratio > 0 else "SELL"
            abs_ratio = abs(ratio)

            # 使用实际的 conId 创建组合腿
            combo_leg = ComboLeg(
                conId=con_id,
                ratio=abs_ratio,
                action=action,
                exchange=exchange,
            )
            combo_legs.append(combo_leg)

        # 创建 BAG 合约
        return IBContract(
            secType="BAG",
            exchange=exchange,
            currency=currency,
            comboLegs=combo_legs,
            comboLegsDescrip=f"组合: {instrument_id.symbol.value}",
        )
    except Exception as e:
        raise ValueError(f"从组合 {instrument_id} 创建 BAG 合约失败: {e}")


def instrument_id_to_ib_contract_raw_symbology(instrument_id: InstrumentId) -> IBContract:
    local_symbol, security_type = instrument_id.symbol.value.rsplit("=", 1)
    exchange = instrument_id.venue.value.replace("/", ".")

    if security_type == "STK":
        return IBContract(
            secType=security_type,
            exchange="SMART",
            primaryExchange=exchange,
            localSymbol=local_symbol,
        )
    elif security_type == "CFD":
        return IBContract(
            secType=security_type,
            exchange="SMART",
            localSymbol=local_symbol,  # 在 IB 中，这是股票 CFD 的本地证券代码，末尾带有一个 "n"，例如 "NVDAn"。
        )
    elif security_type == "CMDTY":
        return IBContract(
            secType=security_type,
            exchange="SMART",
            localSymbol=local_symbol,
        )
    elif security_type == "IND":
        return IBContract(
            secType=security_type,
            exchange=exchange,
            localSymbol=local_symbol,
        )
    else:
        return IBContract(
            secType=security_type,
            exchange=exchange,
            localSymbol=local_symbol,
        )
