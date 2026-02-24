#!/usr/bin/env python3
"""
批量撤销过期未成交委托示例
============================

本脚本连接 MiniQmt 后，查询当日所有委托记录，筛选出超过指定时间
（默认 1 小时）仍未成交的挂单，逐笔执行撤单操作。

撤单范围
--------
- 订单状态为 "可撤" 的委托：已报(50)、部成(55)
- 委托时间早于 `当前时间 - cancel_older_than_mins`

环境变量
--------
- MINIQMT_PATH        : MiniQmt userdata_mini 路径 (必填)
- MINIQMT_ACCOUNT_ID  : 资金账号 (必填)
- MINIQMT_ACCOUNT_TYPE: 账号类型, 默认 STOCK
"""
from __future__ import annotations

import os
import secrets
import time
import unicodedata
import warnings
from datetime import datetime
from datetime import timedelta
from pathlib import Path

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LiveExecEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.trading.strategy import Strategy


warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)


# ---------------------------------------------------------------------------
# 常量
# ---------------------------------------------------------------------------
# 可撤销的订单状态码
CANCELLABLE_STATUSES = {
    50,   # 已报
    55,   # 部成
}

ORDER_STATUS_LABELS = {
    48: "未报", 49: "待报", 50: "已报", 51: "已报待撤", 52: "部成待撤",
    53: "部撤", 54: "已撤", 55: "部成", 56: "已成", 57: "废单", 255: "未知",
}

ORDER_TYPE_LABELS = {23: "买入", 24: "卖出"}


def _load_dotenv() -> None:
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return
    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return


# ---------------------------------------------------------------------------
# CJK 对齐工具
# ---------------------------------------------------------------------------
def _pad(s: str, width: int, align: str = "<") -> str:
    """按终端显示宽度填充 (CJK 字符占 2 列)"""
    display_w = sum(2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1 for ch in s)
    pad = max(0, width - display_w)
    if align == ">":
        return " " * pad + s
    elif align == "^":
        left = pad // 2
        return " " * left + s + " " * (pad - left)
    return s + " " * pad


def _fmt_time(raw_time) -> str:
    """Xtquant 时间戳 → HH:MM:SS"""
    try:
        ts = int(raw_time)
        if ts <= 0:
            return ""
        if ts > 1e15:
            ts = ts / 1e9
        elif ts > 1e12:
            ts = ts / 1e3
        return datetime.fromtimestamp(ts).strftime("%H:%M:%S")
    except (ValueError, TypeError, OSError):
        return str(raw_time) if raw_time else ""


def _ts_to_epoch(raw_time) -> float:
    """Xtquant 时间戳 → Unix epoch seconds"""
    try:
        ts = float(raw_time)
        if ts <= 0:
            return 0.0
        if ts > 1e15:
            return ts / 1e9
        elif ts > 1e12:
            return ts / 1e3
        return ts
    except (ValueError, TypeError):
        return 0.0


def _fmt_table(headers: list[tuple[str, int, str]], rows: list[list[str]]) -> str:
    """格式化 CJK 对齐表格"""
    lines = []
    lines.append(" | ".join(_pad(h, w, "^") for h, w, _ in headers))
    lines.append("-+-".join("-" * w for _, w, _ in headers))
    for row in rows:
        parts = [_pad(val, w, align) for (_, w, align), val in zip(headers, row)]
        lines.append(" | ".join(parts))
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# 策略
# ---------------------------------------------------------------------------
class CancelStaleOrdersConfig(StrategyConfig, frozen=True):
    account_id: str = ""
    client_id: str = TT
    cancel_older_than_mins: float = 60.0   # 撤销 N 分钟前的未成交委托
    query_delay_secs: float = 3.0          # 启动后延迟查询
    auto_stop_secs: float = 15.0           # 撤单完成后自动停止


class CancelStaleOrdersStrategy(Strategy):
    """批量撤销过期未成交委托"""

    def __init__(self, config: CancelStaleOrdersConfig) -> None:
        super().__init__(config)
        self._client_id = ClientId(config.client_id)
        self._account_id_raw = config.account_id
        self._cancel_older_than_mins = config.cancel_older_than_mins
        self._query_delay_secs = config.query_delay_secs
        self._auto_stop_secs = config.auto_stop_secs

    def on_start(self) -> None:
        if not self._account_id_raw:
            self.log.error("未配置 MINIQMT_ACCOUNT_ID")
            self.stop()
            return

        self.log.info(
            f"策略已启动，将在 {self._query_delay_secs}s 后扫描并撤销 "
            f"{self._cancel_older_than_mins} 分钟前的未成交委托"
        )
        self.clock.set_time_alert(
            name="scan_and_cancel",
            alert_time=self.clock.utc_now() + timedelta(seconds=self._query_delay_secs),
            callback=lambda _: self._scan_and_cancel(),
        )

    def on_stop(self) -> None:
        return

    def _scan_and_cancel(self) -> None:
        from nautilus_trader.adapters.thinktrader.factories import THINKTRADER_CLIENTS

        if not THINKTRADER_CLIENTS:
            self.log.error("未找到已缓存的 ThinkTraderClient 实例")
            return

        client = next(iter(THINKTRADER_CLIENTS.values()))
        self.log.info(f"获取到 ThinkTraderClient: account={client._account_id}")

        orders = client.query_orders()
        if not orders:
            print("\n" + "=" * 90)
            print("当日无委托记录，无需撤单")
            print("=" * 90)
            self._schedule_stop()
            return

        now_epoch = time.time()
        cutoff_epoch = now_epoch - self._cancel_older_than_mins * 60
        cutoff_str = datetime.fromtimestamp(cutoff_epoch).strftime("%H:%M:%S")

        # ------ 筛选可撤且超时的委托 ------
        stale_orders = []
        skipped_rows = []

        for o in orders:
            status_code = getattr(o, "order_status", None)
            if status_code not in CANCELLABLE_STATUSES:
                continue

            order_time_raw = getattr(o, "order_time", 0)
            order_epoch = _ts_to_epoch(order_time_raw)

            if order_epoch <= 0:
                continue

            if order_epoch > cutoff_epoch:
                # 未超时，记录到跳过列表
                skipped_rows.append(o)
                continue

            stale_orders.append(o)

        # ------ 打印摘要 ------
        status_label = ORDER_STATUS_LABELS
        type_label = ORDER_TYPE_LABELS

        print("\n" + "=" * 90)
        print(f"扫描条件: 委托时间 < {cutoff_str} (即 {self._cancel_older_than_mins:.0f} 分钟前)")
        print(f"当日委托总数: {len(orders)} | 可撤未超时: {len(skipped_rows)} | 需撤销: {len(stale_orders)}")
        print("-" * 90)

        if not stale_orders:
            print("✓ 无需撤销的过期委托")
            print("=" * 90)
            self._schedule_stop()
            return

        # ------ 展示待撤委托 ------
        headers = [
            ("委托编号",  10, "<"),
            ("证券代码",  12, "<"),
            ("方向",      4,  "^"),
            ("委托价",    8,  ">"),
            ("委托量",    8,  ">"),
            ("已成量",    8,  ">"),
            ("状态",      6,  "^"),
            ("委托时间",  8,  "^"),
        ]

        rows = []
        for o in stale_orders:
            sc = getattr(o, "order_status", None)
            ot = getattr(o, "order_type", None)
            price = float(getattr(o, "price", 0.0) or 0.0)
            order_vol = int(getattr(o, "order_volume", 0) or 0)
            traded_vol = int(getattr(o, "traded_volume", 0) or 0)

            rows.append([
                str(getattr(o, "order_id", "")),
                str(getattr(o, "stock_code", "")),
                type_label.get(ot, f"({ot})"),
                f"{price:.2f}" if price > 0 else "-",
                f"{order_vol:,}",
                f"{traded_vol:,}" if traded_vol > 0 else "-",
                status_label.get(sc, "?"),
                _fmt_time(getattr(o, "order_time", "")),
            ])

        print(_fmt_table(headers, rows))
        print("-" * 90)

        # ------ 逐笔撤单 ------
        success_count = 0
        fail_count = 0

        for o in stale_orders:
            order_id = getattr(o, "order_id", None)
            stock_code = getattr(o, "stock_code", "")
            if not order_id:
                continue

            oid = int(order_id)
            try:
                result = client.cancel_order_async(oid)
                if result > 0:
                    print(f"  ✓ 撤单已发送: {stock_code} (order_id={oid}, seq={result})")
                    success_count += 1
                else:
                    print(f"  ✗ 撤单失败: {stock_code} (order_id={oid}, result={result})")
                    fail_count += 1
            except Exception as e:
                print(f"  ✗ 撤单异常: {stock_code} (order_id={oid}): {e}")
                fail_count += 1

        print("-" * 90)
        print(f"撤单完成: 成功 {success_count} 笔, 失败 {fail_count} 笔")
        print("=" * 90)

        self._schedule_stop()

    def _schedule_stop(self) -> None:
        if self._auto_stop_secs > 0:
            self.clock.set_time_alert(
                name="auto_stop",
                alert_time=self.clock.utc_now() + timedelta(seconds=self._auto_stop_secs),
                callback=lambda _: self.stop(),
            )


# ===========================================================================
# 脚本入口
# ===========================================================================
_load_dotenv()

miniqmt_path = os.environ.get(
    "MINIQMT_PATH",
    r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini",
)
account_id_raw = os.environ.get("MINIQMT_ACCOUNT_ID", "")
account_type = os.environ.get("MINIQMT_ACCOUNT_TYPE", "STOCK")
session_id_env = os.environ.get("MINIQMT_SESSION_ID")
session_id = int(session_id_env) if session_id_env else (secrets.randbelow(900_000) + 100_000)

if not account_id_raw:
    print("ERROR: 请设置环境变量 MINIQMT_ACCOUNT_ID (资金账号)")
    exit(1)

print(f"MiniQMT Path : {miniqmt_path}")
print(f"Account ID   : {account_id_raw}")
print(f"Account Type : {account_type}")
print(f"Session ID   : {session_id}")
print()

instrument_provider = ThinkTraderInstrumentProviderConfig(load_contracts_on_start=True)

config_node = TradingNodeConfig(
    trader_id=TraderId("TT-CANCEL-STALE"),
    logging=LoggingConfig(log_level="INFO"),
    data_engine=LiveDataEngineConfig(validate_data_sequence=True),
    exec_engine=LiveExecEngineConfig(
        reconciliation=True,
        reconciliation_lookback_mins=1440,
    ),
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
            account_id=account_id_raw,
            account_type=account_type,
            session_id=session_id,
            instrument_provider=instrument_provider,
            routing=RoutingConfig(default=True),
        ),
    },
    timeout_connection=90.0,
    timeout_reconciliation=10.0,
    timeout_portfolio=10.0,
    timeout_disconnection=5.0,
    timeout_post_stop=2.0,
)


if __name__ == "__main__":
    node = TradingNode(config=config_node)

    strategy = CancelStaleOrdersStrategy(
        config=CancelStaleOrdersConfig(
            account_id=account_id_raw,
            cancel_older_than_mins=60.0,      # 撤销 1 小时前的未成交委托
            query_delay_secs=3.0,
            auto_stop_secs=15.0,
        ),
    )
    node.trader.add_strategy(strategy)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
    node.build()

    try:
        node.run()
    except KeyboardInterrupt:
        print("\n[INFO] 用户停止运行 (Ctrl+C)")
    finally:
        print("[INFO] 正在释放资源...")
        node.dispose()
        print("[INFO] 已完成")
        os._exit(0)
