# ThinkTrader Adapter 开发手册

## 目录

1. [架构总览](#1-架构总览)
2. [分层设计](#2-分层设计)
3. [底层客户端 (ThinkTraderClient)](#3-底层客户端-thinktraderclient)
   - 3.1 [Mixin 架构](#31-mixin-架构)
   - 3.2 [BaseMixin 与类型契约](#32-basemixin-与类型契约)
   - 3.3 [回调分发机制](#33-回调分发机制)
   - 3.4 [请求与订阅管理](#34-请求与订阅管理)
4. [行情数据层](#4-行情数据层)
   - 4.1 [DataClient 与 MarketDataMixin 的职责划分](#41-dataclient-与-marketdatamixin-的职责划分)
   - 4.2 [订阅生命周期](#42-订阅生命周期)
   - 4.3 [数据流转链路](#43-数据流转链路)
   - 4.4 [历史数据分块请求](#44-历史数据分块请求)
5. [交易执行层](#5-交易执行层)
   - 5.1 [订单 ID 映射体系](#51-订单-id-映射体系)
   - 5.2 [下单流程 (同步/异步)](#52-下单流程-同步异步)
   - 5.3 [撤单流程与多级回退](#53-撤单流程与多级回退)
   - 5.4 [回报处理 — 事件驱动链](#54-回报处理--事件驱动链)
   - 5.5 [持仓协调 (Reconciliation)](#55-持仓协调-reconciliation)
   - 5.6 [资金与持仓推送](#56-资金与持仓推送)
6. [数据解析层 (Parsing)](#6-数据解析层-parsing)
   - 6.1 [行情数据解析](#61-行情数据解析)
   - 6.2 [执行回报解析与状态映射](#62-执行回报解析与状态映射)
   - 6.3 [证券信息解析与代码转换](#63-证券信息解析与代码转换)
7. [配置与工厂](#7-配置与工厂)
   - 7.1 [配置数据类](#71-配置数据类)
   - 7.2 [工厂与单例缓存](#72-工厂与单例缓存)
8. [证券信息提供者](#8-证券信息提供者)
9. [历史数据客户端](#9-历史数据客户端)
10. [线程安全模型](#10-线程安全模型)
11. [扩展指南](#11-扩展指南)
    - 11.1 [添加新的市场类型支持](#111-添加新的市场类型支持)
    - 11.2 [添加新的订单类型支持](#112-添加新的订单类型支持)
    - 11.3 [添加新的数据类型支持](#113-添加新的数据类型支持)
    - 11.4 [添加新的 Mixin 功能](#114-添加新的-mixin-功能)
12. [设计决策与取舍](#12-设计决策与取舍)
13. [附录：核心类型关系图](#13-附录核心类型关系图)

---

## 1. 架构总览

ThinkTrader Adapter 将 NautilusTrader 框架与中国券商的 MiniQmt (xtquant) 交易系统集成。设计遵循以下原则：

- **分层解耦** — 底层 SDK 交互 → 数据解析 → NautilusTrader 接口适配，各层单向依赖
- **Mixin 组合** — 底层客户端通过多继承将连接、行情、账户、订单、合约、错误处理拆分为独立 Mixin
- **事件驱动** — xtquant 回调线程的事件通过 `call_soon_threadsafe` 安全注入 asyncio 事件循环
- **单例共享** — DataClient 与 ExecClient 通过工厂缓存共享同一底层 `ThinkTraderClient` 实例

```
                    NautilusTrader 框架
    ┌────────────────────────────────────────────┐
    │  Strategy ←→ DataEngine ←→ ExecEngine      │
    └──────┬──────────────────────────┬──────────┘
           │                          │
 ┌─────────▼──────────┐   ┌──────────▼───────────┐
 │ ThinkTraderData    │   │ ThinkTraderExecution │
 │ Client (data.py)   │   │ Client (execution.py)│
 └─────────┬──────────┘   └──────────┬───────────┘
           │    ▲ parsing/             │    ▲ parsing/
           │    │ data.py              │    │ execution.py
           │    │ instruments.py       │    │ instruments.py
           ▼                           ▼
 ┌─────────────────────────────────────────────────┐
 │           ThinkTraderClient (client/)           │
 │  Connection │ MarketData │ Account │ Order      │
 │  Contract   │ Error      │ Callback             │
 └──────────────────────┬──────────────────────────┘
                        │
             ┌──────────▼──────────┐
             │  xtquant SDK        │
             │  XtQuantTrader      │
             │  xtdata API         │
             └──────────┬──────────┘
                        │
             ┌──────────▼──────────┐
             │     券商柜台         │
             └─────────────────────┘
```

---

## 2. 分层设计

适配器分为四层，每层有明确的职责边界：

| 层级 | 模块 | 职责 |
|------|------|------|
| **Layer 4: 接口层** | `data.py`, `execution.py`, `providers.py` | 继承 `LiveDataClient` / `LiveExecutionClient`，面向 NautilusTrader 引擎 |
| **Layer 3: 解析层** | `parsing/data.py`, `parsing/execution.py`, `parsing/instruments.py` | XtQuant 原始数据 → Nautilus 数据模型。纯函数，无状态，无副作用 |
| **Layer 2: 客户端层** | `client/` (6 Mixin + 聚合类 + Callback) | 封装 XtQuantTrader / xtdata API，管理连接、订阅、请求生命周期 |
| **Layer 1: SDK 层** | `xtquant` (XtQuantTrader + xtdata) | 券商提供的交易与行情 SDK |

**依赖方向**：Layer 4 → Layer 3 → Layer 2 → Layer 1。Layer 3 是纯函数层，可独立测试。

---

## 3. 底层客户端 (ThinkTraderClient)

### 3.1 Mixin 架构

`ThinkTraderClient` 通过 Python 多继承聚合 6 个 Mixin，每个 Mixin 封装一组相关功能：

```python
# client/client.py
class ThinkTraderClient(
    ThinkTraderClientConnectionMixin,    # 连接管理
    ThinkTraderClientMarketDataMixin,    # 行情数据
    ThinkTraderClientAccountMixin,       # 账户/持仓查询
    ThinkTraderClientOrderMixin,         # 下单/撤单
    ThinkTraderClientContractMixin,      # 合约查询
    ThinkTraderClientErrorMixin,         # 错误处理
):
    ...
```

| Mixin | 文件 | 核心方法 | 依赖的 SDK API |
|-------|------|---------|----------------|
| `ConnectionMixin` | `connection.py` | `_connect()`, `_disconnect()`, `subscribe()`, `unsubscribe()`, `_check_connection()` | `XtQuantTrader.connect/start/stop` |
| `MarketDataMixin` | `market_data.py` | `subscribe_ticks()`, `subscribe_market_data()`, `subscribe_order_book()`, `subscribe_realtime_bars()`, `get_historical_bars()`, `get_historical_ticks()`, `subscribe_whole_quote()`, `download_history_data()` | `xtdata.subscribe_quote`, `xtdata.get_market_data`, `xtdata.download_history_data` |
| `AccountMixin` | `account.py` | `query_asset()`, `query_positions()`, `query_orders()`, `query_trades()`, `query_orders_async()`, `query_trades_async()` | `XtQuantTrader.query_stock_*` |
| `OrderMixin` | `order.py` | `place_order()`, `place_order_async()`, `cancel_order()`, `cancel_order_async()`, `cancel_order_by_sysid()`, `cancel_order_by_sysid_async()`, `set_relaxed_response_order_enabled()` | `XtQuantTrader.order_stock*`, `cancel_order_stock*` |
| `ContractMixin` | `contract.py` | `get_instrument_detail()`, `get_instrument_type()`, `get_stock_list()`, `get_trading_dates()` | `xtdata.get_instrument_detail`, `xtdata.get_stock_list_in_sector`, `xtdata.get_trading_dates` |
| `ErrorMixin` | `error.py` | `_handle_order_error()`, `_handle_cancel_error()`, `_handle_connection_error()` | — |

### 3.2 BaseMixin 与类型契约

所有 Mixin 继承自 `BaseMixin` (ABC)，它定义了 Mixin 可安全访问的共享属性：

```python
# client/common.py
class BaseMixin(ABC):
    _loop: asyncio.AbstractEventLoop      # asyncio 事件循环
    _log: Any                             # Logger 实例
    _trader: Any                          # XtQuantTrader 实例
    _account: Any                         # StockAccount 实例
    _miniqmt_path: str                    # MiniQmt userdata_mini 路径
    _session_id: int                      # 会话 ID
    _account_id: str                      # 资金账号
    _account_type: str                    # 账号类型
    _callback: Any                        # XtQuantTraderCallback
    _is_connected: asyncio.Event          # 连接状态标志
    _event_handlers: dict[str, Any]       # 事件处理器注册表
    _subscriptions: Subscriptions         # 订阅管理器
    _requests: Requests                   # 请求管理器
    _req_id: int                          # 请求 ID 计数器 (起始值 1000)
```

> **重要**：`BaseMixin` 使用类型注解而非实例变量定义。这些属性由 `ThinkTraderClient.__init__()` 统一初始化，各 Mixin 通过 Python MRO (Method Resolution Order) 访问。这是一个**隐式契约** — 新增 Mixin 必须仅访问 `BaseMixin` 中声明的属性。

### 3.3 回调分发机制

xtquant SDK 在**独立线程**中触发回调，适配器通过三级分发将事件安全注入 asyncio 事件循环：

| 级别 | 组件 | 线程 | 职责 |
|------|------|------|------|
| **Level 1** | `ThinkTraderClientCallback.on_*()` | xtquant 回调线程 | 接收 SDK 原始回调，转发至 `ThinkTraderClient` 对应方法 |
| **Level 2** | `ThinkTraderClient._on_*()` | xtquant → asyncio | 通过 `loop.call_soon_threadsafe()` 将处理函数调度到主线程 |
| **Level 3** | `ThinkTraderClient._handle_*()` | asyncio 主线程 | 查找并调用 `_event_handlers` 中注册的处理器 |

```python
# Level 1: xtquant 回调线程中执行
class ThinkTraderClientCallback(XtQuantTraderCallback):
    def on_stock_order(self, order):
        self._client._on_order(order)

# Level 2: 跨线程调度
class ThinkTraderClient:
    def _on_order(self, order):
        self._loop.call_soon_threadsafe(self._handle_order_update, order)

    # Level 3: asyncio 主线程中执行
    def _handle_order_update(self, order):
        if handler := self._event_handlers.get("order_update"):
            handler(order)
```

**完整回调映射表**：

| Callback 方法 | Client 方法 | 事件名称 |
|---------------|------------|----------|
| `on_disconnected()` | `_on_disconnected()` | `"disconnected"` |
| `on_account_status(status)` | `_on_account_status()` | *(直接日志记录)* |
| `on_stock_order(order)` | `_on_order()` → `_handle_order_update()` | `"order_update"` |
| `on_stock_trade(trade)` | `_on_trade()` → `_handle_trade()` | `"trade"` |
| `on_stock_asset(asset)` | `_on_asset()` → `_handle_asset()` | `"asset_update"` |
| `on_stock_position(position)` | `_on_position()` → `_handle_position()` | `"position_update"` |
| `on_order_error(error)` | `_handle_order_error()` | `"order_rejected"` |
| `on_cancel_error(error)` | `_handle_cancel_error()` | `"cancel_rejected"` |
| `on_order_stock_async_response(resp)` | `_on_order_async_response()` | `"order_async_response"` |
| `on_cancel_order_stock_async_response(resp)` | `_on_cancel_async_response()` | `"cancel_async_response"` |
| `on_cancel_order_stock_sysid_async_response(resp)` | `_on_cancel_async_response()` | `"cancel_async_response"` |

**ExecutionClient 中的事件处理器注册** (`execution.py.__init__`)：

```python
self._client.register_event_handler("order_update", self._on_order_update)
self._client.register_event_handler("trade", self._on_trade)
self._client.register_event_handler("order_rejected", self._on_order_rejected)
self._client.register_event_handler("order_async_response", self._on_order_async_response)
self._client.register_event_handler("cancel_async_response", self._on_cancel_async_response)
self._client.register_event_handler("asset_update", self._on_asset_update)
self._client.register_event_handler("position_update", self._on_position_update)
self._client.register_event_handler("cancel_rejected", self._on_cancel_rejected)
```

### 3.4 请求与订阅管理

底层客户端通过 `Requests` 和 `Subscriptions` 两个容器类统一管理异步请求和实时订阅，二者均继承自 `Base` 抽象基类：

```
              ┌──────────────┐
              │   Base (ABC) │
              │ req_id_to_*  │
              └──────┬───────┘
           ┌─────────┴─────────┐
    ┌──────▼──────┐     ┌──────▼──────┐
    │  Requests   │     │Subscriptions│
    │  + future   │     │ + last      │
    │  + result   │     │             │
    └─────────────┘     └─────────────┘
```

**`Request`** (一次性异步数据请求)：

```python
class Request(msgspec.Struct, frozen=True):
    req_id: int              # 请求 ID (>0)
    name: str | tuple        # 请求标识 (用于去重)
    handle: Callable         # 执行请求的函数
    cancel: Callable         # 取消请求的函数
    future: asyncio.Future   # 异步结果等待
    result: list[Any]        # 结果累积列表
```

**`Subscription`** (持续的数据订阅)：

```python
class Subscription(msgspec.Struct, frozen=True):
    req_id: int              # 订阅 ID (>0)
    name: str | tuple        # 订阅标识 (用于去重)
    handle: partial | Callable  # 重新订阅时使用的函数
    cancel: Callable         # 取消订阅的函数
    last: Any                # 最近一次收到的值
```

**关键数据结构** (`client/common.py`)：

| 结构体 | 用途 | 核心字段 |
|--------|------|----------|
| `TTPosition` | 持仓信息容器 | `account_id`, `stock_code`, `volume`, `available_volume`, `avg_price`, `market_value`, `float_pnl` |
| `TTOrder` | 委托信息容器 | `order_id`, `order_sysid`, `stock_code`, `order_type`, `order_volume`, `traded_volume`, `price`, `traded_price`, `order_status`, `order_remark` |
| `TTTrade` | 成交信息容器 | `account_id`, `order_id`, `order_sysid`, `stock_code`, `traded_id`, `traded_price`, `traded_volume`, `traded_time` |

**请求去重**：`Base._validation_check()` 阻止相同 `req_id` 或 `name` 的重复注册，防止资源泄漏。

---

## 4. 行情数据层

### 4.1 DataClient 与 MarketDataMixin 的职责划分

| 职责 | 模块 | 角色 |
|------|------|------|
| 实现 NautilusTrader 的 `LiveMarketDataClient` 接口 | `data.py` | 面向上层的适配 |
| `InstrumentId` ↔ `stock_code` 双向转换 | `data.py` | 调用 `parsing/instruments.py` |
| xtdata API 调用 | `client/market_data.py` | 底层 SDK 封装 |
| 原始数据 → Nautilus 数据模型解析 | `parsing/data.py` | 纯函数转换 |

`ThinkTraderDataClient` (`data.py`) 的核心职责：

1. 将 NautilusTrader 的 `subscribe_quote_ticks(InstrumentId)` 转换为 `client.subscribe_ticks(instrument_id, stock_code)`
2. 历史数据请求的分块处理 (chunked)，避免单次请求数据量过大
3. 注册 `"data"` 事件处理器，将解析后的数据路由到 DataEngine

**DataClient 支持的订阅类型**：

| 方法 | 数据类型 | 底层 Mixin 方法 |
|------|----------|----------------|
| `_subscribe_quote_ticks()` | `QuoteTick` | `subscribe_ticks()` |
| `_subscribe_trade_ticks()` | `TradeTick` | `subscribe_market_data()` |
| `_subscribe_order_book_deltas()` | `OrderBookDelta` | `subscribe_order_book()` |
| `_subscribe_order_book_depth()` | `OrderBookDelta` | `subscribe_order_book()` |
| `_subscribe_bars()` | `Bar` | `subscribe_realtime_bars()` |
| `_request_bars()` | `list[Bar]` | `get_historical_bars_chunked()` |
| `_request_quote_ticks()` | `list[QuoteTick]` | `get_historical_ticks_chunked()` |
| `_request_trade_ticks()` | `list[TradeTick]` | `get_historical_ticks_chunked()` |

### 4.2 订阅生命周期

**订阅方法的统一入口** (`MarketDataMixin._subscribe`)：

```python
async def _subscribe(self, name, subscription_method, cancellation_method, *args, **kwargs):
    subscription = self._subscriptions.get(name=name)
    if subscription is None:
        handle_func = functools.partial(subscription_method, *args, **kwargs)
        seq = handle_func()                           # 执行订阅
        if seq <= 0:
            raise RuntimeError(f"XtQuant 订阅失败: {name}")
        subscription = self._subscriptions.add(       # 注册到管理器
            req_id=seq, name=name, handle=handle_func, ...
        )
    return subscription                               # 重复订阅返回已有实例 (幂等)
```

设计要点：
- **幂等性**：重复订阅同一标的不会创建新连接
- **handle 保存**：`functools.partial` 被保存到 `Subscription.handle`，支持重新订阅
- **取消函数绑定**：`cancel` 绑定了具体的 `seq` ID，确保精确取消

### 4.3 数据流转链路

以 QuoteTick 订阅为例：

```
xtdata 回调线程                           asyncio 主线程
    │                                        │
    │ xtdata.subscribe_quote() 触发回调       │
    ▼                                        │
_on_quote_data(datas, name)                  │
    │ for stock_code, data_list              │
    │   call_soon_threadsafe ───────────────►│
    │                          _handle_quote_data(name, stock_code, data)
    │                              │
    │                              ├─ name 是 tuple → 解析 (instrument_id, data_type)
    │                              │   ├─ "tick"          → parse_tick_to_quote_tick
    │                              │   │                    + parse_tick_to_trade_tick
    │                              │   ├─ "market_data"   → parse_tick_to_quote_tick
    │                              │   ├─ "order_book"    → parse_l2_quote_to_order_book_deltas
    │                              │   ├─ "l2order"       → parse_l2_order_to_delta
    │                              │   └─ "l2transaction" → parse_l2_transaction_to_trade_tick
    │                              │
    │                              ├─ name 是 str → 解析为 BarType
    │                              │   └─ parse_kline_to_bar
    │                              │
    │                              └─ name 是 None (全推模式)
    │                                  └─ 从 cache 查找 instrument_id → parse_tick_to_quote_tick
    │                              
    │                              └─ _forward_data(parsed) → event_handlers["data"]
    │                                                           → DataClient._on_client_data
    │                                                             → self._handle_data(data)
```

### 4.4 历史数据分块请求

`DataClient` 对历史数据请求实现了自动分块，避免单次 xtdata API 调用的数据量过大：

```python
# data.py
async def get_historical_bars_chunked(self, *, bar_type, stock_code, start_ns, end_ns, timeout=60):
    period = bar_spec_to_period(bar_type.spec)
    chunk_ns = self._default_chunk_ns_for_period(period)
    chunks = self._iter_time_chunks_ns(start_ns, end_ns, chunk_ns)

    seen: set[int] = set()       # 基于 ts_event 去重
    out: list[Bar] = []

    for chunk_start_ns, chunk_end_ns in chunks:
        await self._client.download_history_data(...)   # 先触发 xtdata 本地缓存下载
        data = await self._client.get_historical_bars(...)
        bars = self._parse_historical_bars_dict(...)
        for bar in bars:
            if bar.ts_event not in seen:
                seen.add(bar.ts_event)
                out.append(bar)

    out.sort(key=lambda b: b.ts_event)
    return out
```

**分块时间窗口策略**（`_default_chunk_ns_for_period`）：

| 周期 | 分块大小 |
|------|----------|
| `tick` | 1 天 |
| 分钟级 (`*m`) | 31 天 |
| 小时级 (`*h`) | 180 天 |
| `1d` | 730 天 |
| `1w` / `1mon` | 3650 天 |

---

## 5. 交易执行层

### 5.1 订单 ID 映射体系

适配器需维护 NautilusTrader 本地 ID 与 xtquant 柜台 ID 之间的映射：

```python
# execution.py 中的 5 个映射字典
self._client_order_id_to_order_id: dict[ClientOrderId, int] = {}
    # NautilusTrader ClientOrderId → xtquant order_id (整数)

self._order_id_to_client_order_id: dict[int, ClientOrderId] = {}
    # xtquant order_id → ClientOrderId (反向映射)

self._order_seq_to_client_order_id: dict[int, ClientOrderId] = {}
    # 异步下单序号 seq → ClientOrderId

self._cancel_seq_to_client_order_id: dict[int, ClientOrderId] = {}
    # 异步撤单序号 seq → ClientOrderId

self._submitted_orders: dict[ClientOrderId, Any] = {}
    # 已提交但尚未被 Cache 追踪的订单 (解决竞态条件)
```

**`_submitted_orders` 的必要性**：

异步下单场景下存在时序竞争：

```
t1: place_order_async() → 返回 seq
t2: xtquant 回调 on_stock_order() → _on_order_update() 被触发
t3: NautilusTrader Cache 才完成订单注册

问题：在 t2 时 Cache 可能尚未识别该订单，
      self._cache.order(client_order_id) 返回 None
```

`_submitted_orders` 作为临时暂存区：在 `_submit_order` 时写入，在 `_on_order_update` 和 `_on_trade` 中作为 fallback 使用：

```python
cached_order = self._cache.order(client_order_id) or self._submitted_orders.get(client_order_id)
```

### 5.2 下单流程 (同步/异步)

```
_submit_order(command)
    │
    ├─ 订单方向检查: NAUTILUS_SIDE_TO_XT.get(order.side)
    │  └─ 不支持 → generate_order_rejected
    │
    ├─ 订单类型检查: order.order_type
    │  └─ 非 LIMIT/MARKET → generate_order_rejected
    │
    ├─ 参数构建:
    │  ├─ LIMIT  → price=order.price, price_type=FIX_PRICE
    │  └─ MARKET → price=0.0, price_type=MARKET_PEER_PRICE_FIRST
    │
    ├─ order_remark=str(order.client_order_id)  ← 关键：用于回调中恢复映射
    │
    └─ use_async_order?
       ├─ True  → place_order_async() → seq
       │  ├─ seq > 0: 记录 seq→ClientOrderId + generate_order_submitted
       │  └─ seq ≤ 0: generate_order_rejected
       │
       └─ False → place_order() → order_id
          ├─ order_id > 0: 建立双向 ID 映射 + generate_order_submitted
          └─ order_id ≤ 0: generate_order_rejected
```

**同步 vs 异步对比**：

| 特性 | 同步 (`place_order`) | 异步 (`place_order_async`) |
|------|---------------------|-----------------------------|
| 返回值 | `order_id` (柜台订单号) | `seq` (请求序号) |
| ID 映射时机 | 立即建立 | 在 `_on_order_async_response` 回调中建立 |
| 阻塞 | 阻塞至柜台响应 | 不阻塞 |
| 推荐场景 | 调试/测试 | **生产环境 (默认)** |

**异步下单的 ID 映射建立过程**：

```
t1: place_order_async() → seq=12345
    _order_seq_to_client_order_id[12345] = "O-20260210-001"

t2: on_order_stock_async_response(response)    // response.seq=12345, order_id=8888
    _on_order_async_response():
        client_order_id = _order_seq_to_client_order_id[12345]
        _client_order_id_to_order_id["O-20260210-001"] = 8888
        _order_id_to_client_order_id[8888] = "O-20260210-001"

t3: 后续 on_stock_order / on_stock_trade 通过 order_id=8888 查找 client_order_id
```

**`_modify_order`**：当前不支持修改订单，直接返回 `generate_order_modify_rejected`，原因为"ThinkTrader 暂不支持修改订单, 请撤单后重下"。

### 5.3 撤单流程与多级回退

撤单逻辑实现了**四级回退链**，确保各种情况下均可尝试撤单：

```
_cancel_order(command)
    │
    ├─ 缓存中无订单 → generate_order_cancel_rejected
    │
    ├─ 订单已关闭 (is_closed) → 跳过 (幂等)
    │
    ├─ Level 1: 内存映射
    │  └─ _client_order_id_to_order_id 中找到 order_id → _cancel_order_id()
    │
    ├─ Level 2: 查询柜台匹配
    │  └─ _find_order_id_from_orders():
    │     遍历 query_orders() 结果，通过 order_sysid 或 order_remark 匹配
    │     └─ 找到 → _cancel_order_id()
    │
    ├─ Level 3: 系统编号撤单
    │  └─ _try_cancel_by_sysid():
    │     通过 venue_order_id (order_sysid) + market 调用 cancel_order_stock_sysid
    │     └─ 成功 → 完成
    │
    └─ Level 4: 报告失败 → generate_order_cancel_rejected
```

**`_cancel_order_id` 内部**也区分同步与异步：

```python
def _cancel_order_id(self, *, order_id, client_order_id):
    if self._config.use_async_cancel:
        seq = self._client.cancel_order_async(order_id)
        if seq <= 0:
            self._on_cancel_rejected(order_id, ...)
            return
        self._cancel_seq_to_client_order_id[seq] = client_order_id
    else:
        result = self._client.cancel_order(order_id)
        if result <= 0:
            self._on_cancel_rejected(order_id, ...)
```

**`_try_cancel_by_sysid` 的 API 回退**：

`cancel_order_by_sysid_async` 在部分 xtquant 版本中不可用。`OrderMixin` 中对此做了防御性处理：

```python
def cancel_order_by_sysid_async(self, market, order_sysid):
    method = getattr(self._trader, "cancel_order_stock_sysid_async", None)
    if method is None:
        return self.cancel_order_by_sysid(market=market, order_sysid=order_sysid)  # 回退到同步
    ...
```

**`_cancel_all_orders`**：遍历 `query_orders()` 结果，对所有未终结的委托逐一执行撤单。

### 5.4 回报处理 — 事件驱动链

#### `_on_order_update(order)` — 委托状态变更

```
XtOrder → _on_order_update
    │
    ├─ 从 order_id 查找 client_order_id
    │   └─ 失败则使用 order.order_remark 恢复
    │
    ├─ 查找 cached_order (Cache 优先，_submitted_orders 兜底)
    │   └─ 均未找到 → _send_order_status_report_from_order_update
    │       (生成 OrderStatusReport 供协调机制处理)
    │
    └─ 根据 ORDER_STATUS_MAP[order.order_status] 生成事件:
        ├─ ACCEPTED  → generate_order_accepted (仅当前状态为 SUBMITTED)
        ├─ PARTIALLY_FILLED / FILLED → 异步生成 OrderStatusReport
        ├─ CANCELED  → generate_order_canceled
        └─ REJECTED  → generate_order_rejected (排除已终态订单)
```

#### `_on_trade(trade)` — 成交回报

```
XtTrade → _on_trade
    │
    ├─ 从 order_id 或 order_remark 查找 client_order_id
    │
    ├─ 未找到 cached_order → _try_parse_xt_trade_to_fill_report → FillReport
    │
    └─ 找到 cached_order:
        └─ generate_order_filled(
               last_qty=trade.traded_volume,
               last_px=trade.traded_price,
               trade_id=trade.traded_id,
               commission=trade.commission (CNY),
           )
```

> **外部订单处理**：当收到无法匹配 `ClientOrderId` 的回报时（如通过其他终端下的单），适配器不会抛出错误，而是生成 `OrderStatusReport` 或 `FillReport`，交由 NautilusTrader 协调机制处理。

#### `_on_order_rejected` / `_on_cancel_rejected`

二者的逻辑相似：从 `_order_id_to_client_order_id` 查找映射；若映射不存在，则遍历 `query_orders()` 结果通过 `order_remark` 匹配。找到后检查订单是否已处于终态，避免重复生成事件。

### 5.5 持仓协调 (Reconciliation)

启动时，`ExecEngine` 调用 `generate_mass_status()` 进行全量持仓协调：

```python
async def generate_mass_status(self, lookback_mins=None):
    mass_status = ExecutionMassStatus(...)

    # 1. 委托状态报告
    order_reports = await self.generate_order_status_reports(...)
    mass_status.add_order_reports(order_reports)

    # 2. 成交记录报告
    fill_reports = await self.generate_fill_reports(...)
    mass_status.add_fill_reports(fill_reports)

    # 3. 持仓快照报告
    pos_reports = await self.generate_position_status_reports(...)
    mass_status.add_position_reports(pos_reports)

    return mass_status
```

**`generate_position_status_reports`** 内部使用 `AccountMixin.query_positions()` 获取 `TTPosition` 列表，并转换为 `PositionStatusReport`。其中 `available_volume` 反映 A 股 T+1 规则下的可卖数量。

**`_try_parse_xt_order_to_order_status_report`** 的价格处理逻辑：

```python
# 如果限价单价格为 0（如最优五档即时），尝试使用成交均价
if price_val <= 0.0 and order_type == OrderType.LIMIT:
    price_val = float(getattr(xt_order, "traded_price", 0.0) or 0.0)

# 如果仍为 0，强制转为市价单，避免 OrderUnpacker 报错
if order_type == OrderType.LIMIT and price_obj is None:
    order_type = OrderType.MARKET
```

### 5.6 资金与持仓推送

`ExecutionClient` 注册了 `asset_update` 和 `position_update` 两个推送事件处理器：

**`_on_asset_update(asset)`**：接收 xtquant 资金变动推送，调用 `generate_account_state()` 更新 NautilusTrader 的账户状态（`total_asset`, `cash`, `frozen_cash`, `market_value`）。

**`_on_position_update(position)`**：接收 xtquant 持仓变动推送。由于推送的持仓可能涉及尚未加载证券信息的标的，处理流程为：

1. 尝试从 cache 获取 instrument
2. 若不存在，异步调用 `instrument_provider.load_ids_async()` 加载
3. 加载完成后通过 `_send_position_report_after_load()` 生成 `PositionStatusReport`

---

## 6. 数据解析层 (Parsing)

### 6.1 行情数据解析

`parsing/data.py` 包含纯函数，负责 XtQuant 原始数据 → NautilusTrader 数据模型的转换：

| 函数 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `parse_tick_to_quote_tick()` | XtQuant tick dict | `QuoteTick` | 提取 `bidPrice[0]`, `askPrice[0]` 及量 |
| `parse_tick_to_trade_tick()` | XtQuant tick dict | `TradeTick` | 提取 `lastPrice`, `volume` |
| `parse_kline_to_bar()` | XtQuant kline dict | `Bar` | 提取 OHLCV |
| `parse_l2_quote_to_order_book_deltas()` | XtQuant l2quote dict | `list[OrderBookDelta]` | 多档买卖盘快照 |
| `parse_l2_order_to_delta()` | XtQuant l2order dict | `OrderBookDelta` | 逐笔委托 |
| `parse_l2_transaction_to_trade_tick()` | XtQuant l2transaction dict | `TradeTick` | 逐笔成交 (含攻击方) |

**时间戳转换**：

```python
def xt_time_to_ns(time_val: int) -> int:
    """XtQuant 两种时间格式：
    - 毫秒时间戳 (位数 ≤ 13): × 1_000_000 → 纳秒
    - YYYYMMDDHHMMSS (位数 > 13): 解析为 datetime → 纳秒
    """

def ns_to_xt_time(ts_ns: int) -> str:
    """纳秒时间戳 → XtQuant 格式 (YYYYMMDDHHMMSS)"""
```

**K 线周期映射**：

| Nautilus BarAggregation | XtQuant period |
|------------------------|----------------|
| `TICK` | `"tick"` |
| `MINUTE` (step=1) | `"1m"` |
| `MINUTE` (step=5) | `"5m"` |
| `MINUTE` (step=15) | `"15m"` |
| `MINUTE` (step=30) | `"30m"` |
| `HOUR` (step=1) | `"1h"` |
| `DAY` | `"1d"` |
| `WEEK` | `"1w"` |
| `MONTH` | `"1mon"` |

**Level2 数据类型**：

| XtQuant period | 说明 |
|---------------|------|
| `l2quote` | Level2 实时行情快照 |
| `l2order` | Level2 逐笔委托 |
| `l2transaction` | Level2 逐笔成交 |
| `l2quoteaux` | Level2 行情补充 |
| `l2orderqueue` | Level2 委托队列 |

### 6.2 执行回报解析与状态映射

`parsing/execution.py` 定义了 XtQuant 与 NautilusTrader 之间的枚举映射表。

**订单状态映射**：

| XtQuant 常量 | 值 | NautilusTrader OrderStatus | 含义 |
|-------------|-----|---------------------------|------|
| `ORDER_UNREPORTED` | 48 | `SUBMITTED` | 未报 |
| `ORDER_WAIT_REPORTING` | 49 | `SUBMITTED` | 待报 |
| `ORDER_REPORTED` | 50 | `ACCEPTED` | 已报 |
| `ORDER_REPORTED_CANCEL` | 51 | `PENDING_CANCEL` | 已报待撤 |
| `ORDER_PARTSUCC_CANCEL` | 52 | `PARTIALLY_FILLED` | 部成待撤 |
| `ORDER_PART_CANCEL` | 53 | `CANCELED` | 部撤 |
| `ORDER_CANCELED` | 54 | `CANCELED` | 已撤 |
| `ORDER_PART_SUCC` | 55 | `PARTIALLY_FILLED` | 部成 |
| `ORDER_SUCCEEDED` | 56 | `FILLED` | 已成 |
| `ORDER_JUNK` | 57 | `REJECTED` | 废单 |
| `ORDER_UNKNOWN` | 255 | `DENIED` | 未知 |

**多品种委托类型映射**：

| 品种 | 映射表 | 典型值 |
|------|--------|--------|
| 股票 | `STOCK_ORDER_TYPE_MAP` | `STOCK_BUY` / `STOCK_SELL` |
| 信用 | `CREDIT_ORDER_TYPE_MAP` | `CREDIT_FIN_BUY` / `CREDIT_SLO_SELL` / `CREDIT_BUY_SECU_REPAY` 等 |
| 期货 (六键) | `FUTURES_SIX_KEY_ORDER_TYPE_MAP` | `FUTURE_OPEN_LONG` / `FUTURE_CLOSE_LONG_TODAY` 等 |
| 期货 (四键) | `FUTURES_FOUR_KEY_ORDER_TYPE_MAP` | `FUTURE_OPEN_LONG` / `FUTURE_CLOSE_LONG_HISTORY_FIRST` 等 |

**价格类型映射**（`PRICE_TYPE_MAP`）：

| XtQuant 常量 | NautilusTrader OrderType |
|-------------|------------------------|
| `FIX_PRICE` | `LIMIT` |
| `LATEST_PRICE` | `MARKET` |
| `MARKET_PEER_PRICE_FIRST` | `MARKET` |

**方向与开平标志**：

| 映射表 | 用途 |
|--------|------|
| `DIRECTION_MAP` | 多 (`DIRECTION_FLAG_LONG`) / 空 (`DIRECTION_FLAG_SHORT`) |
| `OFFSET_FLAG_MAP` | 开仓 / 平仓 / 强平 / 平今 / 平昨 / 强减 / 本地强平 |
| `NAUTILUS_SIDE_TO_XT` | `OrderSide.BUY` → `STOCK_BUY` / `OrderSide.SELL` → `STOCK_SELL` |

> **注意**：当前 `execution.py` 中 `_submit_order()` **仅使用 `NAUTILUS_SIDE_TO_XT`（股票映射）**。扩展期货/信用交易需根据 `account_type` 选择正确的映射表。参见 [11.2 添加新的订单类型支持](#112-添加新的订单类型支持)。

### 6.3 证券信息解析与代码转换

`parsing/instruments.py` 包含证券解析与代码转换函数：

| 函数 | 作用 |
|------|------|
| `stock_code_to_instrument_id()` | `"600000.SH"` → `InstrumentId("600000.SSE")` |
| `instrument_id_to_stock_code()` | `InstrumentId("600000.SSE")` → `"600000.SH"` |
| `parse_equity()` | XtQuant detail → `Equity` (lot_size=100, CNY) |
| `parse_future()` | XtQuant detail → `FuturesContract` (含合约乘数、到期日) |
| `parse_option()` | XtQuant detail → `OptionContract` (含行权价、期权类型、标的) |
| `_get_precision()` | 从 `PriceTick` 计算价格精度 (小数位数) |

**市场代码映射** (定义在 `parsing/instruments.py` 与 `common.py`)：

```python
MARKET_TO_VENUE = {"SH": "SSE", "SZ": "SZSE", "BJ": "BSE", "SF": "SHFE", ...}
VENUE_TO_MARKET = {"SSE": "SH", "SZSE": "SZ", "BSE": "BJ", "SHFE": "SF", ...}
```

**`instrument_id_to_stock_code` 的三级回退策略**：

1. 若提供 `cache`，从缓存 instrument 的 exchange 属性获取 Venue
2. 使用 `instrument_id.venue` 查表 `VENUE_TO_MARKET`
3. 最后根据 symbol 首字符推断市场（`6*` → SH, `0*/3*` → SZ, `8*/4*` → BJ, 短代码 → IF）

---

## 7. 配置与工厂

### 7.1 配置数据类

所有配置类继承自 NautilusTrader 的基础 Config 类，使用 `frozen=True` 确保不可变：

```python
class ThinkTraderInstrumentProviderConfig(InstrumentProviderConfig, kw_only=True, frozen=True):
    load_contracts_on_start: bool = True
    cache_instruments: bool = True
    sectors: tuple[str, ...] = ("沪深A股",)    # 要加载的板块列表
    filter_expiry: bool = False                # 是否过滤已过期合约
    # load_all / load_ids 继承自 InstrumentProviderConfig

class ThinkTraderDataClientConfig(LiveDataClientConfig, kw_only=True, frozen=True):
    miniqmt_path: str                          # [必填] MiniQmt userdata 路径
    session_id: int = 123456                   # 会话 ID
    instrument_provider: InstrumentProviderConfig = ThinkTraderInstrumentProviderConfig()
    subscribe_whole_quote: bool = False        # 是否使用全推行情
    subscription_delay_secs: float = 0.1       # 订阅间隔 (避免过快)
    skip_trader_login: bool = False            # 是否跳过交易端登录 (仅使用行情)

class ThinkTraderExecClientConfig(LiveExecClientConfig, kw_only=True, frozen=True):
    miniqmt_path: str                          # [必填] MiniQmt userdata 路径
    account_id: str                            # [必填] 资金账号
    account_type: str = "STOCK"                # 账号类型: STOCK, CREDIT, FUTURE, OPTION
    session_id: int = 123456                   # 会话 ID
    use_async_order: bool = True               # 是否使用异步下单
    use_async_cancel: bool = True              # 是否使用异步撤单
    relaxed_response_order: bool = True        # 开启宽松时序模式
    instrument_provider: InstrumentProviderConfig = ThinkTraderInstrumentProviderConfig()
```

### 7.2 工厂与单例缓存

`factories.py` 实现了 NautilusTrader 的工厂接口，并加入**模块级缓存**实现单例：

```python
# 模块级缓存变量
THINKTRADER_CLIENTS: dict[tuple, ThinkTraderClient] = {}
THINKTRADER_INSTRUMENT_PROVIDERS: dict[tuple, ThinkTraderInstrumentProvider] = {}
```

**`get_cached_thinktrader_client`**：

- 缓存 key 为 `(miniqmt_path, session_id)`
- 若已存在但 `account_id` 为空（DataClient 先创建）而本次传入了 `account_id`（ExecClient 创建），则更新现有实例的 `_account_id` 和 `_account_type`

```python
def get_cached_thinktrader_client(loop, miniqmt_path, session_id, account_id, account_type="STOCK"):
    client_key = (miniqmt_path, session_id)
    if client_key not in THINKTRADER_CLIENTS:
        THINKTRADER_CLIENTS[client_key] = ThinkTraderClient(...)
    else:
        client = THINKTRADER_CLIENTS[client_key]
        if not client._account_id and account_id:
            client._account_id = account_id
            client._account_type = account_type
    return THINKTRADER_CLIENTS[client_key]
```

**`get_cached_thinktrader_instrument_provider`**：

- 缓存 key 为 `((miniqmt_path, session_id, account_id), hash(config))`
- 相同客户端 + 相同配置复用同一 provider 实例

```
DataClientFactory.create()  ──┐
                              ├──► THINKTRADER_CLIENTS[(path, session_id)]
ExecClientFactory.create()  ──┘          │
                                         ▼
                              ThinkTraderClient (单例)
                              ├── ThinkTraderDataClient (持有引用)
                              └── ThinkTraderExecutionClient (持有引用)
```

> **重要**：DataClient 和 ExecClient **必须使用相同的 `session_id`**，否则会创建两个独立的底层连接，导致行情和交易不共享同一会话。

---

## 8. 证券信息提供者

`ThinkTraderInstrumentProvider` (`providers.py`) 继承自 `InstrumentProvider`，负责加载和解析证券元数据。

**加载流程**：

```python
async def initialize(self, reload=False):
    # 优先级：load_all_on_start / load_ids_on_start (父类配置)
    # → 若均未设置，检查 load_contracts_on_start
    # → 调用 load_all_async()

async def load_all_async(self, filters=None):
    for sector in self._tt_config.sectors:     # 默认 ("沪深A股",)
        await self._load_sector(sector)

async def _load_sector(self, sector):
    stock_codes = self._client.get_stock_list(sector)
    for stock_code in stock_codes:
        detail = self._client.get_instrument_detail(stock_code)
        instrument = self._parse_instrument(stock_code, detail)
        if instrument:
            self.add(instrument)
        await asyncio.sleep(0.01)    # 限速，避免请求过快

async def load_ids_async(self, instrument_ids, filters=None):
    for instrument_id in instrument_ids:
        if self.find(instrument_id) is not None:   # 跳过已加载
            continue
        stock_code = instrument_id_to_stock_code(instrument_id)
        detail = self._client.get_instrument_detail(stock_code)
        instrument = self._parse_instrument(stock_code, detail)
        if instrument:
            self.add(instrument)
```

**证券类型判断与解析**：

```python
def _parse_instrument(self, stock_code, detail):
    instrument_id = stock_code_to_instrument_id(stock_code)
    instrument_type = self._client.get_instrument_type(stock_code)
    # instrument_type 是 dict，如 {"stock": True} 或 {"future": True}

    if instrument_type.get("stock") or instrument_type.get("fund"):
        return parse_equity(detail, instrument_id)
    elif instrument_type.get("future"):
        return parse_future(detail, instrument_id)
    elif instrument_type.get("option"):
        return parse_option(detail, instrument_id)
    else:
        return parse_equity(detail, instrument_id)     # 默认回退
```

---

## 9. 历史数据客户端

`HistoricThinkTraderClient` (`historical/client.py`) 是**独立的**数据下载客户端，不依赖 TradingNode：

```python
client = HistoricThinkTraderClient(
    miniqmt_path="...",
    session_id=999999,
)

bars = await client.request_bars(
    bar_specifications=["1-MINUTE", "5-MINUTE"],
    instrument_ids=["601808.SSE"],
    end_date_time=datetime(2026, 2, 6, 15, 0, 0),
    duration="60 D",
)

ticks = await client.request_ticks(
    tick_type="BID_ASK",        # 或 "TRADES"
    instrument_ids=["601808.SSE"],
    start_date_time=datetime(2026, 2, 5, 9, 30),
    end_date_time=datetime(2026, 2, 5, 15, 0),
    tz_name="Asia/Shanghai",
)

instruments = await client.request_instruments(
    instrument_ids=["601808.SSE"],
)
```

**内部实现**：

- 创建独立的 `ThinkTraderClient` 实例（不复用工厂缓存的单例）
- 初始化自己的 `InstrumentProvider` 用于 instrument 解析
- `_duration_to_timedelta()` 支持 `"60 D"` / `"4 W"` / `"3 M"` / `"1 Y"` 格式
- `_to_shanghai_ts()` 将时间统一转换为 `Asia/Shanghai` 时区
- `_convert_instrument_ids()` 接受 `str` 或 `InstrumentId` 混合列表

---

## 10. 线程安全模型

适配器涉及两个线程：

| 线程 | 角色 | 运行的组件 |
|------|------|-----------|
| **asyncio 主线程** | NautilusTrader 事件循环 | DataClient, ExecClient, Strategy |
| **xtquant 回调线程** | SDK 内部线程 | `XtQuantTraderCallback` 的所有 `on_*` 方法 |

**关键原则**：

1. xtquant 回调中**只做最小工作** — 立即通过 `call_soon_threadsafe` 转交主线程
2. 所有业务逻辑（ID 映射、状态更新、事件生成）**仅在主线程执行**
3. `asyncio.Event` (`_is_connected`) 是线程安全的，可在两个线程间安全使用
4. `ErrorMixin` 的 `_handle_order_error()` 和 `_handle_cancel_error()` 在回调线程中触发，但通过 `call_soon_threadsafe` 将事件处理器调度到主线程

```python
# 正确：xtquant 回调线程中，仅做跨线程调度
def _on_order(self, order: Any) -> None:
    self._loop.call_soon_threadsafe(
        self._handle_order_update,
        order,
    )
```

> **警告**：不要在 `ThinkTraderClientCallback` 的 `on_*` 方法中直接访问 NautilusTrader 的 Cache 或 MessageBus — 它们不是线程安全的。

---

## 11. 扩展指南

### 11.1 添加新的市场类型支持

1. **更新 `common.py`** — 添加 `VENUE_TO_MARKET` / `MARKET_TO_VENUE` 映射
2. **更新 `parsing/instruments.py`** — 同步更新 `MARKET_TO_VENUE` / `VENUE_TO_MARKET` 映射，确保 `stock_code_to_instrument_id()` 和 `instrument_id_to_stock_code()` 正确转换
3. **更新 `providers.py`** — 在 `_parse_instrument()` 中添加新类型的解析分支（如需要）

### 11.2 添加新的订单类型支持

**当前限制**：`execution.py` 中 `_submit_order()` 仅支持股票买卖。

**扩展期货支持示例**：

```python
async def _submit_order(self, command):
    ...
    if self._config.account_type == "STOCK":
        order_type = NAUTILUS_SIDE_TO_XT.get(order.side)
    elif self._config.account_type == "FUTURE":
        order_type = self._get_futures_order_type(order)
    elif self._config.account_type == "CREDIT":
        order_type = self._get_credit_order_type(order)
    ...
```

**需要处理的复杂性**：

| 品种 | 复杂度来源 |
|------|----------|
| 期货 | 开仓/平仓方向 (`OFFSET_FLAG`)，平今/平昨区分 |
| 信用 | 融资/融券/担保品区分，六种委托类型 |
| 期权 | 行权方向，标的关联 |

### 11.3 添加新的数据类型支持

1. **在 `parsing/data.py` 中**添加解析纯函数

2. **在 `client/market_data.py` 中**添加订阅方法：

```python
async def subscribe_new_data_type(self, instrument_id, stock_code):
    name = (str(instrument_id), "new_type")
    await self._subscribe(
        name,
        xtdata.subscribe_quote,
        xtdata.unsubscribe_quote,
        stock_code=stock_code,
        period="new_period",
        count=0,
        callback=functools.partial(self._on_quote_data, name=name),
    )
```

3. **在 `_handle_tuple_quote_data` 中**添加分支：

```python
if data_type == "new_type":
    parsed = parse_new_type_to_model(instrument_id, data, ts_init)
    self._forward_data(parsed)
```

4. **在 `data.py` 中**暴露 NautilusTrader 接口（`_subscribe_*` / `_unsubscribe_*`）

### 11.4 添加新的 Mixin 功能

1. 创建 `client/new_feature.py`
2. 继承 `BaseMixin`
3. 在 `ThinkTraderClient` 继承列表中添加
4. 确保仅访问 `BaseMixin` 声明的属性

```python
# client/new_feature.py
from nautilus_trader.adapters.thinktrader.client.common import BaseMixin

class ThinkTraderClientNewFeatureMixin(BaseMixin):

    def new_method(self) -> None:
        self._log.info("新功能")
        result = self._trader.some_new_api(...)
        ...
```

```python
# client/client.py
class ThinkTraderClient(
    ThinkTraderClientConnectionMixin,
    ThinkTraderClientMarketDataMixin,
    ThinkTraderClientAccountMixin,
    ThinkTraderClientOrderMixin,
    ThinkTraderClientContractMixin,
    ThinkTraderClientErrorMixin,
    ThinkTraderClientNewFeatureMixin,  # ← 新增
):
    ...
```

---

## 12. 设计决策与取舍

| 设计决策 | 选择 | 原因 |
|---------|------|------|
| **Mixin 多继承** vs 组合 | 多继承 | 各 Mixin 方法可如自身方法般调用，代码简洁；通过 `BaseMixin` 约束共享状态 |
| **模块级缓存** vs 依赖注入 | 模块级缓存 | 符合 NautilusTrader 工厂模式；全局唯一连接更安全 |
| **`order_remark` 携带 ClientOrderId** | 下单时写入 | 利用 xtquant 的 `order_remark` 字段无损传递 ClientOrderId，回调中可靠恢复映射 |
| **异步下单为默认** | `use_async_order=True` | 避免阻塞事件循环；缺点是 ID 映射有延迟 (通过 `_submitted_orders` 缓解) |
| **宽松时序** | `relaxed_response_order=True` | 券商柜台回报可能乱序 (先成交后确认)；启用此模式允许 xtquant 内部重排 |
| **`_forward_data` 事件处理器** | 注册模式 | 解耦底层客户端与 NautilusTrader 消息总线 |
| **纯函数解析层** | `parsing/` 全部为纯函数 | 无状态、无副作用，易于测试和复用 |
| **多级撤单回退** | 四级回退策略 | 异步场景下 ID 映射可能不完整；sysid 撤单是最后的兜底手段 |
| **限价单无价格时转市价单** | `_try_parse_xt_order_to_order_status_report` 中 | 某些柜台返回限价单价格为 0（如最优五档即时），直接标记为 LIMIT 会导致 `OrderUnpacker` 报错 |
| **OmsType.NETTING** | 净额持仓 | A 股不支持同一标的双向持仓 (HEDGING)；所有持仓按净额计算 |
| **`_modify_order` 拒绝** | 直接拒绝 | XtQuant 不支持修改订单，只能撤单后重下 |
| **sysid_async 回退** | 回退至同步 | 部分 xtquant 版本不支持 `cancel_order_stock_sysid_async`，自动降级到同步调用 |

---

## 13. 附录：核心类型关系图

```
                      ┌──────────────┐
                      │  BaseMixin   │ (ABC)
                      └──────┬───────┘
          ┌──────────────────┼──────────────────┐
          │                  │                  │
  ┌───────▼──────┐  ┌───────▼──────┐  ┌────────▼──────┐
  │ Connection   │  │ MarketData   │  │  Account      │
  │ Mixin        │  │ Mixin        │  │  Mixin        │
  │ ──────────── │  │ ──────────── │  │ ──────────────│
  │ _connect()   │  │ subscribe_*()│  │ query_asset() │
  │ _disconnect()│  │ get_hist_*() │  │ query_pos()   │
  │ subscribe()  │  │ download_*() │  │ query_orders()│
  └──────────────┘  └──────────────┘  └───────────────┘
          │                  │                  │
  ┌───────▼──────┐  ┌───────▼──────┐  ┌────────▼──────┐
  │  Order       │  │  Contract    │  │  Error        │
  │  Mixin       │  │  Mixin       │  │  Mixin        │
  │ ──────────── │  │ ──────────── │  │ ──────────────│
  │ place_order()│  │ get_detail() │  │ _handle_*()   │
  │ cancel_*()   │  │ get_type()   │  │ CONNECTION_   │
  │ set_relaxed()│  │ get_list()   │  │   ERRORS      │
  │              │  │ get_dates()  │  │               │
  └──────────────┘  └──────────────┘  └───────────────┘
          │                  │                  │
          └──────────────────┼──────────────────┘
                      ┌──────▼───────┐
                      │ ThinkTrader  │
                      │ Client       │ (聚合类)
                      │ ──────────── │
                      │ __init__()   │
                      │ _next_req_id │
                      │ register_*() │
                      │ _on_*()      │
                      │ _handle_*()  │
                      └──────┬───────┘
                             │
                      ┌──────▼───────┐
                      │ Callback     │
                      │ (XtQuant     │
                      │  Callback)   │
                      └──────────────┘

                             ▲ uses
                ┌────────────┼────────────┐
          ┌─────▼──────┐           ┌──────▼──────┐
          │ ThinkTrader│           │ ThinkTrader │
          │ DataClient │           │ Execution   │
          │ (data.py)  │           │ Client      │
          │ ────────── │           │ (exec.py)   │
          │ subscribe* │           │ ─────────── │
          │ request*   │           │ _submit_*() │
          │ chunked*   │           │ _cancel_*() │
          │            │           │ _on_*()     │
          └────────────┘           │ generate_*()│
                                   └─────────────┘
```

**文件总览**：

```
nautilus_trader/adapters/thinktrader/
├── common.py                # 常量: TT, TT_VENUE, 市场映射, 账号类型映射
├── config.py                # 3 个配置类 (frozen, kw_only)
├── data.py                  # ThinkTraderDataClient (LiveMarketDataClient)
├── execution.py             # ThinkTraderExecutionClient (LiveExecutionClient)
├── factories.py             # 工厂 + 模块级单例缓存
├── providers.py             # ThinkTraderInstrumentProvider
├── client/
│   ├── __init__.py          # 导出 ThinkTraderClient
│   ├── client.py            # ThinkTraderClient (聚合) + ThinkTraderClientCallback
│   ├── common.py            # BaseMixin, Request, Subscription, Requests, Subscriptions,
│   │                        # TTPosition, TTOrder, TTTrade
│   ├── connection.py        # ConnectionMixin: _connect/_disconnect/subscribe/unsubscribe
│   ├── market_data.py       # MarketDataMixin: subscribe_ticks/market_data/order_book/bars,
│   │                        # get_historical_bars/ticks, download_history_data, subscribe_whole_quote
│   ├── account.py           # AccountMixin: query_asset/positions/orders/trades (+async variants)
│   ├── order.py             # OrderMixin: place_order/cancel_order (+async, +by_sysid variants),
│   │                        # set_relaxed_response_order_enabled
│   ├── contract.py          # ContractMixin: get_instrument_detail/type, get_stock_list,
│   │                        # get_trading_dates
│   └── error.py             # ErrorMixin: _handle_order_error/_handle_cancel_error/
│                            # _handle_connection_error, 错误码分类常量
├── historical/
│   └── client.py            # HistoricThinkTraderClient: request_bars/ticks/instruments
└── parsing/
    ├── data.py              # parse_tick_to_quote_tick/trade_tick, parse_kline_to_bar,
    │                        # parse_l2_quote/order/transaction, xt_time_to_ns, ns_to_xt_time,
    │                        # bar_spec_to_period, PERIOD_MAP, LEVEL2_PERIOD_MAP, STEP_TO_PERIOD
    ├── execution.py         # ORDER_STATUS_MAP, STOCK/CREDIT/FUTURES_*_ORDER_TYPE_MAP,
    │                        # DIRECTION_MAP, OFFSET_FLAG_MAP, PRICE_TYPE_MAP, NAUTILUS_SIDE_TO_XT
    └── instruments.py       # parse_equity/future/option, stock_code_to_instrument_id,
                             # instrument_id_to_stock_code, _get_precision, MARKET_TO_VENUE
```
