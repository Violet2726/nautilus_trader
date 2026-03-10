# ThinkTrader 交易相关字段汇总

本文档列出了 ThinkTrader (MiniQMT) 集成中所有与交易相关的数据字段。

---

## 1. 账户资金 (Account Funds)

由 `query_stock_asset()` 返回，包含 15 个字段：

| 字段名 | 类型 | 说明 | 示例值 |
|--------|------|------|--------|
| `account_id` | str | 账户ID | "2055226" |
| `cash` | float | 现金余额 | 123456.78 |
| `frozen_cash` | float | 冻结资金 | 0.0 |
| `market_value` | float | 持仓市值 | 500000.0 |
| `total_asset` | float | 总资产 | 623456.78 |
| `login_status` | str | 登录状态 | "" |
| `account_status` | str | 账户状态 | "" |
| `update_time` | int | 更新时间戳 | 1773107084 |
| `profit_loss` | float | 盈亏 | 0.0 |
| `total_market_value` | float | 总市值 | 500000.0 |
| `withdrawable_amount` | float | 可取金额 | 123456.78 |
| `stock_market_value` | float | 股票市值 | 500000.0 |
| `fund_market_value` | float | 基金市值 | 0.0 |
| `broker_name` | str | 券商名称 | "" |
| `account_name` | str | 账户名称 | "" |

---

## 2. 持仓数据 (Positions)

由 `query_stock_positions()` 返回，转换为 `TTPosition` 结构：

### TTPosition 字段 (18 个)

| 字段名 | 类型 | 说明 | 示例值 |
|--------|------|------|--------|
| `account_id` | str | 账户ID | "2055226" |
| `stock_code` | str | 证券代码 | "000001.SZ" |
| `stock_name` | str | 证券名称 | "平安银行" |
| `volume` | int | 持仓数量 | 1000 |
| `available_volume` | int | 可用数量 | 800 |
| `pending_volume` | int | 待交收数量 | 0 |
| `frozen_volume` | int | 冻结数量 | 200 |
| `market_value` | float | 市值 | 10750.0 |
| `last_price` | float | 最新价 | 10.75 |
| `avg_price` | float | 持仓成本价 | 10.50 |
| `float_pnl` | float | 浮动盈亏 | 250.0 |
| `profit_loss_ratio` | float | 盈亏比例 | 0.0238 |
| `overnight_volume` | int | 隔夜仓 | 0 |
| `account_name` | str | 账户名称 | "" |
| `broker_name` | str | 券商名称 | "" |
| `expiry_date` | str | 到期日期 | "" |

---

## 3. 委托数据 (Orders)

由 `query_stock_orders()` 直接返回 xtquant 的订单对象，包含 38+ 个字段：

### 标准字段

| 字段名 | 类型 | 说明 | 示例值 |
|--------|------|------|--------|
| `order_id` | int | 合同编号 | 8388613 |
| `stock_code` | str | 证券代码 | "000001.SZ" |
| `stock_name` | str | 证券名称 | "平安银行" |
| `order_time` | int | 委托时间戳 | 1773107084 |
| `order_type` | int | 委托类型 | 23=买入, 24=卖出 |
| `order_status` | int | 委托状态 | 56=已成, 53=已报, 57=废单 |
| `order_volume` | int | 委托数量 | 100 |
| `traded_volume` | int | 成交数量 | 100 |
| `canceled_volume` | int | 已撤数量 | 0 |
| `price` | float | 委托价格 | 10.75 |
| `traded_price` | float | 成交均价 | 10.75 |
| `order_sysid` | str | 委托系统ID | "2026031000052350" |
| `frozen_amount` | float | 冻结金额 | 0.0 |
| `order_remark` | str | 委托备注 | "" |
| `status_msg` | str | 状态信息 | "" |
| `strategy_name` | str | 策略名称 | "" |
| `direction` | int | 买卖方向 | 48=买入, 49=卖出 |
| `offset_flag` | int | 开平标志 | 48=开仓, 49=平仓 |
| `price_type` | int | 价格类型 | 50=限价 |

### m_ 前缀字段 (与标准字段等价)

| 字段名 | 对应标准字段 |
|--------|-------------|
| `m_nOrderID` | order_id |
| `m_strStockCode` | stock_code |
| `m_strInstrumentName` | stock_name |
| `m_nOrderTime` | order_time |
| `m_nOrderType` | order_type |
| `m_nOrderStatus` | order_status |
| `m_nOrderVolume` | order_volume |
| `m_nTradedVolume` | traded_volume |
| `m_dPrice` | price |
| `m_dTradedPrice` | traded_price |
| `m_strOrderSysID` | order_sysid |
| `m_nDirection` | direction |
| `m_nOffsetFlag` | offset_flag |
| `m_nPriceType` | price_type |

---

## 4. 成交数据 (Trades)

由 `query_stock_trades()` 直接返回 xtquant 的成交对象，包含 36 个字段：

### 标准字段

| 字段名 | 类型 | 说明 | 示例值 |
|--------|------|------|--------|
| `order_id` | int | 合同编号 | 8388613 |
| `stock_code` | str | 证券代码 | "000001.SZ" |
| `stock_name` | str | 证券名称 | "平安银行" |
| `traded_time` | int | 成交时间戳 | 1773107084 |
| `traded_price` | float | 成交价格 | 10.75 |
| `traded_volume` | int | 成交数量 | 100 |
| `traded_amount` | float | 成交金额 | 1075.0 |
| `traded_id` | str | 成交编号 | "22026031000059120" |
| `order_type` | int | 委托类型 | 23=买入, 24=卖出 |
| `direction` | int | 买卖方向 | 48=买入, 49=卖出 |
| `offset_flag` | int | 开平标志 | 48=开仓, 49=平仓 |
| `commission` | float | 手续费 | 0.0 |
| `order_sysid` | str | 委托系统ID | "2026031000052350" |
| `order_remark` | str | 备注 | "" |
| `strategy_name` | str | 策略名称 | "" |
| `account_id` | str | 账户ID | "2055226" |
| `account_type` | int | 账户类型 | 2 |
| `secu_account` | str | 证券账户 | "" |

### m_ 前缀字段 (与标准字段等价)

| 字段名 | 对应标准字段 |
|--------|-------------|
| `m_nOrderID` | order_id |
| `m_strStockCode` | stock_code |
| `m_strInstrumentName` | stock_name |
| `m_nTradedTime` | traded_time |
| `m_dTradedPrice` | traded_price |
| `m_nTradedVolume` | traded_volume |
| `m_dTradedAmount` | traded_amount |
| `m_strTradedID` | traded_id |
| `m_nOrderType` | order_type |
| `m_nDirection` | direction |
| `m_nOffsetFlag` | offset_flag |
| `m_dCommission` | commission |
| `m_strOrderSysID` | order_sysid |
| `m_strOrderRemark` | order_remark |
| `m_strStrategyName` | strategy_name |
| `m_strAccountID` | account_id |
| `m_nAccountType` | account_type |

---

## 5. 常用字段速查表

### 证券信息

| 用途 | 推荐字段 |
|------|---------|
| 证券代码 | `stock_code` |
| 证券名称 | `stock_name` 或 `instrument_name` |

### 委托/成交数量

| 用途 | 字段 |
|------|------|
| 委托数量 | `order_volume` |
| 成交数量 | `traded_volume` |
| 已撤数量 | `canceled_volume` |
| 冻结数量 | `frozen_volume` |
| 可用数量 | `available_volume` |

### 价格

| 用途 | 字段 |
|------|------|
| 委托价格 | `price` |
| 成交价格 | `traded_price` / `traded_price` |
| 成交金额 | `traded_amount` |
| 成本价/均价 | `avg_price` |
| 最新价 | `last_price` |

### 状态

| 用途 | 字段 |
|------|------|
| 委托状态 | `order_status` (56=已成, 53=已报, 57=废单, 50=已撤) |
| 买卖方向 | `direction` (48=买入, 49=卖出) 或 `order_type` (23=买入, 24=卖出) |
| 开平标志 | `offset_flag` (48=开仓, 49=平仓) |

### 标识

| 用途 | 字段 |
|------|------|
| 合同编号 | `order_id` |
| 成交编号 | `traded_id` |
| 委托系统ID | `order_sysid` |
| 账户ID | `account_id` |

### 盈亏

| 用途 | 字段 |
|------|------|
| 浮动盈亏 | `float_pnl` |
| 盈亏比例 | `profit_loss_ratio` |
| 持仓市值 | `market_value` |
| 总资产 | `total_asset` |

---

## 6. 枚举值参考

### 委托类型 (order_type)

| 值 | 说明 |
|----|------|
| 23 | 买入 |
| 24 | 卖出 |

### 委托状态 (order_status)

| 值 | 说明 |
|----|------|
| 50 | 已撤 |
| 53 | 已报 |
| 56 | 已成 |
| 57 | 废单 |

### 买卖方向 (direction)

| 值 | 说明 |
|----|------|
| 48 | 买入 |
| 49 | 卖出 |

### 开平标志 (offset_flag)

| 值 | 说明 |
|----|------|
| 48 | 开仓 |
| 49 | 平仓 |

---