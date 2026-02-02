# ThinkTrader (XtQuant) API 知识库

> **版本**: v1.4  
> **更新日期**: 2026-01-30  
> **详细文档**: `交易模块文档.md`, `行情模块文档.md`

本文档整理自 XtQuant Native API 文档，涵盖行情 (`xtdata`) 和交易 (`xttrader`) 两大模块的核心功能，旨在为 Nautilus Trader 对接 ThinkTrader 提供快速参考。

---

## 目录

1. [概述](#1-概述)
2. [XtData 行情模块](#2-xtdata-行情模块)
3. [XtTrader 交易模块](#3-xttrader-交易模块)
4. [数据字典](#4-数据字典-xtquant-枚举常量)
5. [数据结构](#5-数据结构)
6. [行情数据结构](#6-行情数据结构)
7. [Nautilus Trader 适配要点](#7-nautilus-trader-适配要点)
8. [常见问题](#8-常见问题)

---

## 1. 概述

XtQuant 是迅投推出的 Python 量化接口。

| 模块 | 说明 | 导入方式 |
|-----|------|---------|
| `xtdata` | 行情模块 | `from xtquant import xtdata` |
| `xttrader` | 交易模块 | `from xtquant.xttrader import XtQuantTrader, XtQuantTraderCallback` |
| `xttype` | 类型定义 | `from xtquant.xttype import StockAccount` |
| `xtconstant` | 常量定义 | `from xtquant import xtconstant` |

**运行依赖**: 必须启动 MiniQmt 客户端（或 QMT 极简模式）并登录。

---

## 2. XtData 行情模块

### 2.1 核心接口一览

#### 订阅接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `subscribe_quote()` | 订阅单股行情 | 订阅号 seq |
| `subscribe_whole_quote()` | 订阅全推行情 | 订阅号 seq |
| `unsubscribe_quote(seq)` | 反订阅行情数据 | None |
| `run()` | 阻塞线程接收行情回调 | None |
| `subscribe_formula()` | 订阅模型 | 订阅号 seq |
| `unsubscribe_formula()` | 反订阅模型 | bool |
| `call_formula()` | 调用模型 | dict |
| `call_formula_batch()` | 批量调用模型 | list[dict] |
| `generate_factor_data()` | 生成因子数据 | - |

#### 行情数据获取

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `get_market_data()` | 获取行情数据 | dict |
| `get_market_data_ex()` | 获取行情数据 (扩展版) | dict{stock: DataFrame} |
| `get_local_data()` | 获取本地行情数据 | dict |
| `get_full_tick()` | 获取全推数据 | dict{stock: tick} |
| `get_full_kline()` | 获取最新交易日K线数据 | dict |
| `get_divid_factors()` | 获取除权数据 | dict |

#### 数据下载

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `download_history_data()` | 下载历史行情数据 | None |
| `download_history_data2()` | 下载过期(退市)合约信息 | None |
| `download_holiday_data()` | 节假日下载 | None |
| `download_cb_data()` | 可转债基础信息的下载 | None |
| `download_etf_info()` | ETF申赎清单信息下载 | None |
| `download_sector_data()` | 下载板块分类信息 | None |
| `download_financial_data()` | 下载财务数据 | None |
| `download_index_weight()` | 下载指数成分权重信息 | None |

#### 财务数据接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `get_financial_data()` | 获取财务数据 | dict |
| `download_financial_data()` | 下载财务数据 | None |

#### 基础行情信息

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `get_instrument_detail()` | 获取合约基础信息 | dict |
| `get_instrument_type()` | 获取合约类型 | dict |
| `get_trading_dates()` | 获取交易日列表 | list |
| `get_trading_calendar()` | 获取交易日历 | list |
| `get_trading_time()` | 获取交易时间段 | list |
| `get_holidays()` | 获取节假日数据 | list |
| `get_cb_data()` | 获取可转债基础信息 | dict |
| `get_ipo_info()` | 获取新股申购信息 | dict |
| `get_period_list()` | 获取可用周期列表 | list |
| `get_etf_info()` | ETF申赎清单信息获取 | dict |

#### 板块管理

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `get_sector_list()` | 获取板块列表 | list |
| `get_stock_list_in_sector()` | 获取板块成分股列表 | list |
| `create_sector_folder_node()` | 创建板块目录节点 | bool |
| `create_sector_node()` | 创建板块 | bool |
| `add_sector_stocks()` | 添加自定义板块 | bool |
| `remove_sector_stocks()` | 移除板块成分股 | bool |
| `remove_sector_node()` | 移除自定义板块 | bool |
| `reset_sector_stocks()` | 重置板块 | bool |

#### 指数数据

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `get_index_weight()` | 获取指数成分权重信息 | dict |
| `download_index_weight()` | 下载指数成分权重信息 | None |


---

## 3. XtTrader 交易模块

### 3.1 核心接口一览

#### 系统设置接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `XtQuantTrader(path, session_id)` | 创建API实例 | XtQuantTrader |
| `register_callback(callback)` | 注册回调类 | None |
| `start()` | 准备API环境 | None |
| `connect()` | 创建连接 | 0=成功 |
| `stop()` | 停止运行 | None |
| `run_forever()` | 阻塞当前线程进入等待状态 | None |
| `set_relaxed_response_order_enabled(enabled)` | 开启主动请求接口的专用线程 | None |

#### 账号操作接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `subscribe(account)` | 订阅账号信息 | 0=成功 |
| `unsubscribe(account)` | 反订阅账号信息 | 0=成功 |

#### 订单操作接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `order_stock()` | 股票同步报单 | order_id |
| `order_stock_async()` | 股票异步报单 | seq |
| `cancel_order_stock()` | 股票同步撤单 (按order_id) | 0=成功 |
| `cancel_order_stock_sysid()` | 股票同步撤单 (按order_sysid) | 0=成功 |
| `cancel_order_stock_async()` | 股票异步撤单 (按order_id) | seq |
| `cancel_order_stock_sysid_async()` | 股票异步撤单 (按order_sysid) | seq |
| `fund_transfer()` | 资金划拨 | (bool, str) |
| `order_external()` | 外部交易数据录入 | order_id |

#### 股票查询接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `query_stock_asset()` | 资产查询 | XtAsset |
| `query_stock_orders()` | 委托查询 | list[XtOrder] |
| `query_stock_trades()` | 成交查询 | list[XtTrade] |
| `query_stock_positions()` | 持仓查询 | list[XtPosition] |
| `query_position_statistics()` | 期货持仓统计查询 | list[XtPositionStatistics] |

#### 信用查询接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `query_credit_detail()` | 信用资产查询 | XtCreditDetail |
| `query_stk_compacts()` | 负债合约查询 | list[StkCompacts] |
| `query_credit_subjects()` | 融资融券标的查询 | list |
| `query_credit_slo_code()` | 可融券数据查询 | list |
| `query_credit_assure()` | 标的担保品查询 | list |

#### 其他查询接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `query_new_purchase_limit()` | 新股申购额度查询 | dict |
| `query_ipo_data()` | 当日新股信息查询 | list |
| `query_account_infos()` | 账号信息查询 | list[XtAccountInfo] |
| `query_account_status()` | 账号状态查询 | list[XtAccountStatus] |
| `query_com_fund()` | 普通柜台资金查询 | dict |
| `query_com_position()` | 普通柜台持仓查询 | list |
| `export_data()` | 通用数据导出 | str |
| `query_data()` | 通用数据查询 | list |

#### 约券相关接口

| 接口 | 功能 | 返回值 |
|-----|------|--------|
| `query_smt_quoter()` | 券源行情查询 | list |
| `smt_appointment()` | 库存券约券申请 | int |
| `query_smt_compact()` | 约券合约查询 | list |

#### 回调类 (XtQuantTraderCallback)

| 方法 | 功能 | 参数类型 |
|-----|------|----------|
| `on_disconnected()` | 连接状态回调 | None |
| `on_account_status(status)` | 账号状态信息推送 | XtAccountStatus |
| `on_stock_order(order)` | 委托信息推送 | XtOrder |
| `on_stock_trade(trade)` | 成交信息推送 | XtTrade |
| `on_order_error(order_error)` | 下单失败信息推送 | XtOrderError |
| `on_cancel_error(cancel_error)` | 撤单失败信息推送 | XtCancelError |
| `on_order_stock_async_response(response)` | 异步下单回报推送 | XtOrderResponse |
| `on_smt_appointment_async_response(response)` | 约券相关异步接口的回报推送 | XtSmtAppointmentResponse |


---

## 4. 数据字典 (XtQuant 枚举常量)

### 4.1 交易市场(market)

- 上交所 - `xtconstant.SH_MARKET`
- 深交所 - `xtconstant.SZ_MARKET`
- 北交所 - `xtconstant.MARKET_ENUM_BEIJING`
- 沪港通 - `xtconstant.MARKET_ENUM_SHANGHAI_HONGKONG_STOCK`
- 深港通 - `xtconstant.MARKET_ENUM_SHENZHEN_HONGKONG_STOCK`
- 上期所 - `xtconstant.MARKET_ENUM_SHANGHAI_FUTURE`
- 大商所 - `xtconstant.MARKET_ENUM_DALIANG_FUTURE`
- 郑商所 - `xtconstant.MARKET_ENUM_ZHENGZHOU_FUTURE`
- 中金所 - `xtconstant.MARKET_ENUM_INDEX_FUTURE`
- 能源中心 - `xtconstant.MARKET_ENUM_INTL_ENERGY_FUTURE`
- 广期所 - `xtconstant.MARKET_ENUM_GUANGZHOU_FUTURE`
- 上海期权 - `xtconstant.MARKET_ENUM_SHANGHAI_STOCK_OPTION`
- 深证期权 - `xtconstant.MARKET_ENUM_SHENZHEN_STOCK_OPTION`

### 4.2 账号类型(account_type)

- 期货 - `xtconstant.FUTURE_ACCOUNT`
- 股票 - `xtconstant.SECURITY_ACCOUNT`
- 信用 - `xtconstant.CREDIT_ACCOUNT`
- 期货期权 - `xtconstant.FUTURE_OPTION_ACCOUNT`
- 股票期权 - `xtconstant.STOCK_OPTION_ACCOUNT`
- 沪港通 - `xtconstant.HUGANGTONG_ACCOUNT`
- 深港通 - `xtconstant.SHENGANGTONG_ACCOUNT`

### 4.3 委托类型(order_type)

- 股票
  - 买入 - `xtconstant.STOCK_BUY`
  - 卖出 - `xtconstant.STOCK_SELL`
- 信用
  - 担保品买入 - `xtconstant.CREDIT_BUY`
  - 担保品卖出 - `xtconstant.CREDIT_SELL`
  - 融资买入 - `xtconstant.CREDIT_FIN_BUY`
  - 融券卖出 - `xtconstant.CREDIT_SLO_SELL`
  - 买券还券 - `xtconstant.CREDIT_BUY_SECU_REPAY`
  - 直接还券 - `xtconstant.CREDIT_DIRECT_SECU_REPAY`
  - 卖券还款 - `xtconstant.CREDIT_SELL_SECU_REPAY`
  - 直接还款 - `xtconstant.CREDIT_DIRECT_CASH_REPAY`
  - 专项融资买入 - `xtconstant.CREDIT_FIN_BUY_SPECIAL`
  - 专项融券卖出 - `xtconstant.CREDIT_SLO_SELL_SPECIAL`
  - 专项买券还券 - `xtconstant.CREDIT_BUY_SECU_REPAY_SPECIAL`
  - 专项直接还券 - `xtconstant.CREDIT_DIRECT_SECU_REPAY_SPECIAL`
  - 专项卖券还款 - `xtconstant.CREDIT_SELL_SECU_REPAY_SPECIAL`
  - 专项直接还款 - `xtconstant.CREDIT_DIRECT_CASH_REPAY_SPECIAL`
- 期货六键风格
  - 开多 - `xtconstant.FUTURE_OPEN_LONG`
  - 平昨多 - `xtconstant.FUTURE_CLOSE_LONG_HISTORY`
  - 平今多 - `xtconstant.FUTURE_CLOSE_LONG_TODAY`
  - 开空 - `xtconstant.FUTURE_OPEN_SHORT`
  - 平昨空 - `xtconstant.FUTURE_CLOSE_SHORT_HISTORY`
  - 平今空 - `xtconstant.FUTURE_CLOSE_SHORT_TODAY`
- 期货四键风格
  - 平多，优先平今 - `xtconstant.FUTURE_CLOSE_LONG_TODAY_FIRST`
  - 平多，优先平昨 - `xtconstant.FUTURE_CLOSE_LONG_HISTORY_FIRST`
  - 平空，优先平今 - `xtconstant.FUTURE_CLOSE_SHORT_TODAY_FIRST`
  - 平空，优先平昨 - `xtconstant.FUTURE_CLOSE_SHORT_HISTORY_FIRST`
- 期货两键风格
  - 卖出，如有多仓，优先平仓，优先平今，如有余量，再开空 - `xtconstant.FUTURE_CLOSE_LONG_TODAY_HISTORY_THEN_OPEN_SHORT`
  - 卖出，如有多仓，优先平仓，优先平昨，如有余量，再开空 - `xtconstant.FUTURE_CLOSE_LONG_HISTORY_TODAY_THEN_OPEN_SHORT`
  - 买入，如有空仓，优先平仓，优先平今，如有余量，再开多 - `xtconstant.FUTURE_CLOSE_SHORT_TODAY_HISTORY_THEN_OPEN_LONG`
  - 买入，如有空仓，优先平仓，优先平昨，如有余量，再开多 - `xtconstant.FUTURE_CLOSE_SHORT_HISTORY_TODAY_THEN_OPEN_LONG`
  - 买入，不优先平仓 - `xtconstant.FUTURE_OPEN`
  - 卖出，不优先平仓 - `xtconstant.FUTURE_CLOSE`
- 期货 - 跨商品套利
  - 开仓 - `xtconstant.FUTURE_ARBITRAGE_OPEN`
  - 平, 优先平昨 - `xtconstant.FUTURE_ARBITRAGE_CLOSE_HISTORY_FIRST`
  - 平, 优先平今 - `xtconstant.FUTURE_ARBITRAGE_CLOSE_TODAY_FIRST`
- 期货展期
  - 看多, 优先平昨 - `xtconstant.FUTURE_RENEW_LONG_CLOSE_HISTORY_FIRST`
  - 看多，优先平今 - `xtconstant.FUTURE_RENEW_LONG_CLOSE_TODAY_FIRST`
  - 看空，优先平昨 - `xtconstant.FUTURE_RENEW_SHORT_CLOSE_HISTORY_FIRST`
  - 看空，优先平今 - `xtconstant.FUTURE_RENEW_SHORT_CLOSE_TODAY_FIRST`
- 股票期权
  - 买入开仓，以下用于个股期权交易业务 - `xtconstant.STOCK_OPTION_BUY_OPEN`
  - 卖出平仓 - `xtconstant.STOCK_OPTION_SELL_CLOSE`
  - 卖出开仓 - `xtconstant.STOCK_OPTION_SELL_OPEN`
  - 买入平仓 - `xtconstant.STOCK_OPTION_BUY_CLOSE`
  - 备兑开仓 - `xtconstant.STOCK_OPTION_COVERED_OPEN`
  - 备兑平仓 - `xtconstant.STOCK_OPTION_COVERED_CLOSE`
  - 认购行权 - `xtconstant.STOCK_OPTION_CALL_EXERCISE`
  - 认沽行权 - `xtconstant.STOCK_OPTION_PUT_EXERCISE`
  - 证券锁定 - `xtconstant.STOCK_OPTION_SECU_LOCK`
  - 证券解锁 - `xtconstant.STOCK_OPTION_SECU_UNLOCK`
- 期货期权
  - 期货期权行权 - `xtconstant.OPTION_FUTURE_OPTION_EXERCISE`
- ETF申赎
  - 申购 - `xtconstant.ETF_PURCHASE`
  - 赎回 - `xtconstant.ETF_REDEMPTION`

### 4.4 报价类型(price_type)

提示

1. 市价类型只在实盘环境中生效，模拟环境不支持市价方式报单

- 最新价 - `xtconstant.LATEST_PRICE`
- 指定价 - `xtconstant.FIX_PRICE`
- 郑商所 期货
  - 市价最优价 - `xtconstant.MARKET_BEST`
- 大商所 期货
  - 市价即成剩撤 - `xtconstant.MARKET_CANCEL`
  - 市价全额成交或撤 - `xtconstant.MARKET_CANCEL_ALL`
- 中金所 期货
  - 市价最优一档即成剩撤 - `xtconstant.MARKET_CANCEL_1`
  - 市价最优五档即成剩撤 - `xtconstant.MARKET_CANCEL_5`
  - 市价最优一档即成剩转 - `xtconstant.MARKET_CONVERT_1`
  - 市价最优五档即成剩转 - `xtconstant.MARKET_CONVERT_5`
- 上交所/北交所 股票
  - 最优五档即时成交剩余撤销 - `xtconstant.MARKET_SH_CONVERT_5_CANCEL`
  - 最优五档即时成交剩转限价 - `xtconstant.MARKET_SH_CONVERT_5_LIMIT`
  - 对手方最优价格委托 - `xtconstant.MARKET_PEER_PRICE_FIRST`
  - 本方最优价格委托 - `xtconstant.MARKET_MINE_PRICE_FIRST`
- 深交所 股票 期权
  - 对手方最优价格委托 - `xtconstant.MARKET_PEER_PRICE_FIRST`
  - 本方最优价格委托 - `xtconstant.MARKET_MINE_PRICE_FIRST`
  - 即时成交剩余撤销委托 - `xtconstant.MARKET_SZ_INSTBUSI_RESTCANCEL`
  - 最优五档即时成交剩余撤销 - `xtconstant.MARKET_SZ_CONVERT_5_CANCEL`
  - 全额成交或撤销委托 - `xtconstant.MARKET_SZ_FULL_OR_CANCEL`

### 4.5 委托状态(order_status)

| 枚举变量名                       | 值   | 含义                                     |
| -------------------------------- | ---- | ---------------------------------------- |
| xtconstant.ORDER_UNREPORTED      | 48   | 未报                                     |
| xtconstant.ORDER_WAIT_REPORTING  | 49   | 待报                                     |
| xtconstant.ORDER_REPORTED        | 50   | 已报                                     |
| xtconstant.ORDER_REPORTED_CANCEL | 51   | 已报待撤                                 |
| xtconstant.ORDER_PARTSUCC_CANCEL | 52   | 部成待撤                                 |
| xtconstant.ORDER_PART_CANCEL     | 53   | 部撤（已经有一部分成交，剩下的已经撤单） |
| xtconstant.ORDER_CANCELED        | 54   | 已撤                                     |
| xtconstant.ORDER_PART_SUCC       | 55   | 部成（已经有一部分成交，剩下的待成交）   |
| xtconstant.ORDER_SUCCEEDED       | 56   | 已成                                     |
| xtconstant.ORDER_JUNK            | 57   | 废单                                     |
| xtconstant.ORDER_UNKNOWN         | 255  | 未知                                     |

### 4.6 账号状态(account_status)

| 枚举变量名                              | 值   | 含义                              |
| --------------------------------------- | ---- | --------------------------------- |
| xtconstant.ACCOUNT_STATUS_INVALID       | -1   | 无效                              |
| xtconstant.ACCOUNT_STATUS_OK            | 0    | 正常                              |
| xtconstant.ACCOUNT_STATUS_WAITING_LOGIN | 1    | 连接中                            |
| xtconstant.ACCOUNT_STATUSING            | 2    | 登陆中                            |
| xtconstant.ACCOUNT_STATUS_FAIL          | 3    | 失败                              |
| xtconstant.ACCOUNT_STATUS_INITING       | 4    | 初始化中                          |
| xtconstant.ACCOUNT_STATUS_CORRECTING    | 5    | 数据刷新校正中                    |
| xtconstant.ACCOUNT_STATUS_CLOSED        | 6    | 收盘后                            |
| xtconstant.ACCOUNT_STATUS_ASSIS_FAIL    | 7    | 穿透副链接断开                    |
| xtconstant.ACCOUNT_STATUS_DISABLEBYSYS  | 8    | 系统停用（总线使用-密码错误超限） |
| xtconstant.ACCOUNT_STATUS_DISABLEBYUSER | 9    | 用户停用（总线使用）              |

### 4.7 划拨方向(transfer_direction)

| 枚举变量名                                | 值   | 含义                            |
| ----------------------------------------- | ---- | ------------------------------- |
| xtconstant.FUNDS_TRANSFER_NORMAL_TO_SPEED | 510  | 资金划拨-普通柜台到极速柜台     |
| xtconstant.FUNDS_TRANSFER_SPEED_TO_NORMAL | 511  | 资金划拨-极速柜台到普通柜台     |
| xtconstant.NODE_FUNDS_TRANSFER_SH_TO_SZ   | 512  | 节点资金划拨-上海节点到深圳节点 |
| xtconstant.NODE_FUNDS_TRANSFER_SZ_TO_SH   | 513  | 节点资金划拨-深圳节点到上海节点 |

### 4.8 多空方向(direction)

| 枚举变量名                      | 值   | 含义 |
| ------------------------------- | ---- | ---- |
| xtconstant.DIRECTION_FLAG_LONG  | 48   | 多   |
| xtconstant.DIRECTION_FLAG_SHORT | 49   | 空   |

### 4.9 交易操作(offset_flag)

| 枚举变量名                             | 值   | 含义       |
| -------------------------------------- | ---- | ---------- |
| xtconstant.OFFSET_FLAG_OPEN            | 48   | 买入，开仓 |
| xtconstant.OFFSET_FLAG_CLOSE           | 49   | 卖出，平仓 |
| xtconstant.OFFSET_FLAG_FORCECLOSE      | 50   | 强平       |
| xtconstant.OFFSET_FLAG_CLOSETODAY      | 51   | 平今       |
| xtconstant.OFFSET_FLAG_ClOSEYESTERDAY  | 52   | 平昨       |
| xtconstant.OFFSET_FLAG_FORCEOFF        | 53   | 强减       |
| xtconstant.OFFSET_FLAG_LOCALFORCECLOSE | 54   | 本地强平   |

---

## 5. 数据结构

### 资产XtAsset

| 属性         | 类型  | 注释                                                         |
| ------------ | ----- | ------------------------------------------------------------ |
| account_type | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id   | str   | 资金账号                                                     |
| cash         | float | 可用金额                                                     |
| frozen_cash  | float | 冻结金额                                                     |
| market_value | float | 持仓市值                                                     |
| total_asset  | float | 总资产                                                       |

### 委托XtOrder

| 属性          | 类型  | 注释                                                         |
| ------------- | ----- | ------------------------------------------------------------ |
| account_type  | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id    | str   | 资金账号                                                     |
| stock_code    | str   | 证券代码，例如"600000.SH"                                    |
| order_id      | int   | 订单编号                                                     |
| order_sysid   | str   | 柜台合同编号                                                 |
| order_time    | int   | 报单时间                                                     |
| order_type    | int   | 委托类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#委托类型-order-type) |
| order_volume  | int   | 委托数量                                                     |
| price_type    | int   | 报价类型，该字段在返回时为柜台返回类型，不等价于下单传入的price_type，枚举值不一样功能一样，参见[数据字典](https://dict.thinktrader.net/innerApi/enum_constants.html#enum-ebrokerpricetype-价格类型) |
| price         | float | 委托价格                                                     |
| traded_volume | int   | 成交数量                                                     |
| traded_price  | float | 成交均价                                                     |
| order_status  | int   | 委托状态，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#委托状态-order-status) |
| status_msg    | str   | 委托状态描述，如废单原因                                     |
| strategy_name | str   | 策略名称                                                     |
| order_remark  | str   | 委托备注，最大 24 个英文字符                                 |
| direction     | int   | 多空方向，股票不适用；参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#多空方向-direction) |
| offset_flag   | int   | 交易操作，用此字段区分股票买卖，期货开、平仓，期权买卖等；参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#交易操作-offset-flag) |

### 成交XtTrade

| 属性          | 类型  | 注释                                                         |
| ------------- | ----- | ------------------------------------------------------------ |
| account_type  | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id    | str   | 资金账号                                                     |
| stock_code    | str   | 证券代码                                                     |
| order_type    | int   | 委托类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#委托类型-order-type) |
| traded_id     | str   | 成交编号                                                     |
| traded_time   | int   | 成交时间                                                     |
| traded_price  | float | 成交均价                                                     |
| traded_volume | int   | 成交数量                                                     |
| traded_amount | float | 成交金额                                                     |
| order_id      | int   | 订单编号                                                     |
| order_sysid   | str   | 柜台合同编号                                                 |
| strategy_name | str   | 策略名称                                                     |
| order_remark  | str   | 委托备注，最大 24 个英文字符(                                |
| direction     | int   | 多空方向，股票不适用；参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#多空方向-direction) |
| offset_flag   | int   | 交易操作，用此字段区分股票买卖，期货开、平仓，期权买卖等；参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#交易操作-offset-flag) |

### 持仓XtPosition

| 属性             | 类型  | 注释                                                         |
| ---------------- | ----- | ------------------------------------------------------------ |
| account_type     | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id       | str   | 资金账号                                                     |
| stock_code       | str   | 证券代码                                                     |
| volume           | int   | 持仓数量                                                     |
| can_use_volume   | int   | 可用数量                                                     |
| open_price       | float | 开仓价（返回与成本价一致）                                   |
| market_value     | float | 市值                                                         |
| frozen_volume    | int   | 冻结数量                                                     |
| on_road_volume   | int   | 在途股份                                                     |
| yesterday_volume | int   | 昨夜拥股                                                     |
| avg_price        | float | 成本价                                                       |
| direction        | int   | 多空方向，股票不适用；参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#多空方向-direction) |

### 期货持仓统计XtPositionStatistics

| 属性                               | 类型   | 注释                                                         |
| ---------------------------------- | ------ | ------------------------------------------------------------ |
| account_id                         | string | 账户                                                         |
| exchange_id                        | string | 市场代码                                                     |
| exchange_name                      | string | 市场名称                                                     |
| product_id                         | string | 品种代码                                                     |
| instrument_id                      | string | 合约代码                                                     |
| instrument_name                    | string | 合约名称                                                     |
| direction                          | int    | 多空方向，股票不适用；参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#多空方向-direction) |
| hedge_flag                         | int    | 投保类型；参见[投保类型](https://dict.thinktrader.net/innerApi/enum_constants.html#enum-ehedge-flag-type) |
| position                           | int    | 持仓数量                                                     |
| yesterday_position                 | int    | 昨仓数量                                                     |
| today_position                     | int    | 今仓数量                                                     |
| can_close_vol                      | int    | 可平数量                                                     |
| position_cost                      | float  | 持仓成本                                                     |
| avg_price                          | float  | 持仓均价                                                     |
| position_profit                    | float  | 持仓盈亏                                                     |
| float_profit                       | float  | 浮动盈亏                                                     |
| open_price                         | float  | 开仓均价                                                     |
| open_cost                          | float  | 开仓成本                                                     |
| used_margin                        | float  | 已使用保证金                                                 |
| used_commission                    | float  | 已使用的手续费                                               |
| frozen_margin                      | float  | 冻结保证金                                                   |
| frozen_commission                  | float  | 冻结手续费                                                   |
| instrument_value                   | float  | 市值，合约价值                                               |
| open_times                         | int    | 开仓次数                                                     |
| open_volume                        | int    | 总开仓量 中间平仓不减                                        |
| cancel_times                       | int    | 撤单次数                                                     |
| last_price                         | float  | 最新价                                                       |
| rise_ratio                         | float  | 当日涨幅                                                     |
| product_name                       | string | 产品名称                                                     |
| royalty                            | float  | 权利金市值                                                   |
| expire_date                        | string | 到期日                                                       |
| assest_weight                      | float  | 资产占比                                                     |
| increase_by_settlement             | float  | 当日涨幅（结）                                               |
| margin_ratio                       | float  | 保证金占比                                                   |
| float_profit_divide_by_used_margin | float  | 浮盈比例（保证金）                                           |
| float_profit_divide_by_balance     | float  | 浮盈比例（动态权益）                                         |
| today_profit_loss                  | float  | 当日盈亏（结）                                               |
| yesterday_init_position            | int    | 昨日持仓                                                     |
| frozen_royalty                     | float  | 冻结权利金                                                   |
| today_close_profit_loss            | float  | 当日盈亏（收）                                               |
| close_profit                       | float  | 平仓盈亏                                                     |
| ft_product_name                    | string | 品种名称                                                     |

### 异步下单委托反馈XtOrderResponse

| 属性          | 类型 | 注释                                                         |
| ------------- | ---- | ------------------------------------------------------------ |
| account_type  | int  | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id    | str  | 资金账号                                                     |
| order_id      | int  | 订单编号                                                     |
| strategy_name | str  | 策略名称                                                     |
| order_remark  | str  | 委托备注                                                     |
| seq           | int  | 异步下单的请求序号                                           |

### 异步撤单委托反馈XtCancelOrderResponse

| 属性          | 类型 | 注释                                                         |
| ------------- | ---- | ------------------------------------------------------------ |
| account_type  | int  | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id    | str  | 资金账号                                                     |
| order_id      | int  | 订单编号                                                     |
| order_sysid   | str  | 柜台委托编号                                                 |
| cancel_result | int  | 撤单结果（0 成功，-1 失败）                                  |
| seq           | int  | 异步撤单的请求序号                                           |

### 下单失败错误XtOrderError

| 属性          | 类型 | 注释                                                         |
| ------------- | ---- | ------------------------------------------------------------ |
| account_type  | int  | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id    | str  | 资金账号                                                     |
| order_id      | int  | 订单编号                                                     |
| error_id      | int  | 下单失败错误码                                               |
| error_msg     | str  | 下单失败具体信息                                             |
| strategy_name | str  | 策略名称                                                     |
| order_remark  | str  | 委托备注                                                     |

### 撤单失败错误XtCancelError

| 属性         | 类型 | 注释                                                         |
| ------------ | ---- | ------------------------------------------------------------ |
| account_type | int  | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id   | str  | 资金账号                                                     |
| order_id     | int  | 订单编号                                                     |
| market       | int  | 交易市场 0:上海 1:深圳                                       |
| order_sysid  | str  | 柜台委托编号                                                 |
| error_id     | int  | 下单失败错误码                                               |
| error_msg    | str  | 下单失败具体信息                                             |

### 信用账号资产XtCreditDetail

| 属性                     | 类型  | 注释                                                         |
| ------------------------ | ----- | ------------------------------------------------------------ |
| account_type             | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id               | str   | 资金账号                                                     |
| m_nStatus                | int   | 账号状态                                                     |
| m_nUpdateTime            | int   | 更新时间                                                     |
| m_nCalcConfig            | int   | 计算参数                                                     |
| m_dFrozenCash            | float | 冻结金额                                                     |
| m_dBalance               | float | 总资产                                                       |
| m_dAvailable             | float | 可用金额                                                     |
| m_dPositionProfit        | float | 持仓盈亏                                                     |
| m_dMarketValue           | float | 总市值                                                       |
| m_dFetchBalance          | float | 可取金额                                                     |
| m_dStockValue            | float | 股票市值                                                     |
| m_dFundValue             | float | 基金市值                                                     |
| m_dTotalDebt             | float | 总负债                                                       |
| m_dEnableBailBalance     | float | 可用保证金                                                   |
| m_dPerAssurescaleValue   | float | 维持担保比例                                                 |
| m_dAssureAsset           | float | 净资产                                                       |
| m_dFinDebt               | float | 融资负债                                                     |
| m_dFinDealAvl            | float | 融资本金                                                     |
| m_dFinFee                | float | 融资息费                                                     |
| m_dSloDebt               | float | 融券负债                                                     |
| m_dSloMarketValue        | float | 融券市值                                                     |
| m_dSloFee                | float | 融券息费                                                     |
| m_dOtherFare             | float | 其它费用                                                     |
| m_dFinMaxQuota           | float | 融资授信额度                                                 |
| m_dFinEnableQuota        | float | 融资可用额度                                                 |
| m_dFinUsedQuota          | float | 融资冻结额度                                                 |
| m_dSloMaxQuota           | float | 融券授信额度                                                 |
| m_dSloEnableQuota        | float | 融券可用额度                                                 |
| m_dSloUsedQuota          | float | 融券冻结额度                                                 |
| m_dSloSellBalance        | float | 融券卖出资金                                                 |
| m_dUsedSloSellBalance    | float | 已用融券卖出资金                                             |
| m_dSurplusSloSellBalance | float | 剩余融券卖出资金                                             |

### 负债合约StkCompacts

| 属性                 | 类型  | 注释                                                         |
| -------------------- | ----- | ------------------------------------------------------------ |
| account_type         | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id           | str   | 资金账号                                                     |
| compact_type         | int   | 合约类型                                                     |
| cashgroup_prop       | int   | 头寸来源                                                     |
| exchange_id          | int   | 证券市场                                                     |
| open_date            | int   | 开仓日期                                                     |
| business_vol         | int   | 合约证券数量                                                 |
| real_compact_vol     | int   | 未还合约数量                                                 |
| ret_end_date         | int   | 到期日                                                       |
| business_balance     | float | 合约金额                                                     |
| businessFare         | float | 合约息费                                                     |
| real_compact_balance | float | 未还合约金额                                                 |
| real_compact_fare    | float | 未还合约息费                                                 |
| repaid_fare          | float | 已还息费                                                     |
| repaid_balance       | float | 已还金额                                                     |
| instrument_id        | str   | 证券代码                                                     |
| compact_id           | str   | 合约编号                                                     |
| position_str         | str   | 定位串                                                       |

### 融资融券标的CreditSubjects

| 属性          | 类型  | 注释                                                         |
| ------------- | ----- | ------------------------------------------------------------ |
| account_type  | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id    | str   | 资金账号                                                     |
| slo_status    | int   | 融券状态                                                     |
| fin_status    | int   | 融资状态                                                     |
| exchange_id   | int   | 证券市场                                                     |
| slo_ratio     | float | 融券保证金比例                                               |
| fin_ratio     | float | 融资保证金比例                                               |
| instrument_id | str   | 证券代码                                                     |

### 可融券数据CreditSloCode

| 属性           | 类型 | 注释                                                         |
| -------------- | ---- | ------------------------------------------------------------ |
| account_type   | int  | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id     | str  | 资金账号                                                     |
| cashgroup_prop | int  | 头寸来源                                                     |
| exchange_id    | int  | 证券市场                                                     |
| enable_amount  | int  | 融券可融数量                                                 |
| instrument_id  | str  | 证券代码                                                     |

### 标的担保品CreditAssure

| 属性          | 类型  | 注释                                                         |
| ------------- | ----- | ------------------------------------------------------------ |
| account_type  | int   | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id    | str   | 资金账号                                                     |
| assure_status | int   | 是否可做担保                                                 |
| exchange_id   | int   | 证券市场                                                     |
| assure_ratio  | float | 担保品折算比例                                               |
| instrument_id | str   | 证券代码                                                     |

### 账号状态XtAccountStatus

| 属性         | 类型 | 注释                                                         |
| ------------ | ---- | ------------------------------------------------------------ |
| account_type | int  | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id   | str  | 资金账号                                                     |
| status       | int  | 账号状态，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号状态-account-status) |

### 账号信息XtAccountInfo

| 属性                   | 类型 | 注释                                                         |
| ---------------------- | ---- | ------------------------------------------------------------ |
| account_type           | int  | 账号类型，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号类型-account-type) |
| account_id             | str  | 资金账号                                                     |
| broker_type            | int  | 同 account_type                                              |
| platform_id            | int  | 平台号                                                       |
| account_classification | int  | 账号分类                                                     |
| login_status           | int  | 账号状态，参见[数据字典](http://dict.thinktrader.net/nativeApi/xttrader.html#账号状态-account-status) |

### 约券相关异步接口的反馈XtSmtAppointmentResponse

| 属性     | 类型 | 注释                                   |
| -------- | ---- | -------------------------------------- |
| seq      | int  | 异步请求序号                           |
| success  | bool | 申请是否成功                           |
| msg      | str  | 反馈信息                               |
| apply_id | str  | 若申请成功返回资券申请编号，否则返回-1 |


---

## 6. 行情数据结构

### 6.1 周期类型 (period)

| 类型 | 周期值 | 说明 |
|-----|--------|------|
| Level1 | `tick` | 分笔数据 |
| Level1 | `1m`, `5m`, `15m`, `30m`, `1h` | 分钟/小时线 |
| Level1 | `1d` | 日线 |
| Level1 | `1w`, `1mon`, `1q`, `1hy`, `1y` | 周/月/季/半年/年线 |
| Level2 | `l2quote` | Level2实时行情快照 |
| Level2 | `l2order` | Level2逐笔委托 |
| Level2 | `l2transaction` | Level2逐笔成交 |
| Level2 | `l2quoteaux` | Level2实时行情补充（总买总卖） |
| Level2 | `l2orderqueue` | Level2委买委卖一档委托队列 |

### 6.2 tick - 分笔数据

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 |
| `lastPrice` | 最新价 |
| `open` | 开盘价 |
| `high` | 最高价 |
| `low` | 最低价 |
| `lastClose` | 前收盘价 |
| `amount` | 成交总额 |
| `volume` | 成交总量 |
| `pvolume` | 原始成交总量 |
| `stockStatus` | 证券状态 |
| `openInt` | 持仓量 |
| `lastSettlementPrice` | 前结算 |
| `askPrice` | 委卖价 |
| `bidPrice` | 委买价 |
| `askVol` | 委卖量 |
| `bidVol` | 委买量 |
| `transactionNum` | 成交笔数 |

### 6.3 1m / 5m / 1d - K线数据

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 |
| `open` | 开盘价 |
| `high` | 最高价 |
| `low` | 最低价 |
| `close` | 收盘价 |
| `volume` | 成交量 |
| `amount` | 成交额 |
| `settelementPrice` | 今结算 |
| `openInterest` | 持仓量 |
| `preClose` | 前收价 |
| `suspendFlag` | 停牌标记 (0=正常, 1=停牌, -1=当日起复牌) |

### 6.4 除权数据

| 字段 | 说明 |
|-----|------|
| `interest` | 每股股利（税前，元） |
| `stockBonus` | 每股红股（股） |
| `stockGift` | 每股转增股本（股） |
| `allotNum` | 每股配股数（股） |
| `allotPrice` | 配股价格（元） |
| `gugai` | 是否股改 |
| `dr` | 除权系数 |

### 6.5 l2quote - Level2实时行情快照

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 |
| `lastPrice` | 最新价 |
| `open` | 开盘价 |
| `high` | 最高价 |
| `low` | 最低价 |
| `amount` | 成交额 |
| `volume` | 成交总量 |
| `pvolume` | 原始成交总量 |
| `openInt` | 持仓量 |
| `stockStatus` | 证券状态 |
| `transactionNum` | 成交笔数 |
| `lastClose` | 前收盘价 |
| `lastSettlementPrice` | 前结算 |
| `settlementPrice` | 今结算 |
| `pe` | 市盈率 |
| `askPrice` | 多档委卖价 |
| `bidPrice` | 多档委买价 |
| `askVol` | 多档委卖量 |
| `bidVol` | 多档委买量 |

### 6.6 l2order - Level2逐笔委托

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 |
| `price` | 委托价 |
| `volume` | 委托量 |
| `entrustNo` | 委托号 |
| `entrustType` | 委托类型 |
| `entrustDirection` | 委托方向 |

### 6.7 l2transaction - Level2逐笔成交

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 |
| `price` | 成交价 |
| `volume` | 成交量 |
| `amount` | 成交额 |
| `tradeIndex` | 成交记录号 |
| `buyNo` | 买方委托号 |
| `sellNo` | 卖方委托号 |
| `tradeType` | 成交类型 |
| `tradeFlag` | 成交标志 |

### 6.8 l2quoteaux - Level2实时行情补充（总买总卖）

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 |
| `avgBidPrice` | 委买均价 |
| `totalBidQuantity` | 委买总量 |
| `avgOffPrice` | 委卖均价 |
| `totalOffQuantity` | 委卖总量 |
| `withdrawBidQuantity` | 买入撤单总量 |
| `withdrawBidAmount` | 买入撤单总额 |
| `withdrawOffQuantity` | 卖出撤单总量 |
| `withdrawOffAmount` | 卖出撤单总额 |

### 6.9 l2orderqueue - Level2委买委卖一档委托队列

| 字段 | 说明 |
|-----|------|
| `time` | 时间戳 |
| `bidLevelPrice` | 委买价 |
| `bidLevelVolume` | 委买量 |
| `offerLevelPrice` | 委卖价 |
| `offerLevelVolume` | 委卖量 |
| `bidLevelNumber` | 委买数量 |
| `offLevelNumber` | 委卖数量 |

### 6.10 合约详情字段 (get_instrument_detail)

| 字段 | 说明 |
|-----|------|
| `InstrumentID` | 合约代码 |
| `InstrumentName` | 合约名称 |
| `ExchangeID` | 市场代码 |
| `ExchangeCode` | 交易所代码 |
| `UniCode` | 统一规则代码 |
| `PriceTick` | 最小价格变动单位 |
| `VolumeMultiple` | 合约乘数 |
| `UpStopPrice` | 涨停价 |
| `DownStopPrice` | 跌停价 |
| `PreClose` | 前收盘价 |
| `SettlementPrice` | 前结算价 (期货) |
| `FloatVolume` | 流通股本 |
| `TotalVolume` | 总股本 |
| `CreateDate` | 上市日期 (期货) |
| `OpenDate` | IPO日期 (股票) |
| `ExpireDate` | 退市日/到期日 |
| `IsTrading` | 是否可交易 |
| `MainContract` | 主力合约标记 (1/2/3) |

### 6.11 行情数据字典

#### 证券状态 (stockStatus)

| 值 | 说明 |
|---|------|
| 0, 10 | 默认为未知 |
| 11 | 开盘前 S |
| 12 | 集合竞价时段 C |
| 13 | 连续交易 T |
| 14 | 休市 B |
| 15 | 闭市 E |
| 16 | 波动性中断 V |
| 17 | 临时停牌 P |
| 18 | 收盘集合竞价 U |
| 19 | 盘中集合竞价 M |
| 20 | 暂停交易至闭市 N |
| 21 | 获取字段异常 |
| 22 | 盘后固定价格行情 |
| 23 | 盘后固定价格行情完毕 |

#### 委托类型 (entrustType / tradeType)

> 适用于: l2order 的 `entrustType`, l2transaction 的 `tradeType`

| 值 | 说明 |
|---|------|
| 0 | 未知 |
| 1 | 正常交易业务 |
| 2 | 即时成交剩余撤销 |
| 3 | ETF基金申报 |
| 4 | 最优五档即时成交剩余撤销 |
| 5 | 全额成交或撤销 |
| 6 | 本方最优价格 |
| 7 | 对手方最优价格 |

#### 委托方向 (entrustDirection)

> 适用于: l2order 的 `entrustDirection`  
> 注：上交所的撤单信息在逐笔委托的委托方向，区分撤买撤卖

| 值 | 说明 |
|---|------|
| 1 | 买入 |
| 2 | 卖出 |
| 3 | 撤买（上交所） |
| 4 | 撤卖（上交所） |

#### 成交标志 (tradeFlag)

> 适用于: l2transaction 的 `tradeFlag`  
> 注：深交所的在逐笔成交的成交标志，只有撤单，没有方向

| 值 | 说明 |
|---|------|
| 0 | 未知 |
| 1 | 外盘 |
| 2 | 内盘 |
| 3 | 撤单（深交所） |

#### 现金替代标志

> 适用于: ETF申赎清单成份股现金替代标志

| 值 | 说明 |
|---|------|
| 0 | 禁止现金替代（必须有股票） |
| 1 | 允许现金替代（先用股票，股票不足的话用现金替代） |
| 2 | 必须现金替代 |
| 3 | 非沪市（股票）退补现金替代 |
| 4 | 非沪市（股票）必须现金替代 |
| 5 | 非沪深退补现金替代 |
| 6 | 非沪深必须现金替代 |
| 7 | 港市退补现金替代（仅适用于跨沪深ETF产品） |
| 8 | 港市必须现金替代（仅适用于跨沪深港ETF产品） |

---

## 7. Nautilus Trader 适配要点

### 7.1 线程安全 ⚠️

XtQuant 回调在独立线程触发，**必须**使用 `loop.call_soon_threadsafe()`:

```python
def on_stock_order(self, order):
    # ✅ 正确: 调度到主循环
    self._loop.call_soon_threadsafe(self._handle_order, order)
    
    # ❌ 错误: 直接调用会导致线程安全问题
    # self._handle_order(order)
```

### 7.2 订单 ID 映射

```
订单生命周期:
1. order_stock() 返回 order_id (本地 ID)
2. on_order_stock_async_response 回调返回 seq -> order_id 映射
3. on_stock_order 回调返回 order_sysid (柜台 ID)

需要维护三者映射:
ClientOrderId <-> order_id <-> order_sysid
```

### 7.3 使用 order_remark 存储 ClientOrderId

```python
order_id = xt_trader.order_stock(
    ...,
    order_remark=str(client_order_id),  # 最大 24 个英文字符
)
```

### 7.4 连接管理

```python
# 连接是一次性的，断开后不会自动重连
if xt_trader.connect() != 0:
    # 需要手动重连
    pass

# 断开回调
def on_disconnected(self):
    # 在这里实现重连逻辑
    pass
```

### 7.5 回调中避免同步查询

```python
def on_stock_order(self, order):
    # ⚠️ 在回调中调用同步查询可能会卡住
    # orders = xt_trader.query_stock_orders(account)  # 不推荐
    
    # ✅ 使用异步版本或开启宽松时序
    xt_trader.set_relaxed_response_order_enabled(True)
```

### 7.6 时间戳转换

```python
# XtQuant 时间戳单位是毫秒
ts_ms = data.get("time", 0)
ts_ns = ts_ms * 1_000_000  # 转换为纳秒 (Nautilus 使用)

# 转换示例
import time
def conv_time(ct):
    """ms时间戳转可读格式"""
    local_time = time.localtime(ct / 1000)
    return time.strftime('%Y%m%d%H%M%S', local_time)
```

### 7.7 委托状态映射

| XtQuant 状态 | 值 | Nautilus OrderStatus |
|-------------|---|----------------------|
| `ORDER_UNREPORTED` | 48 | `SUBMITTED` |
| `ORDER_WAIT_REPORTING` | 49 | `SUBMITTED` |
| `ORDER_REPORTED` | 50 | `ACCEPTED` |
| `ORDER_REPORTED_CANCEL` | 51 | `PENDING_CANCEL` |
| `ORDER_PARTSUCC_CANCEL` | 52 | `PARTIALLY_FILLED` |
| `ORDER_PART_CANCEL` | 53 | `CANCELED` |
| `ORDER_CANCELED` | 54 | `CANCELED` |
| `ORDER_PART_SUCC` | 55 | `PARTIALLY_FILLED` |
| `ORDER_SUCCEEDED` | 56 | `FILLED` |
| `ORDER_JUNK` | 57 | `REJECTED` |
| `ORDER_UNKNOWN` | 255 | `DENIED` |

### 7.8 合约代码转换

| Nautilus InstrumentId | XtQuant stock_code | 说明 |
|----------------------|-------------------|------|
| `600000.SSE` | `600000.SH` | 上交所股票 |
| `000001.SZSE` | `000001.SZ` | 深交所股票 |
| `IF2401.CFFEX` | `IF2401.IF` | 中金所期货 |
| `AU2406.SHFE` | `au2406.SF` | 上期所期货 |

---

## 8. 常见问题

### Q1: 为什么订阅后收不到数据?
- 确保 MiniQmt 已启动并登录
- 检查 `userdata_mini` 路径是否正确
- 确认账户已订阅: `xt_trader.subscribe(account)`

### Q2: 市价单不生效?
- 市价单只在**实盘环境**有效，模拟环境不支持
- 模拟环境请使用 `xtconstant.FIX_PRICE` (限价)

### Q3: 如何区分上交所和深交所撤单?
- `cancel_order_stock_sysid()` 需要传入 `market` 参数
- 上交所: `xtconstant.SH_MARKET`
- 深交所: `xtconstant.SZ_MARKET`

### Q4: order_remark 最大长度?
- 最大 **24 个英文字符**

### Q5: 如何获取废单原因?
- 检查 `order.status_msg` 字段
- 或通过 `on_order_error` 回调获取 `error_msg`

### Q6: 期货如何区分开/平仓?
- 使用 `order.offset_flag` 字段
- 48=开仓, 49=平仓, 51=平今, 52=平昨

### Q7: 单股订阅数量限制?
- 建议不超过 **50 只**
- 大量订阅请使用 `subscribe_whole_quote` 全推行情

### Q8: 如何处理断线重连?
- XtQuant 不会自动重连
- 需要在 `on_disconnected()` 回调中实现重连逻辑
- 重连后需要重新订阅账号和行情

### Q9: 异步下单和同步下单的区别?
- 同步下单 (`order_stock`) 阻塞直到返回 `order_id`
- 异步下单 (`order_stock_async`) 立即返回 `seq`，通过回调获取 `order_id`
- 高频交易推荐使用异步下单

### Q10: T+1 交易限制?
- A股当日买入不能卖出 (T+1)
- `can_use_volume` 字段表示可卖数量
- 期货当日可平仓 (T+0)

---
