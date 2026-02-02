# ThinkTrader (XtQuant) Adapter 实现计划

本文档详细描述如何参照 `interactive_brokers` 适配器为 Nautilus Trader 开发 ThinkTrader (迅投 XtQuant) 适配器。

> **文档版本**: v2.0  
> **最后更新**: 2026-01-30  
> **参考文档**: `THINKTRADER_API_NOTES.md` (v1.4), `交易模块文档.md`, `行情模块文档.md`

---

## 目录

1. [项目概述](#1-项目概述)
2. [目录结构规划](#2-目录结构规划)
3. [与 IB 适配器的文件对照](#3-与-ib-适配器的文件对照)
4. [依赖与环境](#4-依赖与环境)
5. [实现步骤](#5-实现步骤)
    - [Phase 1: 基础框架搭建](#phase-1-基础框架搭建)
    - [Phase 2: 底层客户端实现](#phase-2-底层客户端实现)
    - [Phase 3: 行情数据适配](#phase-3-行情数据适配)
    - [Phase 4: 交易执行适配](#phase-4-交易执行适配)
    - [Phase 5: 工厂与集成](#phase-5-工厂与集成)
    - [Phase 6: 测试与文档](#phase-6-测试与文档)
6. [核心映射与转换](#6-核心映射与转换)
7. [XtQuant 数据结构详解](#7-xtquant-数据结构详解)
8. [注意事项与风险点](#8-注意事项与风险点)
9. [参考资料](#9-参考资料)


---

## 1. 项目概述

### 1.1 目标
为 Nautilus Trader 实现 ThinkTrader (XtQuant) 适配器，支持：
- 实时行情数据订阅 (Tick, Bar)
- 历史数据获取
- 订单执行 (下单、撤单、查询)
- 账户与持仓管理

### 1.2 XtQuant API 概况
XtQuant 由迅投提供，分为两个主要模块：
- **`xtdata`**: 行情数据模块，提供订阅和获取行情的函数式 API。
- **`xttrader`**: 交易模块，提供 `XtQuantTrader` 类和 `XtQuantTraderCallback` 回调机制。

**运行依赖**: 必须在本地运行 MiniQmt 客户端（Windows 环境，或通过 Wine）。

---

## 2. 目录结构规划

参照 `interactive_brokers` 适配器的结构，ThinkTrader 适配器的完整目录结构如下：

```
nautilus_trader/adapters/thinktrader/
├── __init__.py                 # 模块导出
├── common.py                   # 公共类型、常量、辅助函数
├── config.py                   # 配置类定义
├── data.py                     # ThinkTraderDataClient (LiveMarketDataClient)
├── execution.py                # ThinkTraderExecutionClient (LiveExecutionClient)
├── factories.py                # 工厂类，用于创建客户端实例
├── providers.py                # ThinkTraderInstrumentProvider
├── client/                     # 底层 XtQuant API 封装
│   ├── __init__.py
│   ├── client.py               # ThinkTraderClient 主类 (含 Callback 实现)
│   ├── common.py               # 客户端共享类型
│   ├── connection.py           # 连接管理 Mixin
│   ├── market_data.py          # 行情订阅 Mixin
│   ├── account.py              # 账户/持仓查询 Mixin
│   ├── order.py                # 订单操作 Mixin
│   ├── contract.py             # 合约查询 Mixin
│   └── error.py                # 错误处理 Mixin
└── parsing/                    # 数据转换与映射
    ├── __init__.py
    ├── data.py                 # 行情数据转换
    ├── instruments.py          # 合约解析
    └── execution.py            # 订单/成交状态映射
```

---

## 3. 与 IB 适配器的文件对照

### 顶层文件

| IB 文件 | ThinkTrader 是否需要 | 理由 |
|---------|---------------------|------|
| `__init__.py` | ✅ 需要 | 模块导出，必须 |
| `common.py` | ✅ 需要 | 公共类型、常量定义 |
| `config.py` | ✅ 需要 | 配置类定义 |
| `data.py` | ✅ 需要 | 数据客户端实现 |
| `execution.py` | ✅ 需要 | 执行客户端实现 |
| `factories.py` | ✅ 需要 | 工厂类 |
| `providers.py` | ✅ 需要 | 工具提供者 |
| `gateway.py` | ❌ 不需要 | IB 用于管理 Docker 容器。XtQuant 依赖本地 MiniQmt，无需 Docker。 |
| `web.py` | ❌ 不需要 | IB 用于网页抓取产品列表。XtQuant 有内置接口获取合约列表。 |
| `historical/` | ❌ 不需要 | XtQuant 历史数据功能可集成到 `data.py` 中。 |

### `client/` 目录

| IB 文件 | ThinkTrader 是否需要 | 理由 |
|---------|---------------------|------|
| `__init__.py` | ✅ 需要 | 模块导出 |
| `client.py` | ✅ 需要 | 主客户端类，聚合所有 Mixin |
| `common.py` | ✅ 需要 | 客户端共享类型 |
| `connection.py` | ✅ 需要 | 连接管理 |
| `market_data.py` | ✅ 需要 | 行情订阅 |
| `account.py` | ✅ 需要 | 账户/持仓查询 |
| `order.py` | ✅ 需要 | 订单操作 |
| `contract.py` | ✅ 需要 | 合约查询封装，供 `providers.py` 调用 |
| `error.py` | ✅ 需要 | 错误码定义、错误处理逻辑 |
| `wrapper.py` | ❌ 不需要 | XtQuant 回调较简单，合并到 `client.py` |

### `parsing/` 目录

| IB 文件 | ThinkTrader 是否需要 | 理由 |
|---------|---------------------|------|
| `__init__.py` | ✅ 需要 | 模块导出 |
| `data.py` | ✅ 需要 | 行情数据转换 |
| `instruments.py` | ✅ 需要 | 合约解析 |
| `execution.py` | ✅ 需要 | 订单/成交状态映射 |
| `price_conversion.py` | ❌ 不需要 | 中国市场无价格缩放机制 |

---

## 4. 依赖与环境

### 4.1 Python 依赖
```toml
# pyproject.toml 或 requirements.txt
xtquant  # XtQuant SDK (通常需要从迅投官方获取或 pip 安装)
```

### 4.2 运行环境要求
- **操作系统**: Windows (MiniQmt 原生支持) 或 Linux (需通过 Wine/虚拟机运行 MiniQmt)
- **MiniQmt 客户端**: 必须已登录并保持运行
- **MiniQmt UserData 路径**: 需要在配置中指定，例如 `C:\迅投\client\userdata_mini`

---

## 5. 实现步骤

### Phase 1: 基础框架搭建

#### 1.1 创建目录结构
```bash
mkdir -p nautilus_trader/adapters/thinktrader/client
mkdir -p nautilus_trader/adapters/thinktrader/parsing
touch nautilus_trader/adapters/thinktrader/__init__.py
touch nautilus_trader/adapters/thinktrader/client/__init__.py
touch nautilus_trader/adapters/thinktrader/parsing/__init__.py
```

#### 1.2 实现 `common.py`
定义适配器共享的常量和类型：

> **参考**: `THINKTRADER_API_NOTES.md` Section 4.1 市场代码, Section 4.2 账号类型

```python
# common.py
from nautilus_trader.model.identifiers import Venue

TT = "THINKTRADER"
TT_VENUE = Venue(TT)

# ============================================================================
# XtQuant 市场代码映射 (来源: API_NOTES 4.1)
# ============================================================================
MARKET_CODE_MAP = {
    "SH": "xtconstant.SH_MARKET",                      # 上交所
    "SZ": "xtconstant.SZ_MARKET",                      # 深交所
    "BJ": "xtconstant.MARKET_ENUM_BEIJING",            # 北交所
    "HGT": "xtconstant.MARKET_ENUM_SHANGHAI_HONGKONG_STOCK",  # 沪港通
    "SGT": "xtconstant.MARKET_ENUM_SHENZHEN_HONGKONG_STOCK",  # 深港通
    "SF": "xtconstant.MARKET_ENUM_SHANGHAI_FUTURE",     # 上期所
    "DF": "xtconstant.MARKET_ENUM_DALIANG_FUTURE",      # 大商所
    "ZF": "xtconstant.MARKET_ENUM_ZHENGZHOU_FUTURE",    # 郑商所
    "IF": "xtconstant.MARKET_ENUM_INDEX_FUTURE",        # 中金所
    "INE": "xtconstant.MARKET_ENUM_INTL_ENERGY_FUTURE", # 能源中心
    "GF": "xtconstant.MARKET_ENUM_GUANGZHOU_FUTURE",    # 广期所
    "SHO": "xtconstant.MARKET_ENUM_SHANGHAI_STOCK_OPTION",  # 上海期权
    "SZO": "xtconstant.MARKET_ENUM_SHENZHEN_STOCK_OPTION",  # 深圳期权
}

# Nautilus Venue -> XtQuant 市场代码
VENUE_TO_MARKET = {
    "SSE": "SH",     # 上交所
    "SZSE": "SZ",    # 深交所
    "BSE": "BJ",     # 北交所
    "SHFE": "SF",    # 上期所
    "DCE": "DF",     # 大商所
    "CZCE": "ZF",    # 郑商所
    "CFFEX": "IF",   # 中金所
    "INE": "INE",    # 能源中心
    "GFEX": "GF",    # 广期所
}

# XtQuant 市场代码 -> Nautilus Venue
MARKET_TO_VENUE = {v: k for k, v in VENUE_TO_MARKET.items()}

# ============================================================================
# 账号类型映射 (来源: API_NOTES 4.2)
# ============================================================================
ACCOUNT_TYPE_MAP = {
    "FUTURE": "xtconstant.FUTURE_ACCOUNT",          # 期货
    "STOCK": "xtconstant.SECURITY_ACCOUNT",         # 股票
    "CREDIT": "xtconstant.CREDIT_ACCOUNT",          # 信用
    "FUTURE_OPTION": "xtconstant.FUTURE_OPTION_ACCOUNT",    # 期货期权
    "STOCK_OPTION": "xtconstant.STOCK_OPTION_ACCOUNT",      # 股票期权
    "HGT": "xtconstant.HUGANGTONG_ACCOUNT",         # 沪港通
    "SGT": "xtconstant.SHENGANGTONG_ACCOUNT",       # 深港通
}
```

#### 1.3 实现 `config.py`
定义配置类：

```python
# config.py
from nautilus_trader.config import LiveDataClientConfig, LiveExecClientConfig, InstrumentProviderConfig

class ThinkTraderInstrumentProviderConfig(InstrumentProviderConfig, frozen=True):
    """ThinkTrader 工具提供者配置"""
    load_contracts_on_start: bool = True
    cache_instruments: bool = True
    sectors: list[str] = ["沪深A股"]  # 要加载的板块列表
    filter_expiry: bool = False  # 是否过滤已过期合约

class ThinkTraderDataClientConfig(LiveDataClientConfig, frozen=True):
    """ThinkTrader 数据客户端配置"""
    miniqmt_path: str  # MiniQmt userdata 路径
    session_id: int = 123456  # 会话 ID
    instrument_provider: ThinkTraderInstrumentProviderConfig = None
    subscribe_whole_quote: bool = False  # 是否使用全推行情
    subscription_delay_secs: float = 0.1  # 订阅间隔 (避免过快)

class ThinkTraderExecClientConfig(LiveExecClientConfig, frozen=True):
    """ThinkTrader 执行客户端配置"""
    miniqmt_path: str
    account_id: str       # 资金账号
    account_type: str = "STOCK"  # 账号类型: STOCK, CREDIT, FUTURE, OPTION
    session_id: int = 123456  # 会话 ID
    use_async_order: bool = True  # 是否使用异步下单
    relaxed_response_order: bool = True  # 开启宽松时序模式
    instrument_provider: ThinkTraderInstrumentProviderConfig = None
```

---

### Phase 2: 底层客户端实现

#### 2.1 实现 `client/common.py`
定义客户端共享的类型和辅助结构：

```python
# client/common.py
from typing import NamedTuple, Callable, Any
from decimal import Decimal
from abc import ABC

class TTPosition(NamedTuple):
    """ThinkTrader 持仓结构"""
    account_id: str
    stock_code: str
    volume: int
    available_volume: int
    avg_price: float
    market_value: float

class TTOrder(NamedTuple):
    """ThinkTrader 订单结构"""
    order_id: int
    order_sysid: str
    stock_code: str
    order_type: int
    order_volume: int
    traded_volume: int
    price: float
    traded_price: float
    order_status: int
    order_remark: str

class TTTrade(NamedTuple):
    """ThinkTrader 成交结构"""
    account_id: str
    order_id: int
    order_sysid: str
    stock_code: str
    traded_id: str
    traded_price: float
    traded_volume: int
    traded_time: int

class BaseMixin(ABC):
    """Mixin 基类，提供类型提示"""
    _loop: Any
    _log: Any
    _trader: Any
    _account: Any
    _miniqmt_path: str
    _session_id: int
    _account_id: str
    _subscriptions: dict
    _requests: dict
```

#### 2.2 实现 `client/connection.py`
管理与 MiniQmt 的连接：

```python
# client/connection.py
from xtquant import xttrader
from xtquant.xttype import StockAccount

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin
from nautilus_trader.common.enums import LogColor

class ThinkTraderClientConnectionMixin(BaseMixin):
    """管理与 MiniQmt 的连接"""
    
    async def _connect(self) -> None:
        """建立连接"""
        self._log.info(f"正在连接到 MiniQmt: {self._miniqmt_path}")
        
        self._trader = xttrader.XtQuantTrader(
            self._miniqmt_path,
            self._session_id,
        )
        self._trader.register_callback(self._callback)
        self._trader.start()
        
        connect_result = self._trader.connect()
        if connect_result != 0:
            raise ConnectionError(f"连接 MiniQmt 失败，错误码: {connect_result}")
        
        self._account = StockAccount(self._account_id)
        subscribe_result = self._trader.subscribe(self._account)
        if subscribe_result != 0:
            raise ConnectionError(f"订阅账户失败，错误码: {subscribe_result}")
        
        self._is_connected.set()
        self._log.info(
            f"已连接到 MiniQmt，账户: {self._account_id}",
            LogColor.GREEN,
        )
    
    async def _disconnect(self) -> None:
        """断开连接"""
        if self._trader:
            self._trader.unsubscribe(self._account)
            self._trader.stop()
        self._is_connected.clear()
        self._log.info("已断开与 MiniQmt 的连接")
    
    def _check_connection(self) -> bool:
        """检查连接状态"""
        return self._is_connected.is_set()
```

#### 2.3 实现 `client/error.py`
集中处理错误：

```python
# client/error.py
from typing import Final

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin
from nautilus_trader.common.enums import LogColor

class ThinkTraderClientErrorMixin(BaseMixin):
    """处理 ThinkTrader 客户端的错误和警告"""
    
    # 常见错误码分类
    CONNECTION_ERRORS: Final[set[int]] = {-1, -2, -3}
    ORDER_REJECTION_CODES: Final[set[int]] = {50001, 50002, 50003}
    AUTHENTICATION_ERRORS: Final[set[int]] = {10001, 10002}
    
    def _handle_order_error(self, order_error) -> None:
        """处理下单错误"""
        error_id = order_error.error_id
        error_msg = order_error.error_msg
        order_id = order_error.order_id
        
        self._log.error(
            f"下单失败: order_id={order_id}, "
            f"error_id={error_id}, error_msg={error_msg}"
        )
        
        # 触发订单拒绝事件
        if handler := self._event_handlers.get("order_rejected"):
            self._loop.call_soon_threadsafe(
                handler,
                order_id,
                error_msg,
            )
    
    def _handle_cancel_error(self, cancel_error) -> None:
        """处理撤单错误"""
        error_id = cancel_error.error_id
        error_msg = cancel_error.error_msg
        order_id = cancel_error.order_id
        
        self._log.warning(
            f"撤单失败: order_id={order_id}, "
            f"error_id={error_id}, error_msg={error_msg}"
        )
    
    def _handle_connection_error(self, error_code: int) -> None:
        """处理连接错误"""
        if error_code in self.CONNECTION_ERRORS:
            self._log.error(f"连接错误: {error_code}")
            self._is_connected.clear()
```

#### 2.4 实现 `client/contract.py`
封装合约查询：

```python
# client/contract.py
from xtquant import xtdata

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin

class ThinkTraderClientContractMixin(BaseMixin):
    """处理合约查询"""
    
    def get_instrument_detail(self, stock_code: str) -> dict | None:
        """
        获取合约详情
        
        Parameters
        ----------
        stock_code : str
            合约代码，如 '600000.SH'
        
        Returns
        -------
        dict | None
            合约详情字典，如果未找到则返回 None
        """
        try:
            detail = xtdata.get_instrument_detail(stock_code)
            if detail:
                self._log.debug(f"获取合约详情: {stock_code}")
                return detail
            else:
                self._log.warning(f"未找到合约: {stock_code}")
                return None
        except Exception as e:
            self._log.error(f"获取合约详情失败: {stock_code}, 错误: {e}")
            return None
    
    def get_stock_list(self, sector: str = "沪深A股") -> list[str]:
        """
        获取板块内的合约列表
        
        Parameters
        ----------
        sector : str
            板块名称，如 '沪深A股', '上证50' 等
        
        Returns
        -------
        list[str]
            合约代码列表
        """
        try:
            stocks = xtdata.get_stock_list_in_sector(sector)
            self._log.debug(f"获取 {sector} 合约列表，共 {len(stocks)} 个")
            return stocks
        except Exception as e:
            self._log.error(f"获取合约列表失败: {sector}, 错误: {e}")
            return []
    
    def get_trading_dates(
        self,
        market: str = "SH",
        start_date: str = "",
        end_date: str = "",
    ) -> list[str]:
        """获取交易日历"""
        try:
            dates = xtdata.get_trading_dates(market, start_date, end_date)
            return dates
        except Exception as e:
            self._log.error(f"获取交易日历失败: {e}")
            return []
```

#### 2.5 实现 `client/market_data.py`
封装 `xtdata` 行情接口：

```python
# client/market_data.py
from xtquant import xtdata

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin

class ThinkTraderClientMarketDataMixin(BaseMixin):
    """处理行情数据订阅"""
    
    def subscribe_quote(
        self,
        stock_code: str,
        period: str = "tick",
        callback = None,
    ) -> int:
        """
        订阅行情
        
        Parameters
        ----------
        stock_code : str
            合约代码
        period : str
            周期: 'tick', '1m', '5m', '1d' 等
        callback : callable
            回调函数
        
        Returns
        -------
        int
            订阅号，大于 0 表示成功
        """
        seq = xtdata.subscribe_quote(
            stock_code=stock_code,
            period=period,
            callback=callback or self._on_quote_data,
        )
        if seq > 0:
            self._subscriptions[seq] = {
                "stock_code": stock_code,
                "period": period,
            }
            self._log.debug(f"已订阅 {stock_code} ({period}), seq={seq}")
        else:
            self._log.warning(f"订阅失败: {stock_code} ({period})")
        return seq
    
    def subscribe_whole_quote(
        self,
        code_list: list[str],
        callback = None,
    ) -> int:
        """订阅全推行情"""
        seq = xtdata.subscribe_whole_quote(
            code_list=code_list,
            callback=callback or self._on_whole_quote_data,
        )
        if seq > 0:
            self._log.debug(f"已订阅全推行情，共 {len(code_list)} 个品种")
        return seq
    
    def unsubscribe_quote(self, seq: int) -> None:
        """取消订阅"""
        xtdata.unsubscribe_quote(seq)
        if seq in self._subscriptions:
            info = self._subscriptions.pop(seq)
            self._log.debug(f"已取消订阅: {info}")
    
    def get_market_data(
        self,
        stock_list: list[str],
        period: str = "1d",
        start_time: str = "",
        end_time: str = "",
        count: int = -1,
    ) -> dict:
        """获取历史行情数据"""
        return xtdata.get_market_data_ex(
            stock_list=stock_list,
            period=period,
            start_time=start_time,
            end_time=end_time,
            count=count,
        )
    
    def _on_quote_data(self, datas: dict) -> None:
        """行情回调处理"""
        for stock_code, data_list in datas.items():
            for data in data_list:
                self._loop.call_soon_threadsafe(
                    self._handle_quote_data,
                    stock_code,
                    data,
                )
    
    def _on_whole_quote_data(self, datas: dict) -> None:
        """全推行情回调处理"""
        for stock_code, data in datas.items():
            self._loop.call_soon_threadsafe(
                self._handle_quote_data,
                stock_code,
                data,
            )
```

#### 2.6 实现 `client/account.py`
账户与持仓查询：

```python
# client/account.py
from nautilus_trader.adapters.thinktrader.client.common import BaseMixin, TTPosition

class ThinkTraderClientAccountMixin(BaseMixin):
    """处理账户和持仓查询"""
    
    def query_asset(self) -> dict | None:
        """查询账户资产"""
        try:
            asset = self._trader.query_stock_asset(self._account)
            return asset
        except Exception as e:
            self._log.error(f"查询资产失败: {e}")
            return None
    
    def query_positions(self) -> list[TTPosition]:
        """查询持仓"""
        try:
            positions = self._trader.query_stock_positions(self._account)
            result = []
            for pos in positions:
                result.append(TTPosition(
                    account_id=pos.account_id,
                    stock_code=pos.stock_code,
                    volume=pos.volume,
                    available_volume=pos.can_use_volume,
                    avg_price=pos.open_price,
                    market_value=pos.market_value,
                ))
            return result
        except Exception as e:
            self._log.error(f"查询持仓失败: {e}")
            return []
    
    def query_orders(self) -> list:
        """查询当日委托"""
        try:
            return self._trader.query_stock_orders(self._account)
        except Exception as e:
            self._log.error(f"查询委托失败: {e}")
            return []
    
    def query_trades(self) -> list:
        """查询当日成交"""
        try:
            return self._trader.query_stock_trades(self._account)
        except Exception as e:
            self._log.error(f"查询成交失败: {e}")
            return []
```

#### 2.7 实现 `client/order.py`
封装订单操作：

```python
# client/order.py
from xtquant import xtconstant

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin

class ThinkTraderClientOrderMixin(BaseMixin):
    """处理订单操作"""
    
    def place_order(
        self,
        stock_code: str,
        order_type: int,
        volume: int,
        price: float,
        price_type: int = xtconstant.FIX_PRICE,
        strategy_name: str = "",
        order_remark: str = "",
    ) -> int:
        """
        下单
        
        Parameters
        ----------
        stock_code : str
            合约代码
        order_type : int
            委托类型: STOCK_BUY, STOCK_SELL 等
        volume : int
            委托数量
        price : float
            委托价格
        price_type : int
            报价类型: FIX_PRICE, LATEST_PRICE 等
        strategy_name : str
            策略名称
        order_remark : str
            委托备注 (用于存储 ClientOrderId)
        
        Returns
        -------
        int
            订单 ID，大于 0 表示成功
        """
        order_id = self._trader.order_stock(
            account=self._account,
            stock_code=stock_code,
            order_type=order_type,
            order_volume=volume,
            price_type=price_type,
            price=price,
            strategy_name=strategy_name,
            order_remark=order_remark,
        )
        
        if order_id > 0:
            self._log.debug(
                f"下单成功: {stock_code}, order_id={order_id}, "
                f"type={order_type}, volume={volume}, price={price}"
            )
        else:
            self._log.warning(f"下单失败: {stock_code}, 返回值={order_id}")
        
        return order_id
    
    def cancel_order(self, order_id: int) -> int:
        """撤单 (通过订单 ID)"""
        result = self._trader.cancel_order_stock(
            account=self._account,
            order_id=order_id,
        )
        self._log.debug(f"撤单请求: order_id={order_id}, result={result}")
        return result
    
    def cancel_order_by_sysid(self, market: int, order_sysid: str) -> int:
        """
        撤单 (通过柜台编号)
        
        注意: 根据文档，此接口需要传入 market 参数
        - market: 交易市场，xtconstant.SH_MARKET 或 xtconstant.SZ_MARKET
        - order_sysid: 券商柜台的合同编号
        """
        result = self._trader.cancel_order_stock_sysid(
            account=self._account,
            market=market,
            order_sysid=order_sysid,
        )
        self._log.debug(f"撤单请求: market={market}, order_sysid={order_sysid}, result={result}")
        return result
    
    def place_order_async(
        self,
        stock_code: str,
        order_type: int,
        volume: int,
        price: float,
        price_type: int = xtconstant.FIX_PRICE,
        strategy_name: str = "",
        order_remark: str = "",
    ) -> int:
        """
        异步下单
        
        返回下单请求序号 seq，成功时 seq > 0，失败返回 -1。
        异步下单后会收到 on_order_stock_async_response 回调。
        """
        seq = self._trader.order_stock_async(
            account=self._account,
            stock_code=stock_code,
            order_type=order_type,
            order_volume=volume,
            price_type=price_type,
            price=price,
            strategy_name=strategy_name,
            order_remark=order_remark,
        )
        
        if seq > 0:
            self._log.debug(
                f"异步下单: {stock_code}, seq={seq}, "
                f"type={order_type}, volume={volume}, price={price}"
            )
        else:
            self._log.warning(f"异步下单失败: {stock_code}, 返回值={seq}")
        
        return seq
    
    def cancel_order_async(self, order_id: int) -> int:
        """异步撤单 (通过订单 ID)"""
        seq = self._trader.cancel_order_stock_async(
            account=self._account,
            order_id=order_id,
        )
        self._log.debug(f"异步撤单请求: order_id={order_id}, seq={seq}")
        return seq
```

#### 2.8 实现 `client/client.py`
组合所有 Mixin：

```python
# client/client.py
import asyncio
from xtquant.xttrader import XtQuantTraderCallback

from nautilus_trader.common.component import Logger
from nautilus_trader.adapters.thinktrader.client.connection import (
    ThinkTraderClientConnectionMixin,
)
from nautilus_trader.adapters.thinktrader.client.market_data import (
    ThinkTraderClientMarketDataMixin,
)
from nautilus_trader.adapters.thinktrader.client.account import (
    ThinkTraderClientAccountMixin,
)
from nautilus_trader.adapters.thinktrader.client.order import (
    ThinkTraderClientOrderMixin,
)
from nautilus_trader.adapters.thinktrader.client.contract import (
    ThinkTraderClientContractMixin,
)
from nautilus_trader.adapters.thinktrader.client.error import (
    ThinkTraderClientErrorMixin,
)


class ThinkTraderClientCallback(XtQuantTraderCallback):
    """XtQuant 回调实现"""
    
    def __init__(self, client: "ThinkTraderClient"):
        super().__init__()
        self._client = client
    
    def on_disconnected(self):
        """连接断开"""
        self._client._on_disconnected()
    
    def on_account_status(self, status):
        """账号状态变更"""
        self._client._on_account_status(status)
    
    def on_stock_order(self, order):
        """委托回报"""
        self._client._on_order(order)
    
    def on_stock_trade(self, trade):
        """成交回报"""
        self._client._on_trade(trade)
    
    def on_order_error(self, order_error):
        """下单错误"""
        self._client._handle_order_error(order_error)
    
    def on_cancel_error(self, cancel_error):
        """撤单错误"""
        self._client._handle_cancel_error(cancel_error)
    
    def on_order_stock_async_response(self, response):
        """异步下单回报"""
        self._client._on_order_async_response(response)


class ThinkTraderClient(
    ThinkTraderClientConnectionMixin,
    ThinkTraderClientMarketDataMixin,
    ThinkTraderClientAccountMixin,
    ThinkTraderClientOrderMixin,
    ThinkTraderClientContractMixin,
    ThinkTraderClientErrorMixin,
):
    """
    ThinkTrader 底层客户端
    
    聚合所有 Mixin 功能，与 XtQuant API 交互。
    """
    
    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        logger: Logger,
        miniqmt_path: str,
        session_id: int,
        account_id: str,
    ):
        self._loop = loop
        self._log = logger
        self._miniqmt_path = miniqmt_path
        self._session_id = session_id
        self._account_id = account_id
        
        self._trader = None
        self._account = None
        self._callback = ThinkTraderClientCallback(self)
        
        self._is_connected = asyncio.Event()
        self._subscriptions: dict[int, dict] = {}
        self._requests: dict[int, dict] = {}
        self._event_handlers: dict[str, callable] = {}
    
    def register_event_handler(self, event_name: str, handler: callable) -> None:
        """注册事件处理器"""
        self._event_handlers[event_name] = handler
    
    def _on_disconnected(self) -> None:
        """处理连接断开"""
        self._log.warning("与 MiniQmt 的连接已断开")
        self._is_connected.clear()
        if handler := self._event_handlers.get("disconnected"):
            self._loop.call_soon_threadsafe(handler)
    
    def _on_account_status(self, status) -> None:
        """处理账号状态变更"""
        self._log.info(
            f"账号状态: {status.account_id}, "
            f"type={status.account_type}, status={status.status}"
        )
    
    def _on_order(self, order) -> None:
        """处理委托回报"""
        self._loop.call_soon_threadsafe(
            self._handle_order_update,
            order,
        )
    
    def _on_trade(self, trade) -> None:
        """处理成交回报"""
        self._loop.call_soon_threadsafe(
            self._handle_trade,
            trade,
        )
    
    def _on_order_async_response(self, response) -> None:
        """处理异步下单回报"""
        self._log.debug(
            f"异步下单回报: account={response.account_id}, "
            f"order_id={response.order_id}, seq={response.seq}"
        )
    
    def _handle_quote_data(self, stock_code: str, data: dict) -> None:
        """处理行情数据 (在主循环中执行)"""
        if handler := self._event_handlers.get("quote_data"):
            handler(stock_code, data)
    
    def _handle_order_update(self, order) -> None:
        """处理委托更新 (在主循环中执行)"""
        if handler := self._event_handlers.get("order_update"):
            handler(order)
    
    def _handle_trade(self, trade) -> None:
        """处理成交 (在主循环中执行)"""
        if handler := self._event_handlers.get("trade"):
            handler(trade)
```

---

### Phase 3: 行情数据适配

#### 3.1 实现 `parsing/data.py`
转换 XtQuant 的行情数据为 Nautilus 格式：

> **参考**: `THINKTRADER_API_NOTES.md` Section 6 行情数据结构
>
> 字段定义 (来源: API_NOTES 6.2 tick分笔数据):
> - `bidPrice` / `askPrice`: 委买价/委卖价 (数组，支持多档)
> - `bidVol` / `askVol`: 委买量/委卖量 (数组)
> - `lastPrice`: 最新价
> - `stockStatus`: 证券状态 (参见 API_NOTES 6.11)
> - `volume`: 成交总量
> - `time`: 时间戳 (毫秒)

```python
# parsing/data.py
from nautilus_trader.model.data import QuoteTick, TradeTick, Bar, BarType
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.objects import Price, Quantity
from nautilus_trader.model.enums import BarAggregation

# ============================================================================
# 周期类型映射 (来源: API_NOTES 6.1)
# ============================================================================
PERIOD_MAP = {
    # Level1 数据
    BarAggregation.TICK: "tick",
    BarAggregation.MINUTE: "1m",
    BarAggregation.HOUR: "1h",
    BarAggregation.DAY: "1d",
    BarAggregation.WEEK: "1w",
    BarAggregation.MONTH: "1mon",
}

# Level2 周期类型
LEVEL2_PERIOD_MAP = {
    "l2quote": "l2quote",        # Level2实时行情快照
    "l2order": "l2order",        # Level2逐笔委托
    "l2transaction": "l2transaction",  # Level2逐笔成交
    "l2quoteaux": "l2quoteaux",  # Level2实时行情补充
    "l2orderqueue": "l2orderqueue",    # Level2委托队列
}

# step -> period (分钟级别)
STEP_TO_PERIOD = {
    1: "1m",
    5: "5m",
    15: "15m",
    30: "30m",
    60: "1h",
}

# ============================================================================
# 证券状态映射 (来源: API_NOTES 6.11)
# ============================================================================
STOCK_STATUS_MAP = {
    0: "UNKNOWN",       # 默认未知
    10: "UNKNOWN",      # 默认未知
    11: "PRE_OPEN",     # 开盘前 S
    12: "AUCTION",      # 集合竞价时段 C
    13: "CONTINUOUS",   # 连续交易 T
    14: "BREAK",        # 休市 B
    15: "CLOSED",       # 闭市 E
    16: "HALT",         # 波动性中断 V
    17: "SUSPENDED",    # 临时停牌 P
    18: "CLOSE_AUCTION", # 收盘集合竞价 U
    19: "MID_AUCTION",  # 盘中集合竞价 M
    20: "HALT_TO_CLOSE", # 暂停交易至闭市 N
    21: "ERROR",        # 获取字段异常
    22: "POST_TRADING", # 盘后固定价格行情
    23: "POST_CLOSED",  # 盘后固定价格行情完毕
}

def parse_tick_to_quote_tick(
    instrument_id: InstrumentId,
    data: dict,
    ts_init: int,
) -> QuoteTick:
    """
    将 XtQuant tick 数据转换为 QuoteTick
    
    XtQuant tick 字段 (参见 行情模块文档.md - tick分笔数据):
    - time: 时间戳 (毫秒)
    - lastPrice: 最新价
    - bidPrice: 委买价 (数组，多档)
    - askPrice: 委卖价 (数组，多档)
    - bidVol: 委买量 (数组，多档)
    - askVol: 委卖量 (数组，多档)
    """
    # 取第一档买卖盘
    bid_prices = data.get("bidPrice", [0.0])
    ask_prices = data.get("askPrice", [0.0])
    bid_vols = data.get("bidVol", [0])
    ask_vols = data.get("askVol", [0])
    
    # 确保数组非空
    bid_price = bid_prices[0] if bid_prices else 0.0
    ask_price = ask_prices[0] if ask_prices else 0.0
    bid_vol = int(bid_vols[0]) if bid_vols else 0
    ask_vol = int(ask_vols[0]) if ask_vols else 0
    
    # time 是毫秒时间戳，需要转换为纳秒
    ts_event = int(data.get("time", 0)) * 1_000_000
    
    return QuoteTick(
        instrument_id=instrument_id,
        bid_price=Price.from_str(f"{bid_price:.4f}"),
        ask_price=Price.from_str(f"{ask_price:.4f}"),
        bid_size=Quantity.from_int(bid_vol),
        ask_size=Quantity.from_int(ask_vol),
        ts_event=ts_event,
        ts_init=ts_init,
    )

def parse_tick_to_trade_tick(
    instrument_id: InstrumentId,
    data: dict,
    ts_init: int,
) -> TradeTick:
    """
    将 XtQuant tick 数据转换为 TradeTick
    
    XtQuant tick 字段:
    - lastPrice: 最新价
    - volume: 成交总量
    - transactionNum: 成交笔数
    """
    from nautilus_trader.model.identifiers import TradeId
    from nautilus_trader.model.enums import AggressorSide
    
    ts_event = int(data.get("time", 0)) * 1_000_000
    
    return TradeTick(
        instrument_id=instrument_id,
        price=Price.from_str(f"{data.get('lastPrice', 0.0):.4f}"),
        size=Quantity.from_int(int(data.get("volume", 0))),
        aggressor_side=AggressorSide.NO_AGGRESSOR,
        trade_id=TradeId(str(data.get("time", 0))),
        ts_event=ts_event,
        ts_init=ts_init,
    )

def parse_kline_to_bar(
    instrument_id: InstrumentId,
    bar_type: BarType,
    data: dict,
    ts_init: int,
) -> Bar:
    """
    将 XtQuant K线数据转换为 Bar
    
    XtQuant K线字段 (参见 行情模块文档.md - K线数据):
    - time: 时间戳
    - open/high/low/close: OHLC 价格
    - volume: 成交量
    - amount: 成交额
    - preClose: 前收价
    """
    ts_event = int(data.get("time", 0)) * 1_000_000
    
    return Bar(
        bar_type=bar_type,
        open=Price.from_str(f"{data.get('open', 0.0):.4f}"),
        high=Price.from_str(f"{data.get('high', 0.0):.4f}"),
        low=Price.from_str(f"{data.get('low', 0.0):.4f}"),
        close=Price.from_str(f"{data.get('close', 0.0):.4f}"),
        volume=Quantity.from_int(int(data.get("volume", 0))),
        ts_event=ts_event,
        ts_init=ts_init,
    )
```

#### 3.2 实现 `parsing/instruments.py`
合约解析：

```python
# parsing/instruments.py
from datetime import datetime
from typing import Optional

from nautilus_trader.model.instruments import Equity, FuturesContract, OptionsContract
from nautilus_trader.model.identifiers import InstrumentId, Symbol
from nautilus_trader.model.objects import Price, Quantity, Currency
from nautilus_trader.model.enums import AssetClass, OptionKind

from nautilus_trader.adapters.thinktrader.common import TT_VENUE

# 市场代码映射: XtQuant市场后缀 -> Nautilus Venue 后缀
MARKET_TO_VENUE = {
    "SH": "SSE",     # 上交所
    "SZ": "SZSE",    # 深交所
    "BJ": "BSE",     # 北交所
    "SF": "SHFE",    # 上期所
    "DF": "DCE",     # 大商所
    "ZF": "CZCE",    # 郑商所
    "IF": "CFFEX",   # 中金所
    "INE": "INE",    # 能源中心
    "GF": "GFEX",    # 广期所
}

VENUE_TO_MARKET = {v: k for k, v in MARKET_TO_VENUE.items()}


def parse_equity(detail: dict, instrument_id: InstrumentId) -> Equity:
    """解析股票合约"""
    price_tick = detail.get("PriceTick", 0.01)
    price_precision = _get_precision(price_tick)
    
    return Equity(
        instrument_id=instrument_id,
        raw_symbol=Symbol(detail.get("InstrumentID", "")),
        currency=Currency.from_str("CNY"),
        price_precision=price_precision,
        price_increment=Price.from_str(f"{price_tick}"),
        lot_size=Quantity.from_int(100),  # A股最小交易单位
        ts_event=0,
        ts_init=0,
    )


def parse_future(detail: dict, instrument_id: InstrumentId) -> FuturesContract:
    """解析期货合约"""
    price_tick = detail.get("PriceTick", 0.01)
    price_precision = _get_precision(price_tick)
    multiplier = detail.get("VolumeMultiple", 1)
    
    # 解析到期日: ExpireDate 格式通常是 YYYYMMDD
    expire_date_str = str(detail.get("ExpireDate", ""))
    activation_ns = 0
    expiration_ns = 0
    if expire_date_str and len(expire_date_str) == 8:
        try:
            expire_dt = datetime.strptime(expire_date_str, "%Y%m%d")
            expiration_ns = int(expire_dt.timestamp() * 1_000_000_000)
        except ValueError:
            pass
    
    return FuturesContract(
        instrument_id=instrument_id,
        raw_symbol=Symbol(detail.get("InstrumentID", "")),
        asset_class=AssetClass.COMMODITY,  # 可根据品种调整
        currency=Currency.from_str("CNY"),
        price_precision=price_precision,
        price_increment=Price.from_str(f"{price_tick}"),
        multiplier=Quantity.from_int(int(multiplier)),
        lot_size=Quantity.from_int(1),
        activation_ns=activation_ns,
        expiration_ns=expiration_ns,
        ts_event=0,
        ts_init=0,
    )


def parse_option(detail: dict, instrument_id: InstrumentId) -> OptionsContract:
    """解析期权合约"""
    price_tick = detail.get("PriceTick", 0.0001)
    price_precision = _get_precision(price_tick)
    multiplier = detail.get("OptUnit", detail.get("VolumeMultiple", 10000))
    
    # 期权类型: OptionType -1=非期权, 0=认购, 1=认沽
    option_type = detail.get("OptionType", -1)
    option_kind = OptionKind.CALL if option_type == 0 else OptionKind.PUT
    
    # 行权价
    strike_price = detail.get("OptExercisePrice", 0.0)
    
    # 标的代码
    underlying_code = detail.get("OptUndlCode", "")
    
    # 到期日
    expire_date_str = str(detail.get("ExpireDate", detail.get("EndDelivDate", "")))
    activation_ns = 0
    expiration_ns = 0
    if expire_date_str and len(expire_date_str) >= 8:
        try:
            expire_dt = datetime.strptime(expire_date_str[:8], "%Y%m%d")
            expiration_ns = int(expire_dt.timestamp() * 1_000_000_000)
        except ValueError:
            pass
    
    return OptionsContract(
        instrument_id=instrument_id,
        raw_symbol=Symbol(detail.get("InstrumentID", "")),
        asset_class=AssetClass.EQUITY,  # 股票期权
        currency=Currency.from_str("CNY"),
        price_precision=price_precision,
        price_increment=Price.from_str(f"{price_tick}"),
        multiplier=Quantity.from_int(int(multiplier)),
        lot_size=Quantity.from_int(1),
        underlying=underlying_code,
        option_kind=option_kind,
        strike_price=Price.from_str(f"{strike_price}"),
        activation_ns=activation_ns,
        expiration_ns=expiration_ns,
        ts_event=0,
        ts_init=0,
    )


def _get_precision(price_tick: float) -> int:
    """根据最小价格变动单位计算精度"""
    if price_tick <= 0:
        return 4
    s = f"{price_tick:.10f}".rstrip("0")
    if "." in s:
        return len(s.split(".")[1])
    return 0


def stock_code_to_instrument_id(stock_code: str) -> InstrumentId:
    """将 XtQuant 代码转换为 InstrumentId"""
    # 600000.SH -> 600000.THINKTRADER
    parts = stock_code.split(".")
    symbol = parts[0]
    # 可选: 使用 TT_VENUE 或根据市场使用不同 Venue
    return InstrumentId(Symbol(symbol), TT_VENUE)


def instrument_id_to_stock_code(instrument_id: InstrumentId, cache=None) -> str:
    """
    将 InstrumentId 转换为 XtQuant 代码
    
    如果提供 cache，尝试从缓存的 instrument 获取市场信息
    否则根据 symbol 首字符推断市场
    """
    symbol = str(instrument_id.symbol)
    
    # 尝试从 cache 获取市场信息
    if cache:
        instrument = cache.instrument(instrument_id)
        if instrument and hasattr(instrument, "exchange"):
            market = VENUE_TO_MARKET.get(str(instrument.exchange), "SH")
            return f"{symbol}.{market}"
    
    # 根据 symbol 首字符推断市场
    if symbol.startswith("6"):
        return f"{symbol}.SH"  # 上交所 A 股
    elif symbol.startswith(("0", "3")):
        return f"{symbol}.SZ"  # 深交所 A 股
    elif symbol.startswith("8") or symbol.startswith("4"):
        return f"{symbol}.BJ"  # 北交所
    elif len(symbol) <= 4:
        # 期货合约通常较短
        return f"{symbol}.IF"  # 默认中金所，需要更复杂的判断
    else:
        return f"{symbol}.SH"  # 默认
```

#### 3.4 实现 `providers.py`
工具提供者实现：

```python
# providers.py
import asyncio
from typing import Optional

from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.instruments import Instrument

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.parsing.instruments import (
    parse_equity,
    parse_future,
    parse_option,
    stock_code_to_instrument_id,
)


class ThinkTraderInstrumentProvider(InstrumentProvider):
    """ThinkTrader 工具提供者"""
    
    def __init__(
        self,
        client: ThinkTraderClient,
        config: ThinkTraderInstrumentProviderConfig,
    ):
        super().__init__()
        self._client = client
        self._config = config
        self._loaded = False
    
    async def load_all_async(self, filters: Optional[dict] = None) -> None:
        """异步加载所有工具"""
        if self._loaded and self._config.cache_instruments:
            return
        
        for sector in self._config.sectors:
            await self._load_sector(sector)
        
        self._loaded = True
        self._log.info(f"已加载 {len(self._instruments)} 个工具")
    
    async def _load_sector(self, sector: str) -> None:
        """加载板块内的工具"""
        stock_codes = self._client.get_stock_list(sector)
        
        for stock_code in stock_codes:
            try:
                detail = self._client.get_instrument_detail(stock_code)
                if detail is None:
                    continue
                
                instrument = self._parse_instrument(stock_code, detail)
                if instrument:
                    self.add(instrument)
            except Exception as e:
                self._log.warning(f"加载工具失败: {stock_code}, 错误: {e}")
            
            # 避免请求过快
            await asyncio.sleep(0.01)
    
    def _parse_instrument(self, stock_code: str, detail: dict) -> Optional[Instrument]:
        """解析工具"""
        instrument_id = stock_code_to_instrument_id(stock_code)
        
        # 根据合约类型解析
        instrument_type = self._client.get_instrument_type(stock_code)
        
        if instrument_type.get("stock") or instrument_type.get("fund"):
            return parse_equity(detail, instrument_id)
        elif instrument_type.get("future"):
            return parse_future(detail, instrument_id)
        elif instrument_type.get("option"):
            return parse_option(detail, instrument_id)
        else:
            return parse_equity(detail, instrument_id)  # 默认
    
    async def load_ids_async(
        self,
        instrument_ids: list[InstrumentId],
        filters: Optional[dict] = None,
    ) -> None:
        """按 ID 加载工具"""
        from nautilus_trader.adapters.thinktrader.parsing.instruments import (
            instrument_id_to_stock_code,
        )
        
        for instrument_id in instrument_ids:
            if self.find(instrument_id) is not None:
                continue
            
            stock_code = instrument_id_to_stock_code(instrument_id)
            detail = self._client.get_instrument_detail(stock_code)
            
            if detail:
                instrument = self._parse_instrument(stock_code, detail)
                if instrument:
                    self.add(instrument)
```

#### 3.3 实现 `data.py`
实现 `ThinkTraderDataClient`：

```python
# data.py
from nautilus_trader.live.data_client import LiveMarketDataClient
from nautilus_trader.model.identifiers import ClientId, InstrumentId

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.adapters.thinktrader.parsing.data import (
    parse_tick_to_quote_tick,
)
from nautilus_trader.adapters.thinktrader.parsing.instruments import (
    instrument_id_to_stock_code,
)

class ThinkTraderDataClient(LiveMarketDataClient):
    """ThinkTrader 数据客户端"""
    
    def __init__(
        self,
        loop,
        client: ThinkTraderClient,
        msgbus,
        cache,
        clock,
        instrument_provider,
        config,
    ):
        super().__init__(
            loop=loop,
            client_id=ClientId("THINKTRADER"),
            venue=TT_VENUE,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
        )
        self._client = client
        self._instrument_provider = instrument_provider
        self._config = config
        self._subscription_map: dict[InstrumentId, int] = {}
        
        # 注册回调
        self._client.register_event_handler("quote_data", self._on_quote_data)
    
    async def _connect(self) -> None:
        await self._client._connect()
    
    async def _disconnect(self) -> None:
        await self._client._disconnect()
    
    async def _subscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        stock_code = instrument_id_to_stock_code(instrument_id)
        seq = self._client.subscribe_quote(stock_code, period="tick")
        if seq > 0:
            self._subscription_map[instrument_id] = seq
    
    async def _unsubscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        if seq := self._subscription_map.pop(instrument_id, None):
            self._client.unsubscribe_quote(seq)
    
    def _on_quote_data(self, stock_code: str, data: dict) -> None:
        """处理行情回调"""
        from nautilus_trader.adapters.thinktrader.parsing.instruments import (
            stock_code_to_instrument_id,
        )
        
        instrument_id = stock_code_to_instrument_id(stock_code)
        ts_init = self._clock.timestamp_ns()
        
        quote_tick = parse_tick_to_quote_tick(instrument_id, data, ts_init)
        self._handle_data(quote_tick)
    
    async def _subscribe_bars(self, bar_type) -> None:
        """订阅 K 线"""
        from nautilus_trader.adapters.thinktrader.parsing.data import PERIOD_MAP
        
        stock_code = instrument_id_to_stock_code(bar_type.instrument_id)
        period = PERIOD_MAP.get(bar_type.spec.aggregation, "1m")
        
        seq = self._client.subscribe_quote(stock_code, period=period)
        if seq > 0:
            self._subscription_map[bar_type] = seq
    
    async def _request_bars(
        self,
        bar_type,
        limit: int,
        correlation_id,
        start=None,
        end=None,
    ) -> None:
        """请求历史 K 线"""
        from nautilus_trader.adapters.thinktrader.parsing.data import (
            parse_kline_to_bar,
            PERIOD_MAP,
        )
        
        stock_code = instrument_id_to_stock_code(bar_type.instrument_id)
        period = PERIOD_MAP.get(bar_type.spec.aggregation, "1d")
        
        start_time = start.strftime("%Y%m%d") if start else ""
        end_time = end.strftime("%Y%m%d") if end else ""
        
        data = self._client.get_market_data(
            stock_list=[stock_code],
            period=period,
            start_time=start_time,
            end_time=end_time,
            count=limit,
        )
        
        bars = []
        if stock_code in data:
            df = data[stock_code]
            ts_init = self._clock.timestamp_ns()
            for idx, row in df.iterrows():
                bar = parse_kline_to_bar(
                    bar_type.instrument_id,
                    bar_type,
                    row.to_dict(),
                    ts_init,
                )
                bars.append(bar)
        
        self._handle_bars(bar_type, bars, None, correlation_id)
```

---

### Phase 4: 交易执行适配

#### 4.1 实现 `parsing/execution.py`
订单状态映射：

> **参考**: `THINKTRADER_API_NOTES.md` Section 4.3-4.9, Section 7.7

```python
# parsing/execution.py
from xtquant import xtconstant
from nautilus_trader.model.enums import OrderStatus, OrderSide, OrderType, TimeInForce

# ============================================================================
# 订单状态映射 (来源: API_NOTES 4.5)
# ============================================================================
ORDER_STATUS_MAP = {
    xtconstant.ORDER_UNREPORTED: OrderStatus.SUBMITTED,      # 48: 未报
    xtconstant.ORDER_WAIT_REPORTING: OrderStatus.SUBMITTED,  # 49: 待报
    xtconstant.ORDER_REPORTED: OrderStatus.ACCEPTED,         # 50: 已报
    xtconstant.ORDER_REPORTED_CANCEL: OrderStatus.PENDING_CANCEL,  # 51: 已报待撤
    xtconstant.ORDER_PARTSUCC_CANCEL: OrderStatus.PARTIALLY_FILLED,  # 52: 部成待撤
    xtconstant.ORDER_PART_CANCEL: OrderStatus.CANCELED,      # 53: 部撤
    xtconstant.ORDER_CANCELED: OrderStatus.CANCELED,         # 54: 已撤
    xtconstant.ORDER_PART_SUCC: OrderStatus.PARTIALLY_FILLED,  # 55: 部成
    xtconstant.ORDER_SUCCEEDED: OrderStatus.FILLED,          # 56: 已成
    xtconstant.ORDER_JUNK: OrderStatus.REJECTED,             # 57: 废单
    xtconstant.ORDER_UNKNOWN: OrderStatus.DENIED,            # 255: 未知
}

# ============================================================================
# 股票委托类型映射 (来源: API_NOTES 4.3)
# ============================================================================
STOCK_ORDER_TYPE_MAP = {
    "BUY": xtconstant.STOCK_BUY,                   # 股票买入
    "SELL": xtconstant.STOCK_SELL,                 # 股票卖出
}

# ============================================================================
# 信用委托类型映射 (来源: API_NOTES 4.3)
# ============================================================================
CREDIT_ORDER_TYPE_MAP = {
    "MARGIN_BUY": xtconstant.CREDIT_FIN_BUY,            # 融资买入
    "MARGIN_SELL": xtconstant.CREDIT_DIRECT_SELL,       # 卖券还款
    "SHORT_SELL": xtconstant.CREDIT_SLO_SELL,           # 融券卖出
    "BUY_TO_COVER": xtconstant.CREDIT_BUY_SECU_REPAY,   # 买券还券
    "BUY_COLLATERAL": xtconstant.CREDIT_BUY,            # 担保品买入
    "SELL_COLLATERAL": xtconstant.CREDIT_SELL,          # 担保品卖出
}

# ============================================================================
# 期货委托类型映射 - 六键风格 (来源: API_NOTES 4.3)
# ============================================================================
FUTURES_SIX_KEY_ORDER_TYPE_MAP = {
    "OPEN_LONG": xtconstant.FUTURE_OPEN_LONG,              # 买开
    "CLOSE_LONG_HISTORY": xtconstant.FUTURE_CLOSE_LONG_HISTORY,  # 买平昨
    "CLOSE_LONG_TODAY": xtconstant.FUTURE_CLOSE_LONG_TODAY,      # 买平今
    "OPEN_SHORT": xtconstant.FUTURE_OPEN_SHORT,            # 卖开
    "CLOSE_SHORT_HISTORY": xtconstant.FUTURE_CLOSE_SHORT_HISTORY,  # 卖平昨
    "CLOSE_SHORT_TODAY": xtconstant.FUTURE_CLOSE_SHORT_TODAY,      # 卖平今
}

# ============================================================================
# 期货委托类型映射 - 四键风格 (来源: API_NOTES 4.3)
# ============================================================================
FUTURES_FOUR_KEY_ORDER_TYPE_MAP = {
    "OPEN_LONG": xtconstant.FUTURE_OPEN_LONG,       # 买开
    "CLOSE_LONG": xtconstant.FUTURE_CLOSE_LONG,     # 卖平 (自动优先平昨)
    "OPEN_SHORT": xtconstant.FUTURE_OPEN_SHORT,     # 卖开
    "CLOSE_SHORT": xtconstant.FUTURE_CLOSE_SHORT,   # 买平 (自动优先平昨)
}

# ============================================================================
# 期货委托类型映射 - 两键风格 (来源: API_NOTES 4.3)
# ============================================================================
FUTURES_TWO_KEY_ORDER_TYPE_MAP = {
    "SMART_BUY": xtconstant.FUTURE_SMART_BUY,            # 智能买入 (自动判断开/平)
    "SMART_SELL": xtconstant.FUTURE_SMART_SELL,          # 智能卖出 (自动判断开/平)
    "SMART_BUY_TODAY": xtconstant.FUTURE_SMART_BUY_TODAY,   # 智能买入平今优先
    "SMART_SELL_TODAY": xtconstant.FUTURE_SMART_SELL_TODAY, # 智能卖出平今优先
}

# ============================================================================
# 多空方向 (来源: API_NOTES 4.7)
# ============================================================================
DIRECTION_MAP = {
    "LONG": xtconstant.DIRECTION_FLAG_LONG,         # 48: 多
    "SHORT": xtconstant.DIRECTION_FLAG_SHORT,       # 49: 空
}

# ============================================================================
# 交易操作 / 开平标志 (来源: API_NOTES 4.8)
# ============================================================================
OFFSET_FLAG_MAP = {
    "OPEN": xtconstant.OFFSET_FLAG_OPEN,                  # 48: 开仓
    "CLOSE": xtconstant.OFFSET_FLAG_CLOSE,                # 49: 平仓
    "FORCE_CLOSE": xtconstant.OFFSET_FLAG_FORCECLOSE,     # 50: 强平
    "CLOSE_TODAY": xtconstant.OFFSET_FLAG_CLOSETODAY,     # 51: 平今
    "CLOSE_YESTERDAY": xtconstant.OFFSET_FLAG_ClOSEYESTERDAY,  # 52: 平昨
    "FORCE_OFF": xtconstant.OFFSET_FLAG_FORCEOFF,         # 53: 强减
    "LOCAL_FORCE_CLOSE": xtconstant.OFFSET_FLAG_LOCALFORCECLOSE,  # 54: 本地强平
}

# ============================================================================
# 报价类型映射 (来源: API_NOTES 4.4)
# ============================================================================
PRICE_TYPE_MAP = {
    # 通用
    xtconstant.FIX_PRICE: OrderType.LIMIT,          # 指定价
    xtconstant.LATEST_PRICE: OrderType.MARKET,      # 最新价
    xtconstant.MARKET_PEER_PRICE_FIRST: OrderType.MARKET,  # 对手方最优
}

# 上交所市价单类型
SH_MARKET_PRICE_TYPES = {
    "BEST_5_CANCEL": xtconstant.MARKET_SH_CONVERT_5_CANCEL,  # 最优五档即时成交剩余撤销
    "BEST_5_LIMIT": xtconstant.MARKET_SH_CONVERT_5_LIMIT,    # 最优五档即时成交剩余转限价
}

# 深交所市价单类型
SZ_MARKET_PRICE_TYPES = {
    "BEST_5_CANCEL": xtconstant.MARKET_SZ_CONVERT_5_CANCEL,  # 最优五档即时成交剩余撤销
    "BEST_5_LIMIT": xtconstant.MARKET_SZ_CONVERT_5_LIMIT,    # 最优五档即时成交剩余转限价
    "PEER_BEST": xtconstant.MARKET_PEER_PRICE_FIRST,         # 对手方最优价格
    "SELF_BEST": xtconstant.MARKET_MINE_PRICE_FIRST,         # 本方最优价格
    "FOK": xtconstant.MARKET_SZ_INSTBUSI_RESTCANCEL,         # 全额成交或撤销
}

# Nautilus OrderSide -> XtQuant 委托类型 (股票)
NAUTILUS_SIDE_TO_XT = {
    OrderSide.BUY: xtconstant.STOCK_BUY,
    OrderSide.SELL: xtconstant.STOCK_SELL,
}
```

#### 4.2 实现 `execution.py`
实现 `ThinkTraderExecutionClient`：

```python
# execution.py
from nautilus_trader.live.execution_client import LiveExecutionClient
from nautilus_trader.model.identifiers import AccountId, ClientOrderId, VenueOrderId
from nautilus_trader.model.enums import OrderSide

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.adapters.thinktrader.parsing.execution import (
    ORDER_STATUS_MAP,
    NAUTILUS_SIDE_TO_XT,
)
from nautilus_trader.adapters.thinktrader.parsing.instruments import (
    instrument_id_to_stock_code,
)

class ThinkTraderExecutionClient(LiveExecutionClient):
    """ThinkTrader 执行客户端"""
    
    def __init__(
        self,
        loop,
        client: ThinkTraderClient,
        msgbus,
        cache,
        clock,
        instrument_provider,
        config,
    ):
        super().__init__(
            loop=loop,
            client_id=ClientId("THINKTRADER"),
            venue=TT_VENUE,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
        )
        self._client = client
        self._instrument_provider = instrument_provider
        self._config = config
        
        # 订单 ID 映射
        self._client_order_id_to_order_id: dict[ClientOrderId, int] = {}
        self._order_id_to_client_order_id: dict[int, ClientOrderId] = {}
        
        # 注册回调
        self._client.register_event_handler("order_update", self._on_order_update)
        self._client.register_event_handler("trade", self._on_trade)
        self._client.register_event_handler("order_rejected", self._on_order_rejected)
    
    async def _connect(self) -> None:
        await self._client._connect()
    
    async def _disconnect(self) -> None:
        await self._client._disconnect()
    
    async def _submit_order(self, command) -> None:
        """提交订单"""
        order = command.order
        stock_code = instrument_id_to_stock_code(order.instrument_id)
        
        order_type = NAUTILUS_SIDE_TO_XT.get(order.side)
        price = float(order.price) if order.price else 0
        
        order_id = self._client.place_order(
            stock_code=stock_code,
            order_type=order_type,
            volume=int(order.quantity),
            price=price,
            order_remark=str(order.client_order_id),
        )
        
        if order_id > 0:
            self._client_order_id_to_order_id[order.client_order_id] = order_id
            self._order_id_to_client_order_id[order_id] = order.client_order_id
            self.generate_order_submitted(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                ts_event=self._clock.timestamp_ns(),
            )
        else:
            self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason=f"下单失败，返回值: {order_id}",
                ts_event=self._clock.timestamp_ns(),
            )
    
    async def _cancel_order(self, command) -> None:
        """取消订单"""
        order_id = self._client_order_id_to_order_id.get(command.client_order_id)
        if order_id:
            self._client.cancel_order(order_id)
        else:
            self._log.warning(f"未找到订单映射: {command.client_order_id}")
    
    def _on_order_update(self, order) -> None:
        """处理委托更新"""
        # order 是 XtOrder 对象
        order_id = order.order_id
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        
        if not client_order_id:
            # 可能是外部订单，尝试从 order_remark 恢复
            if order.order_remark:
                client_order_id = ClientOrderId(order.order_remark)
        
        if not client_order_id:
            self._log.warning(f"未知订单: order_id={order_id}")
            return
        
        status = ORDER_STATUS_MAP.get(order.order_status)
        cached_order = self._cache.order(client_order_id)
        
        if cached_order is None:
            self._log.warning(f"缓存中未找到订单: {client_order_id}")
            return
        
        ts_event = self._clock.timestamp_ns()
        
        # 根据状态生成相应事件
        if status == OrderStatus.ACCEPTED:
            if cached_order.status == OrderStatus.SUBMITTED:
                self.generate_order_accepted(
                    strategy_id=cached_order.strategy_id,
                    instrument_id=cached_order.instrument_id,
                    client_order_id=client_order_id,
                    venue_order_id=VenueOrderId(order.order_sysid),
                    ts_event=ts_event,
                )
        elif status == OrderStatus.CANCELED:
            self.generate_order_canceled(
                strategy_id=cached_order.strategy_id,
                instrument_id=cached_order.instrument_id,
                client_order_id=client_order_id,
                venue_order_id=VenueOrderId(order.order_sysid),
                ts_event=ts_event,
            )
        elif status == OrderStatus.REJECTED:
            self.generate_order_rejected(
                strategy_id=cached_order.strategy_id,
                instrument_id=cached_order.instrument_id,
                client_order_id=client_order_id,
                reason=order.status_msg or "废单",
                ts_event=ts_event,
            )
    
    def _on_trade(self, trade) -> None:
        """处理成交回报"""
        from nautilus_trader.model.identifiers import TradeId
        from nautilus_trader.model.objects import Price, Quantity, Money
        from nautilus_trader.adapters.thinktrader.parsing.instruments import (
            stock_code_to_instrument_id,
        )
        
        order_id = trade.order_id
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        
        if not client_order_id:
            # 尝试从 order_remark 恢复
            if trade.order_remark:
                client_order_id = ClientOrderId(trade.order_remark)
            else:
                self._log.warning(f"未知成交: order_id={order_id}")
                return
        
        cached_order = self._cache.order(client_order_id)
        if cached_order is None:
            self._log.warning(f"缓存中未找到订单: {client_order_id}")
            return
        
        instrument_id = stock_code_to_instrument_id(trade.stock_code)
        instrument = self._cache.instrument(instrument_id)
        
        ts_event = int(trade.traded_time) * 1_000_000_000 if trade.traded_time else self._clock.timestamp_ns()
        
        self.generate_order_filled(
            strategy_id=cached_order.strategy_id,
            instrument_id=cached_order.instrument_id,
            client_order_id=client_order_id,
            venue_order_id=VenueOrderId(trade.order_sysid),
            trade_id=TradeId(trade.traded_id),
            order_side=cached_order.side,
            order_type=cached_order.order_type,
            last_qty=Quantity.from_int(trade.traded_volume),
            last_px=Price.from_str(f"{trade.traded_price:.4f}"),
            quote_currency=instrument.quote_currency if instrument else Currency.from_str("CNY"),
            commission=Money(0, Currency.from_str("CNY")),  # 需要额外查询
            liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
            ts_event=ts_event,
        )
    
    def _on_order_rejected(self, order_id: int, reason: str) -> None:
        """处理订单拒绝"""
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        if not client_order_id:
            self._log.warning(f"未知的拒绝订单: order_id={order_id}")
            return
        
        cached_order = self._cache.order(client_order_id)
        if cached_order:
            self.generate_order_rejected(
                strategy_id=cached_order.strategy_id,
                instrument_id=cached_order.instrument_id,
                client_order_id=client_order_id,
                reason=reason,
                ts_event=self._clock.timestamp_ns(),
            )
```

---

### Phase 5: 工厂与集成

#### 5.1 实现 `factories.py`
```python
# factories.py
from nautilus_trader.live.factories import LiveDataClientFactory, LiveExecClientFactory

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.data import ThinkTraderDataClient
from nautilus_trader.adapters.thinktrader.execution import ThinkTraderExecutionClient
from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider

class ThinkTraderLiveDataClientFactory(LiveDataClientFactory):
    @staticmethod
    def create(
        loop,
        name,
        config,
        msgbus,
        cache,
        clock,
    ) -> ThinkTraderDataClient:
        client = ThinkTraderClient(
            loop=loop,
            logger=...,
            miniqmt_path=config.miniqmt_path,
            session_id=config.session_id,
            account_id=config.account_id,
        )
        provider = ThinkTraderInstrumentProvider(client=client, ...)
        return ThinkTraderDataClient(
            loop=loop,
            client=client,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=provider,
            config=config,
        )

class ThinkTraderLiveExecClientFactory(LiveExecClientFactory):
    @staticmethod
    def create(...) -> ThinkTraderExecutionClient:
        # 类似实现
        pass
```

#### 5.2 更新 `__init__.py`
导出公共接口：

```python
# __init__.py
from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.config import (
    ThinkTraderDataClientConfig,
    ThinkTraderExecClientConfig,
)
from nautilus_trader.adapters.thinktrader.data import ThinkTraderDataClient
from nautilus_trader.adapters.thinktrader.execution import ThinkTraderExecutionClient
from nautilus_trader.adapters.thinktrader.factories import (
    ThinkTraderLiveDataClientFactory,
    ThinkTraderLiveExecClientFactory,
)
from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider

__all__ = [
    "ThinkTraderClient",
    "ThinkTraderDataClientConfig",
    "ThinkTraderExecClientConfig",
    "ThinkTraderDataClient",
    "ThinkTraderExecutionClient",
    "ThinkTraderLiveDataClientFactory",
    "ThinkTraderLiveExecClientFactory",
    "ThinkTraderInstrumentProvider",
]
```

---

### Phase 6: 测试与文档

#### 6.1 单元测试
```
tests/unit_tests/adapters/thinktrader/
├── test_config.py
├── test_data_client.py
├── test_execution_client.py
├── test_parsing_data.py
├── test_parsing_execution.py
└── test_parsing_instruments.py
```

#### 6.2 集成测试
- 连接 MiniQmt 模拟环境
- 订阅行情并验证数据格式
- 下单/撤单流程验证

#### 6.3 文档
- 更新项目 README
- 添加使用示例
- 配置说明

---

## 6. 核心映射与转换

### 6.1 合约代码转换
| Nautilus InstrumentId | XtQuant stock_code | 说明 |
|----------------------|-------------------|------|
| `600000.SSE` | `600000.SH` | 上交所股票 |
| `000001.SZSE` | `000001.SZ` | 深交所股票 |
| `IF2401.CFFEX` | `IF2401.IF` | 中金所股指期货 |
| `AG2401.SHFE` | `AG2401.SF` | 上期所白银期货 |
| `M2401.DCE` | `M2401.DF` | 大商所豆粕期货 |
| `SR2401.CZCE` | `SR2401.ZF` | 郑商所白糖期货 |

### 6.2 市场代码映射
```python
# XtQuant 市场代码 (来自 交易模块文档.md)
MARKET_CODES = {
    "SH": "xtconstant.SH_MARKET",           # 上交所
    "SZ": "xtconstant.SZ_MARKET",           # 深交所
    "BJ": "xtconstant.MARKET_ENUM_BEIJING", # 北交所
    "SF": "xtconstant.MARKET_ENUM_SHANGHAI_FUTURE",  # 上期所
    "DF": "xtconstant.MARKET_ENUM_DALIANG_FUTURE",   # 大商所
    "ZF": "xtconstant.MARKET_ENUM_ZHENGZHOU_FUTURE", # 郑商所
    "IF": "xtconstant.MARKET_ENUM_INDEX_FUTURE",     # 中金所
    "INE": "xtconstant.MARKET_ENUM_INTL_ENERGY_FUTURE", # 能源中心
    "GF": "xtconstant.MARKET_ENUM_GUANGZHOU_FUTURE",    # 广期所
}
```

### 6.3 订单类型映射 (委托类型 order_type)
```python
# 来自 交易模块文档.md - 委托类型(order_type)
ORDER_TYPE_MAP = {
    # 股票
    OrderSide.BUY: xtconstant.STOCK_BUY,
    OrderSide.SELL: xtconstant.STOCK_SELL,
    
    # 期货六键风格
    # FUTURE_OPEN_LONG, FUTURE_CLOSE_LONG_HISTORY, FUTURE_CLOSE_LONG_TODAY
    # FUTURE_OPEN_SHORT, FUTURE_CLOSE_SHORT_HISTORY, FUTURE_CLOSE_SHORT_TODAY
}
```

### 6.4 报价类型映射 (price_type)
| Nautilus OrderType | XtQuant price_type | 说明 |
|-------------------|-------------------|------|
| `LIMIT` | `xtconstant.FIX_PRICE` | 指定价 |
| `MARKET` | `xtconstant.LATEST_PRICE` | 最新价 |
| `MARKET` (上交所) | `xtconstant.MARKET_SH_CONVERT_5_CANCEL` | 最优五档即时成交剩余撤销 |
| `MARKET` (深交所) | `xtconstant.MARKET_SZ_CONVERT_5_CANCEL` | 最优五档即时成交剩余撤销 |

### 6.5 订单状态映射 (order_status)
> **来源**: `交易模块文档.md` - 委托状态(order_status)

| 枚举变量 | 值 | 含义 | Nautilus OrderStatus |
|---------|---|------|---------------------|
| `ORDER_UNREPORTED` | 48 | 未报 | `SUBMITTED` |
| `ORDER_WAIT_REPORTING` | 49 | 待报 | `SUBMITTED` |
| `ORDER_REPORTED` | 50 | 已报 | `ACCEPTED` |
| `ORDER_REPORTED_CANCEL` | 51 | 已报待撤 | `PENDING_CANCEL` |
| `ORDER_PARTSUCC_CANCEL` | 52 | 部成待撤 | `PARTIALLY_FILLED` |
| `ORDER_PART_CANCEL` | 53 | 部撤 | `CANCELED` |
| `ORDER_CANCELED` | 54 | 已撤 | `CANCELED` |
| `ORDER_PART_SUCC` | 55 | 部成 | `PARTIALLY_FILLED` |
| `ORDER_SUCCEEDED` | 56 | 已成 | `FILLED` |
| `ORDER_JUNK` | 57 | 废单 | `REJECTED` |
| `ORDER_UNKNOWN` | 255 | 未知 | `DENIED` |

---

## 7. XtQuant 数据结构详解

> **参考**: `THINKTRADER_API_NOTES.md` Section 5 数据结构  
> **完整定义**: 所有数据结构的完整字段定义请参见 API 知识库

### 7.1 资产 XtAsset
| 属性 | 类型 | 说明 |
|-----|------|------|
| `account_type` | int | 账号类型 |
| `account_id` | str | 资金账号 |
| `cash` | float | 可用金额 |
| `frozen_cash` | float | 冻结金额 |
| `market_value` | float | 持仓市值 |
| `total_asset` | float | 总资产 |

### 7.2 委托 XtOrder
| 属性 | 类型 | 说明 |
|-----|------|------|
| `order_id` | int | 订单编号 (本地 ID) |
| `order_sysid` | str | 柜台合同编号 |
| `stock_code` | str | 证券代码，如 "600000.SH" |
| `order_type` | int | 委托类型 |
| `order_volume` | int | 委托数量 |
| `traded_volume` | int | 成交数量 |
| `price` | float | 委托价格 |
| `traded_price` | float | 成交均价 |
| `order_status` | int | 委托状态 |
| `status_msg` | str | 委托状态描述 |
| `order_remark` | str | 委托备注 (可存储 ClientOrderId) |
| `direction` | int | 多空方向 (期货) |
| `offset_flag` | int | 交易操作 (开/平仓) |

### 7.3 成交 XtTrade
| 属性 | 类型 | 说明 |
|-----|------|------|
| `order_id` | int | 订单编号 |
| `order_sysid` | str | 柜台合同编号 |
| `traded_id` | str | 成交编号 |
| `stock_code` | str | 证券代码 |
| `traded_price` | float | 成交价格 |
| `traded_volume` | int | 成交数量 |
| `traded_amount` | float | 成交金额 |
| `traded_time` | int | 成交时间 |

### 7.4 持仓 XtPosition
| 属性 | 类型 | 说明 |
|-----|------|------|
| `stock_code` | str | 证券代码 |
| `volume` | int | 持仓数量 |
| `can_use_volume` | int | 可用数量 |
| `open_price` | float | 开仓价 |
| `avg_price` | float | 成本价 |
| `market_value` | float | 市值 |
| `frozen_volume` | int | 冻结数量 |
| `yesterday_volume` | int | 昨夜拥股 |

### 7.5 Tick 数据字段
> **来源**: `行情模块文档.md` - tick分笔数据

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 (毫秒) |
| `lastPrice` | 最新价 |
| `open` | 开盘价 |
| `high` | 最高价 |
| `low` | 最低价 |
| `lastClose` | 前收盘价 |
| `amount` | 成交总额 |
| `volume` | 成交总量 |
| `bidPrice` | 委买价 (数组) |
| `askPrice` | 委卖价 (数组) |
| `bidVol` | 委买量 (数组) |
| `askVol` | 委卖量 (数组) |

### 7.6 合约详情字段
> **来源**: `行情模块文档.md` - get_instrument_detail

| 字段 | 说明 |
|-----|------|
| `InstrumentID` | 合约代码 |
| `InstrumentName` | 合约名称 |
| `ExchangeID` | 市场代码 |
| `PriceTick` | 最小价格变动单位 |
| `VolumeMultiple` | 合约乘数 |
| `UpStopPrice` | 涨停价 |
| `DownStopPrice` | 跌停价 |
| `PreClose` | 前收盘价 |
| `FloatVolume` | 流通股本 |
| `TotalVolume` | 总股本 |

---

## 8. 注意事项与风险点

### 8.1 线程安全 ⚠️
XtQuant 的回调在独立线程中触发，**必须**使用 `loop.call_soon_threadsafe()` 将处理逻辑调度回 Nautilus 的 asyncio 主循环。

```python
# 正确做法
def on_stock_order(self, order):
    self._loop.call_soon_threadsafe(self._handle_order, order)

# 错误做法 - 会导致线程安全问题
def on_stock_order(self, order):
    self._handle_order(order)  # 直接调用
```

### 8.2 MiniQmt 依赖
- **Windows**: MiniQmt 必须已启动并登录
- **Linux**: 需要通过 Wine 或 Windows 虚拟机运行 MiniQmt
- **路径配置**: 必须正确指定 `userdata_mini` 路径

### 8.3 账号类型区分
```python
# 根据 交易模块文档.md - 账号类型(account_type)
ACCOUNT_TYPES = {
    "STOCK": xtconstant.SECURITY_ACCOUNT,      # 股票
    "CREDIT": xtconstant.CREDIT_ACCOUNT,       # 信用
    "FUTURE": xtconstant.FUTURE_ACCOUNT,       # 期货
    "OPTION": xtconstant.STOCK_OPTION_ACCOUNT, # 期权
}
```

### 8.4 数据精度
- A股价格精度: 2 位小数 (0.01)
- 期货价格精度: 根据品种不同
- 使用 `get_instrument_detail` 获取 `PriceTick` 确定精度

### 8.5 订单 ID 管理
```
订单生命周期:
1. place_order() 返回 order_id (本地 ID)
2. on_stock_order 回调中获取 order_sysid (柜台 ID)
3. 需要维护三者映射: ClientOrderId <-> order_id <-> order_sysid
```

### 8.6 连接状态管理
```python
# 连接断开后不会自动重连
connect_result = xt_trader.connect()
if connect_result != 0:
    # 需要手动处理重连逻辑
```

### 8.7 市价单限制
> 市价类型只在实盘环境中生效，模拟环境不支持市价方式报单

---

## 9. 参考资料

### 核心参考 (必读) ⭐
- **`THINKTRADER_API_NOTES.md`** (v1.4) - XtQuant API 知识库，包含:
  - 41个行情接口 + 46个交易接口完整列表
  - 9个数据字典枚举 (市场/账号/委托/报价/状态等)
  - 18个交易数据结构详解
  - 11种行情数据结构 (含Level2)
  - 5个行情数据字典 (证券状态/委托类型/方向/成交标志等)
  - Nautilus Trader 适配要点
  - 10个常见问题解答

### 本地文档 (原始文档)
- `交易模块文档.md` - XtQuant 交易模块完整 API 文档
- `行情模块文档.md` - XtQuant 行情模块完整 API 文档

### 在线文档
- [XtQuant xtdata 文档](https://dict.thinktrader.net/nativeApi/xtdata.html)
- [XtQuant xttrader 文档](https://dict.thinktrader.net/nativeApi/xttrader.html)
- [XtQuant 枚举常量](https://dict.thinktrader.net/innerApi/enum_constants.html)

### Nautilus Trader 参考
- [Interactive Brokers Adapter](../interactive_brokers/) - 作为实现参考
- [Nautilus Trader 官方文档](https://nautilustrader.io/)
- [Live Data Client](https://docs.nautilustrader.io/api_reference/live.html#livedataclient)
- [Live Execution Client](https://docs.nautilustrader.io/api_reference/live.html#liveexecutionclient)
