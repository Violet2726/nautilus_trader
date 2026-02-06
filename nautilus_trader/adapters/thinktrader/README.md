# ThinkTrader Adapter

此适配器将 NautilusTrader 与 ThinkTrader (MiniQmt) 生态系统集成。它通过 MiniQmt 客户端和 xtquant API 提供实时行情数据、交易执行、证券信息挖掘以及历史数据访问功能。

## 组件

- **数据客户端 (Data client)**: 处理 ThinkTrader 实时行情数据的订阅和请求。
- **执行客户端 (Execution client)**: 处理订单生命周期管理以及账户/持仓查询。
- **证券信息提供者 (Instrument provider)**: 通过 MiniQmt 加载证券元数据（标的信息）。
- **历史数据客户端 (Historical client)**: 通过 xtquant 提供历史数据访问。

## 目录结构

- `client`: 底层 MiniQmt 客户端封装及领域辅助工具。
- `historical`: 历史数据客户端实现。
- `parsing`: 证券信息、行情数据及执行回报的解析器。
- `providers.py`: ThinkTrader 证券信息提供者实现。
- `data.py`: ThinkTrader 实时行情客户端实现。
- `execution.py`: ThinkTrader 执行客户端实现。
- `factories.py`: 用于 `TradingNode` 的实盘客户端工厂类。
- `config.py`: 适配器配置数据类。

## 需求

- 已安装并配置好 ThinkTrader MiniQmt 终端。
- 有效的 MiniQmt 用户数据路径 (`userdata_mini`)。
- 用于交易执行和账户查询的有效资金账号 ID。

## 环境变量

实盘示例中常用的环境变量：

- `MINIQMT_PATH`: MiniQmt 用户数据目录的路径。
- `MINIQMT_ACCOUNT_ID`: 资金账号 ID。
- `MINIQMT_ACCOUNT_TYPE`: 账号类型，例如 `STOCK`（股票）。
- `MINIQMT_SESSION_ID`: 可选的会话 ID 覆盖。
- `TT_STOP_AFTER_SECS`: 可选的自动停止秒数（用于自动关闭的示例脚本）。

## 快速开始

运行一个简单的连接及行情订阅示例：

```bash
python examples/live/thinktrader/connect_with_miniqmt.py
```

查询账户资金及持仓：

```bash
python examples/live/thinktrader/query_account_and_positions.py
```

## 代码示例

查看 `examples/live/thinktrader` 目录下的实盘示例：

- `connect_with_miniqmt.py`: 连接并订阅实时行情。
- `query_account_and_positions.py`: 查询账户余额和持仓。
- `historical_download.py`: 下载历史数据。
- `contract_download.py`: 下载合约/证券信息。
- `comprehensive_test.py`: 适配器功能的综合测试。
- `buy_and_query_position.py`: 包含下单及持仓查询的示例。

## 配置说明

适配器使用 `ThinkTraderInstrumentProviderConfig` 来控制证券信息的加载。例如：

```python
instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_contracts_on_start=True,
)
```

执行客户端和数据客户端通过 `ThinkTraderExecClientConfig` 和 `ThinkTraderDataClientConfig` 进行配置。
在策略层，资金账号 ID 会自动添加 `THINKTRADER` 柜台前缀包装成 `AccountId`，例如：`THINKTRADER-<ACCOUNT_ID>`。
