# ThinkTrader Adapter 用户使用手册

## 目录

1. [概述](#1-概述)
2. [准备工作](#2-准备工作)
3. [快速开始](#3-快速开始)
4. [核心概念](#4-核心概念)
5. [使用场景](#5-使用场景)
   - 5.1 [订阅实时行情](#51-订阅实时行情)
   - 5.2 [下载历史数据](#52-下载历史数据)
   - 5.3 [查询账户资金与持仓](#53-查询账户资金与持仓)
   - 5.4 [提交与撤销订单](#54-提交与撤销订单)
   - 5.5 [多标的量化策略](#55-多标的量化策略)
6. [策略开发指南](#6-策略开发指南)
7. [常见问题与故障排查](#7-常见问题与故障排查)
8. [注意事项与最佳实践](#8-注意事项与最佳实践)
9. [配置项参考](#9-配置项参考)
10. [支持的市场与品种](#10-支持的市场与品种)
11. [附录：架构与目录结构](#11-附录架构与目录结构)

---

## 1. 概述

**ThinkTrader Adapter** 是 NautilusTrader 框架的实盘适配器，通过 MiniQmt 客户端及 `xtquant` SDK 接入中国券商交易系统。主要功能包括：

- **实时行情** — 订阅并接收 Quote Tick、Trade Tick、K 线 (Bar) 等市场数据
- **历史数据** — 批量下载历史 K 线和 Tick 数据，支持脱离 TradingNode 独立使用
- **账户管理** — 查询账户资金、持仓信息，接收资产与持仓变动推送
- **交易执行** — 提交限价单 / 市价单、撤单、查询委托状态

> **NautilusTrader** 是一个高性能事件驱动量化交易框架，提供策略回测与实盘交易功能。ThinkTrader Adapter 作为其实盘适配层，负责与 MiniQmt (xtquant) 交易终端的通信。

---

## 2. 准备工作

### 2.1 环境要求

| 项目 | 要求 | 验证方式 |
|------|------|----------|
| **Python** | ≥ 3.10 | `python --version` |
| **NautilusTrader** | 已安装 | `python -c "import nautilus_trader"` |
| **MiniQmt 终端** | 已安装并能正常登录 | 从券商处获取安装包 |
| **xtquant** | 已安装（MiniQmt 自带） | `python -c "from xtquant import xtdata"` |
| **userdata_mini 路径** | 位于 MiniQmt 安装目录下 | 确认该文件夹存在且非空 |
| **资金账号** | 从 MiniQmt 终端获取 | 一串数字，如 `211800003313` |

> `userdata_mini` 路径示例：`D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini`

### 2.2 配置环境变量

在项目根目录下创建 `.env` 文件：

```env
# MiniQmt 的 userdata_mini 目录绝对路径（必填）
MINIQMT_PATH=D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini

# 资金账号（交易及账户查询场景必填；仅行情 / 历史数据场景可省略）
MINIQMT_ACCOUNT_ID=211800003313

# 账号类型（可选，默认 STOCK）
# MINIQMT_ACCOUNT_TYPE=STOCK
```

| 变量名 | 必填 | 说明 |
|--------|------|------|
| `MINIQMT_PATH` | 是 | MiniQmt `userdata_mini` 目录绝对路径 |
| `MINIQMT_ACCOUNT_ID` | 交易场景必填 | 资金账号 |
| `MINIQMT_ACCOUNT_TYPE` | 否 | 账号类型：`STOCK`(默认) / `CREDIT` / `FUTURE` / `STOCK_OPTION` 等 |

---

## 3. 快速开始

以下步骤演示如何连接 MiniQmt 并订阅实时行情。

### Step 1：启动 MiniQmt 终端

打开 MiniQmt 终端并成功登录。终端需保持运行（可最小化）。

### Step 2：确认 `.env` 已正确配置

参考 [第 2 节](#22-配置环境变量)，确保 `MINIQMT_PATH` 和 `MINIQMT_ACCOUNT_ID` 已填写。

### Step 3：运行示例脚本

```bash
python examples/live/thinktrader/connect_with_miniqmt.py
```

### Step 4：验证输出

正常情况下，终端将持续输出 `QuoteTick` 数据：

```
[INFO] THINKTRADER.ThinkTraderClient: 连接到 MiniQmt 成功
[INFO] THINKTRADER.InstrumentProvider: 加载了 1 个证券信息
[INFO] TRADER-001.SubscribeStrategy: QuoteTick(instrument_id=000547.SZSE, bid=XX.XX, ask=XX.XX, ...)
```

按 **Ctrl+C** 停止程序。

### 常见启动错误

| 错误信息 | 原因 | 解决方法 |
|---------|------|---------|
| `ModuleNotFoundError: xtquant` | xtquant 未正确安装 | 参考 [准备工作](#21-环境要求) |
| 连接超时 / `timeout` | MiniQmt 未运行，或路径错误 | 确认终端已登录；检查 `MINIQMT_PATH` |
| 账户对象未找到 | `MINIQMT_ACCOUNT_ID` 不正确 | 核对资金账号 |
| 无 QuoteTick 输出 | 非交易时段（A 股 9:30-15:00） | 在交易时段运行 |

---

## 4. 核心概念

### 4.1 InstrumentId（标的代码）

NautilusTrader 使用 `InstrumentId` 标识交易标的，格式为 `<证券代码>.<Venue>`，与 MiniQmt 的 `<代码>.<市场>` 格式不同。适配器在内部自动完成双向转换。

| NautilusTrader | MiniQmt | 说明 |
|----------------|---------|------|
| `600000.SSE` | `600000.SH` | 上交所 |
| `000001.SZSE` | `000001.SZ` | 深交所 |
| `430047.BSE` | `430047.BJ` | 北交所 |
| `IF2410.CFFEX` | `IF2410.IF` | 中金所（期货） |

```python
from nautilus_trader.model.identifiers import InstrumentId

instrument_id = InstrumentId.from_str("600000.SSE")   # 浦发银行
instrument_id = InstrumentId.from_str("000001.SZSE")  # 平安银行
```

> 完整交易所映射表见 [第 10 节](#10-支持的市场与品种)。

### 4.2 TradingNode（交易节点）

`TradingNode` 是 NautilusTrader 实盘运行的核心入口，负责管理连接、数据、交易和策略的生命周期。

```python
from nautilus_trader.live.node import TradingNode

node = TradingNode(config=config_node)   # 创建节点
node.build()                              # 构建组件
node.run()                                # 启动事件循环
```

### 4.3 Strategy（策略）

策略继承自 `Strategy` 基类，通过覆盖回调方法实现交易逻辑。NautilusTrader 在相应事件发生时自动调用对应方法：

| 回调方法 | 触发时机 |
|---------|---------|
| `on_start()` | 策略启动 — 订阅数据、初始化状态 |
| `on_quote_tick(tick)` | 收到 QuoteTick — 分析数据、决策交易 |
| `on_order_filled(event)` | 订单成交 — 更新状态、触发后续操作 |
| `on_stop()` | 策略停止 — 清理资源 |

### 4.4 session_id（会话标识）

连接 MiniQmt 时需提供一个整数 `session_id`。**同一脚本中，`data_clients` 和 `exec_clients` 必须使用相同的 `session_id`**，以确保共享同一个底层 `ThinkTraderClient` 实例。

示例脚本通过 `random.randint(100000, 999999)` 随机生成，避免与其他进程冲突。

---

## 5. 使用场景

### 5.1 订阅实时行情

**示例文件**：`examples/live/thinktrader/connect_with_miniqmt.py`

```bash
python examples/live/thinktrader/connect_with_miniqmt.py
```

修改订阅标的：

```python
instrument_id = InstrumentId.from_str("601808.SSE")  # 中国海油
```

订阅多只标的：

```python
ids = [
    InstrumentId.from_str("601808.SSE"),
    InstrumentId.from_str("000001.SZSE"),
]
instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset(ids),
)

# 策略中逐个订阅
def on_start(self):
    for iid in self.instrument_ids:
        self.subscribe_quote_ticks(iid)
```

---

### 5.2 下载历史数据

`HistoricThinkTraderClient` 可独立于 TradingNode 使用，无需配置策略和执行客户端。

**示例文件**：`examples/live/thinktrader/historical_download.py`

**依赖**：`pip install pandas python-dotenv`

```bash
python examples/live/thinktrader/historical_download.py
```

数据输出至 `examples/live/thinktrader/outputs/` 目录。

修改下载参数（脚本顶部）：

```python
TARGET_SYMBOLS = ["601808.SSE", "000001.SZSE"]  # 标的列表
BAR_TYPES = ["1-MINUTE", "1-DAY"]               # K 线类型
END_TIME = datetime.datetime(2026, 2, 6, 15, 0) # 结束时间
BAR_DURATION = "60 D"                            # 回溯时间跨度
```

**最小调用示例**：

```python
import asyncio
from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient

async def main():
    client = HistoricThinkTraderClient(
        miniqmt_path=r"D:\...\userdata_mini",
        session_id=999999,
    )
    bars = await client.request_bars(
        bar_specifications=["1-DAY"],
        instrument_ids=["601808.SSE"],
        duration="30 D",
    )
    print(f"下载了 {len(bars)} 根 K 线")

asyncio.run(main())
```

**支持的 K 线规格**：`"1-MINUTE"` / `"5-MINUTE"` / `"15-MINUTE"` / `"1-HOUR"` / `"1-DAY"`

**Duration 格式**：`"60 D"`（天）/ `"4 W"`（周）/ `"3 M"`（月）/ `"1 Y"`（年）

---

### 5.3 查询账户资金与持仓

**示例文件**：`examples/live/thinktrader/query_account_and_positions.py`

**前提**：`.env` 中须正确配置 `MINIQMT_ACCOUNT_ID`。

```bash
python examples/live/thinktrader/query_account_and_positions.py
```

在策略中查询账户的关键代码：

```python
from nautilus_trader.model.identifiers import AccountId

def _query_account(self):
    account_id = AccountId("THINKTRADER-211800003313")

    # 查询资金
    account = self.cache.account(account_id)
    if account:
        for currency, balance in account.balances().items():
            self.log.info(
                f"{currency}: 总计={balance.total} 可用={balance.free} 冻结={balance.locked}"
            )

    # 查询持仓
    for pos in self.cache.positions():
        self.log.info(f"{pos.instrument_id}: {pos.side} {pos.quantity} @ {pos.avg_px_open}")
```

> AccountId 格式为 `THINKTRADER-<资金账号>`，需包含适配器前缀。

---

### 5.4 提交与撤销订单

**示例文件**：`examples/live/thinktrader/buy_and_sell.py`

```bash
python examples/live/thinktrader/buy_and_sell.py
```

> **注意**：此示例会执行真实交易，请在仿真账号上运行。

创建并提交订单的核心 API：

```python
from nautilus_trader.model.enums import OrderSide, TimeInForce

instrument = self.cache.instrument(self.instrument_id)

# 限价买入 100 股
order = self.order_factory.limit(
    instrument_id=self.instrument_id,
    order_side=OrderSide.BUY,
    quantity=instrument.make_qty(100),      # 确保符合合约最小交易单位
    price=instrument.make_price(25.60),     # 确保价格精度正确
    time_in_force=TimeInForce.DAY,
)
self.submit_order(order)
```

关键约束：

| 约束 | 说明 |
|------|------|
| 数量精度 | A 股最小交易单位为 100 股（1 手），使用 `instrument.make_qty()` |
| 价格精度 | A 股保留 2 位小数，使用 `instrument.make_price()` |
| 交易时段 | A 股连续竞价：9:30-11:30、13:00-15:00 |

---

### 5.5 多标的量化策略

**示例文件**：`examples/live/thinktrader/tick_momentum_strategy.py`

该示例演示了一个多标的 Tick 动量策略，包含以下功能模块：

| 模块 | 说明 |
|------|------|
| 多标的并行监控 | 同时监控多只股票的实时行情 |
| 动量入场信号 | 滑动窗口内价格变化率超过阈值触发买入 |
| 止盈 / 止损 | 每笔持仓独立跟踪止盈和止损价位 |
| 风控规则 | 最大持仓数限制、单标的持仓上限 |

配置示例：

```python
strategy_config = MultiTickMomentumConfig(
    instrument_ids=[
        InstrumentId.from_str("688576.SSE"),
        InstrumentId.from_str("601808.SSE"),
    ],
    trade_qty=100,                   # 每笔交易数量
    entry_momentum_bps=2.0,          # 入场动量阈值（基点）
    take_profit_bps=10.0,            # 止盈阈值（基点）
    stop_loss_bps=15.0,              # 止损阈值（基点）
    max_positions_per_instrument=3,
    max_total_positions=10,
    momentum_window_seconds=15.0,
)
```

---

## 6. 策略开发指南

### 6.1 策略模板

```python
from nautilus_trader.config import StrategyConfig
from nautilus_trader.trading.strategy import Strategy
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.identifiers import InstrumentId


class MyStrategyConfig(StrategyConfig, frozen=True):
    """策略参数配置"""
    instrument_id: InstrumentId
    # trade_qty: int = 100
    # threshold: float = 0.01


class MyStrategy(Strategy):

    def __init__(self, config: MyStrategyConfig):
        super().__init__(config)
        self.instrument_id = config.instrument_id

    # ── 生命周期 ─────────────────────────────

    def on_start(self):
        """订阅数据、初始化状态"""
        self.subscribe_quote_ticks(self.instrument_id)

    def on_stop(self):
        """取消订阅、清理资源"""
        self.unsubscribe_quote_ticks(self.instrument_id)

    # ── 数据回调 ─────────────────────────────

    def on_quote_tick(self, tick: QuoteTick):
        """处理报价数据 — 策略核心逻辑"""
        bid = tick.bid_price.as_double()
        ask = tick.ask_price.as_double()
        # 在此实现交易决策逻辑

    # ── 订单回调 ─────────────────────────────

    def on_order_filled(self, event):
        """订单成交回调"""
        self.log.info(f"成交: {event}")

    def on_order_rejected(self, event):
        """订单拒绝回调"""
        self.log.error(f"订单拒绝: {event}")

    def on_order_canceled(self, event):
        """订单撤销回调"""
        self.log.warning(f"订单撤销: {event}")
```

### 6.2 常用 API

#### 数据操作

| 操作 | 方法 |
|------|------|
| 订阅 QuoteTick | `self.subscribe_quote_ticks(instrument_id)` |
| 订阅 TradeTick | `self.subscribe_trade_ticks(instrument_id)` |
| 取消订阅 | `self.unsubscribe_quote_ticks(instrument_id)` |
| 获取最新报价 | `self.cache.quote_tick(instrument_id)` |
| 获取合约信息 | `self.cache.instrument(instrument_id)` |

#### 交易操作

| 操作 | 方法 |
|------|------|
| 创建限价单 | `self.order_factory.limit(instrument_id, side, quantity, price, time_in_force)` |
| 创建市价单 | `self.order_factory.market(instrument_id, side, quantity)` |
| 提交订单 | `self.submit_order(order)` |
| 撤销订单 | `self.cancel_order(order)` |
| 撤销全部订单 | `self.cancel_all_orders(instrument_id)` |

#### 账户与持仓

| 操作 | 方法 |
|------|------|
| 查询账户 | `self.cache.account(AccountId("THINKTRADER-<账号>"))` |
| 获取余额 | `account.balances()` → `dict[Currency, AccountBalance]` |
| 获取全部持仓 | `self.cache.positions()` |
| 获取指定标的持仓 | `self.cache.positions_open(instrument_id=...)` |

#### 定时任务

```python
from datetime import timedelta

self.clock.set_time_alert(
    name="task_name",
    alert_time=self.clock.utc_now() + timedelta(seconds=5.0),
    callback=lambda _: self.do_something(),
)
```

### 6.3 接入 TradingNode

```python
import os
import random
from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import (
    ThinkTraderDataClientConfig,
    ThinkTraderExecClientConfig,
    ThinkTraderInstrumentProviderConfig,
)
from nautilus_trader.adapters.thinktrader.factories import (
    ThinkTraderLiveDataClientFactory,
    ThinkTraderLiveExecClientFactory,
)
from nautilus_trader.config import (
    LoggingConfig,
    RoutingConfig,
    TradingNodeConfig,
)
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.identifiers import InstrumentId, TraderId

# ① 基本参数
miniqmt_path = os.environ["MINIQMT_PATH"]
account_id = os.environ["MINIQMT_ACCOUNT_ID"]
session_id = random.randint(100000, 999999)
instrument_id = InstrumentId.from_str("601808.SSE")

# ② 证券信息配置
instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id]),
)

# ③ 节点配置
config_node = TradingNodeConfig(
    trader_id=TraderId("TRADER-001"),
    logging=LoggingConfig(log_level="INFO"),
    data_clients={
        TT: ThinkTraderDataClientConfig(
            miniqmt_path=miniqmt_path,
            session_id=session_id,
            instrument_provider=instrument_provider,
        ),
    },
    exec_clients={
        TT: ThinkTraderExecClientConfig(
            miniqmt_path=miniqmt_path,
            account_id=account_id,
            session_id=session_id,
            instrument_provider=instrument_provider,
            routing=RoutingConfig(default=True),
        ),
    },
    timeout_connection=90.0,
)

# ④ 创建节点、添加策略、运行
node = TradingNode(config=config_node)

strategy = MyStrategy(MyStrategyConfig(instrument_id=instrument_id))
node.trader.add_strategy(strategy)

node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
node.build()

if __name__ == "__main__":
    try:
        node.run()
    except KeyboardInterrupt:
        pass
    finally:
        node.dispose()
```

---

## 7. 常见问题与故障排查

### Q1: 连接超时

**诊断**：

1. 确认 MiniQmt 终端已启动并成功登录
2. 检查 `.env` 中的 `MINIQMT_PATH` 是否指向正确的 `userdata_mini` 目录
3. 增大连接超时：`TradingNodeConfig(timeout_connection=120.0)`

### Q2: `ModuleNotFoundError: xtquant`

**原因**：xtquant 未安装到当前 Python 环境。

**解决**：
1. 执行 `pip show xtquant` 确认安装状态
2. 若未安装，从 MiniQmt 安装目录的 `bin.x64` 下定位 xtquant 包并安装
3. 或将 xtquant 路径添加到 `PYTHONPATH` 环境变量

### Q3: 持仓查询返回空，但实际存在持仓

**原因**：未加载持仓标的的证券信息，NautilusTrader 无法识别。

**解决**：
```python
# 方式 1：加载整个板块
ThinkTraderInstrumentProviderConfig(load_contracts_on_start=True)

# 方式 2：指定加载持仓涉及的标的（推荐，启动更快）
ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([
        InstrumentId.from_str("601808.SSE"),
        InstrumentId.from_str("000001.SZSE"),
    ]),
)
```

### Q4: 订单被拒绝

**常见原因**：

| 原因 | 说明 |
|------|------|
| 资金不足 | 可用资金低于订单所需金额 |
| 数量不合规 | A 股数量须为 100 的整数倍 |
| 价格超限 | 超出涨跌停限制或小数位数不正确 |
| 非交易时段 | A 股交易时段为 9:30-11:30、13:00-15:00 |
| 账号类型不匹配 | `account_type` 与实际操作不一致 |

**建议**：使用 `instrument.make_qty()` 和 `instrument.make_price()` 构建订单参数，由系统自动处理精度。

### Q5: 回报时序异常

**原因**：券商柜台回报顺序不保证，属正常现象。

**解决**：确认配置 `relaxed_response_order=True`（默认已开启），适配器会自动处理乱序回报。

### Q6: 程序无法正常退出

**解决**：在 `finally` 块中调用 `os._exit(0)` 强制终止残留线程：

```python
import os

try:
    node.run()
except KeyboardInterrupt:
    pass
finally:
    node.dispose()
    os._exit(0)
```

### Q7: 历史数据下载结果为空

**排查方向**：

1. 确认请求的时间范围覆盖了有效交易日
2. MiniQmt 本地缓存中可能不存在该数据，需先在终端中手动请求一次
3. 检查 `MINIQMT_PATH` 是否指向正确的 `userdata_mini` 目录

---

## 8. 注意事项与最佳实践

### 关键约束

| 约束 | 说明 |
|------|------|
| MiniQmt 需保持运行 | 适配器依赖 MiniQmt 进程通信，终端关闭即断开连接 |
| session_id 一致性 | 同一脚本中 `data_clients` 和 `exec_clients` 的 `session_id` 必须相同 |
| A 股 T+1 规则 | 当日买入的股票当日不可卖出，`available_volume` 字段反映可卖数量 |
| 仿真先行 | 建议始终先在仿真账号上验证策略逻辑 |

### 推荐做法

1. **使用 `.env` 管理敏感配置**，避免在代码中硬编码路径和账号

2. **按需加载标的信息**，可显著缩短启动时间：
   ```python
   ThinkTraderInstrumentProviderConfig(
       load_all=False,
       load_ids=frozenset([InstrumentId.from_str("601808.SSE")]),
   )
   ```

3. **使用 `self.log` 记录关键事件**：
   ```python
   self.log.info(f"买入信号: {instrument_id} @ {price}")
   self.log.warning("持仓超限，跳过本次入场")
   self.log.error(f"下单失败: {reason}")
   ```

4. **确保程序正确退出**：
   ```python
   try:
       node.run()
   except KeyboardInterrupt:
       pass
   finally:
       node.dispose()
   ```

5. **适当增大连接超时**，MiniQmt 首次连接可能耗时较长：
   ```python
   TradingNodeConfig(timeout_connection=90.0)
   ```

---

## 9. 配置项参考

### 9.1 ThinkTraderInstrumentProviderConfig

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `load_contracts_on_start` | `bool` | `True` | 启动时自动加载证券信息 |
| `cache_instruments` | `bool` | `True` | 缓存已加载的证券信息 |
| `sectors` | `tuple[str, ...]` | `("沪深A股",)` | 板块列表 |
| `filter_expiry` | `bool` | `False` | 过滤已过期合约（期货 / 期权场景） |
| `load_all` | `bool` | `True` | 设为 `False` 配合 `load_ids` 按需加载 |
| `load_ids` | `frozenset` | `None` | 指定要加载的 InstrumentId 集合 |

### 9.2 ThinkTraderDataClientConfig

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `miniqmt_path` | `str` | *(必填)* | MiniQmt `userdata_mini` 目录路径 |
| `session_id` | `int` | `123456` | 会话标识，需与 ExecClient 一致 |
| `subscribe_whole_quote` | `bool` | `False` | 订阅全市场行情（数据量极大，慎用） |
| `subscription_delay_secs` | `float` | `0.1` | 连续订阅间隔（秒） |
| `skip_trader_login` | `bool` | `False` | 仅用行情功能时可设为 `True` |

### 9.3 ThinkTraderExecClientConfig

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `miniqmt_path` | `str` | *(必填)* | 同上 |
| `account_id` | `str` | *(必填)* | 资金账号 |
| `account_type` | `str` | `"STOCK"` | 账号类型 |
| `session_id` | `int` | `123456` | 须与 DataClient 相同 |
| `use_async_order` | `bool` | `True` | 异步下单（推荐） |
| `use_async_cancel` | `bool` | `True` | 异步撤单（推荐） |
| `relaxed_response_order` | `bool` | `True` | 宽松时序模式（推荐） |

### 9.4 TradingNode 超时参数

| 参数 | 推荐值 | 说明 |
|------|--------|------|
| `timeout_connection` | `90.0` | 连接超时（秒），建议 60~120 |
| `timeout_reconciliation` | `10.0` | 持仓协调超时 |
| `timeout_portfolio` | `10.0` | 组合初始化超时 |
| `timeout_disconnection` | `5.0` | 断开连接超时 |
| `timeout_post_stop` | `2.0` | 停止后等待时间 |

---

## 10. 支持的市场与品种

### 交易所映射

| NautilusTrader Venue | XtQuant 代码 | 交易所 |
|---------------------|-------------|--------|
| `SSE` | `SH` | 上海证券交易所 |
| `SZSE` | `SZ` | 深圳证券交易所 |
| `BSE` | `BJ` | 北京证券交易所 |
| `SHFE` | `SF` | 上海期货交易所 |
| `DCE` | `DF` | 大连商品交易所 |
| `CZCE` | `ZF` | 郑州商品交易所 |
| `CFFEX` | `IF` | 中国金融期货交易所 |
| `INE` | `INE` | 上海国际能源交易中心 |
| `GFEX` | `GF` | 广州期货交易所 |

### 账号类型

| 值 | 说明 | XtQuant 常量 |
|----|------|-------------|
| `STOCK` | 股票（普通） | `SECURITY_ACCOUNT` |
| `CREDIT` | 信用（融资融券） | `CREDIT_ACCOUNT` |
| `FUTURE` | 期货 | `FUTURE_ACCOUNT` |
| `STOCK_OPTION` | 股票期权 | `STOCK_OPTION_ACCOUNT` |
| `FUTURE_OPTION` | 期货期权 | `FUTURE_OPTION_ACCOUNT` |
| `HGT` | 沪港通 | `HUGANGTONG_ACCOUNT` |
| `SGT` | 深港通 | `SHENGANGTONG_ACCOUNT` |

### 支持的证券类型

| 类型 | NautilusTrader 对象 | 说明 |
|------|-------------------|------|
| 股票 / 基金 | `Equity` | 默认 lot_size = 100 |
| 期货 | `FuturesContract` | 含合约乘数、到期日 |
| 期权 | `OptionContract` | 含行权价、期权类型、标的代码 |

---

## 11. 附录：架构与目录结构

### 系统架构

```
┌───────────────────────────────────────────────────┐
│              NautilusTrader 框架                   │
│  Strategy ←→ DataEngine ←→ ExecEngine             │
└────────────┬──────────────────────────┬───────────┘
             │                          │
   ┌─────────▼─────────┐    ┌──────────▼──────────┐
   │  ThinkTrader       │    │  ThinkTrader         │
   │  DataClient        │    │  ExecutionClient     │
   └─────────┬─────────┘    └──────────┬──────────┘
             │                          │
   ┌─────────▼──────────────────────────▼──────────┐
   │          ThinkTraderClient (底层客户端)          │
   │  Connection │ MarketData │ Order │ Account     │
   │  Contract   │ Error      │                     │
   └─────────────────────┬─────────────────────────┘
                          │
               ┌──────────▼──────────┐
               │   xtquant (迅投SDK)  │
               │   MiniQmt 客户端     │
               └──────────┬──────────┘
                          │
               ┌──────────▼──────────┐
               │      券商柜台        │
               └─────────────────────┘
```

### 目录结构

```
nautilus_trader/adapters/thinktrader/
├── common.py                # 通用常量（交易所映射等）
├── config.py                # 配置类定义
├── data.py                  # 行情数据客户端 (LiveDataClient)
├── execution.py             # 执行客户端 (LiveExecutionClient)
├── factories.py             # 工厂类（创建和缓存客户端实例）
├── providers.py             # 证券信息提供者 (InstrumentProvider)
├── client/                  # 底层客户端封装
│   ├── client.py            # ThinkTraderClient（聚合所有 Mixin）
│   ├── common.py            # 基础数据结构与类型定义
│   ├── connection.py        # 连接管理 Mixin
│   ├── market_data.py       # 行情数据 Mixin
│   ├── account.py           # 账户 / 持仓查询 Mixin
│   ├── order.py             # 下单 / 撤单 Mixin
│   ├── contract.py          # 合约信息 Mixin
│   └── error.py             # 错误处理 Mixin
├── historical/
│   └── client.py            # 独立历史数据客户端
└── parsing/                 # 数据解析与格式转换
    ├── data.py              # 行情数据解析
    ├── execution.py         # 执行回报解析
    └── instruments.py       # 证券信息解析与代码映射

examples/live/thinktrader/
├── connect_with_miniqmt.py           # 连接与实时行情订阅
├── query_account_and_positions.py    # 账户资金与持仓查询
├── historical_download.py            # 历史数据批量下载
├── buy_and_sell.py                   # 买入卖出交易示例
├── tick_momentum_strategy.py         # 多标的 Tick 动量策略
├── contract_download.py              # 合约信息查询
└── outputs/                          # 历史数据输出目录
```
