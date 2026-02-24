#!/usr/bin/env python3
"""
查询当日所有委托与成交记录示例
================================

本脚本连接 MiniQmt 后查询当日全部委托记录 (含已成、部成、已撤、废单等各种状态) 和
全部成交记录，以格式化表格输出。

输出内容
--------
1. 【当日委托】 — 所有委托状态 (已成、部成、已撤、废单/已拒、待报等)
2. 【当日成交】 — 所有成交明细 (成交价、成交量、成交时间)

环境变量
--------
- MINIQMT_PATH : MiniQmt userdata_mini 路径 (必填)
- MINIQMT_ACCOUNT_ID : 资金账号 (必填)
- MINIQMT_ACCOUNT_TYPE : 账号类型, 默认 STOCK
"""
from __future__ import annotations

import os
import secrets
import warnings
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
# XtQuant 订单状态码 → 中文描述
# ---------------------------------------------------------------------------
ORDER_STATUS_LABELS = {
    48: "未报",
    49: "待报",
    50: "已报",
    51: "已报待撤",
    52: "部成待撤",
    53: "部撤",
    54: "已撤",
    55: "部成",
    56: "已成",
    57: "废单",
    255: "未知",
}

# XtQuant order_type 码 → 中文描述
ORDER_TYPE_LABELS = {
    23: "买入",
    24: "卖出",
}


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


class QueryAllOrdersConfig(StrategyConfig, frozen=True):
    account_id: str = ""
    client_id: str = TT
    query_delay_secs: float = 3.0   # 启动后等待多久再查询 (等待连接稳定)
    auto_stop_secs: float = 10.0    # 查询后多久自动停止 (0=不自动停止)


class QueryAllOrdersStrategy(Strategy):
    """查询当日全部委托与成交记录的策略"""

    def __init__(self, config: QueryAllOrdersConfig) -> None:
        super().__init__(config)
        self._client_id = ClientId(config.client_id)
        self._account_id_raw = config.account_id
        self._query_delay_secs = config.query_delay_secs
        self._auto_stop_secs = config.auto_stop_secs

    def on_start(self) -> None:
        if not self._account_id_raw:
            self.log.error("未配置 MINIQMT_ACCOUNT_ID，无法查询订单")
            self.stop()
            return

        self.log.info(f"策略已启动，将在 {self._query_delay_secs}s 后查询当日全部委托与成交")

        # 设置延迟查询，等待连接与协调完成
        self.clock.set_time_alert(
            name="query_all_orders",
            alert_time=self.clock.utc_now() + timedelta(seconds=self._query_delay_secs),
            callback=lambda _: self._query_and_dump(),
        )

    def on_stop(self) -> None:
        return

    # ------------------------------------------------------------------
    # 核心查询逻辑
    # ------------------------------------------------------------------
    def _query_and_dump(self) -> None:
        """查询并输出当日全部委托与成交"""
        from nautilus_trader.adapters.thinktrader.factories import THINKTRADER_CLIENTS

        if not THINKTRADER_CLIENTS:
            self.log.error("未找到已缓存的 ThinkTraderClient 实例")
            return

        # 取第一个 (通常只有一个)
        client = next(iter(THINKTRADER_CLIENTS.values()))
        self.log.info(f"获取到 ThinkTraderClient: account={client._account_id}")

        self._dump_orders(client)
        self._dump_trades(client)

        # 自动停止
        if self._auto_stop_secs > 0:
            self.clock.set_time_alert(
                name="auto_stop",
                alert_time=self.clock.utc_now() + timedelta(seconds=self._auto_stop_secs),
                callback=lambda _: self.stop(),
            )

    # ------------------------------------------------------------------
    # 格式化工具
    # ------------------------------------------------------------------
    @staticmethod
    def _display_width(s: str) -> int:
        """计算字符串的终端显示宽度 (CJK 字符占 2 列)"""
        import unicodedata
        w = 0
        for ch in s:
            eaw = unicodedata.east_asian_width(ch)
            w += 2 if eaw in ("W", "F") else 1
        return w

    @staticmethod
    def _pad(s: str, width: int, align: str = "<") -> str:
        """按终端显示宽度填充字符串 (支持 CJK)"""
        import unicodedata
        display_w = 0
        for ch in s:
            eaw = unicodedata.east_asian_width(ch)
            display_w += 2 if eaw in ("W", "F") else 1
        pad = max(0, width - display_w)
        if align == ">":
            return " " * pad + s
        elif align == "^":
            left = pad // 2
            return " " * left + s + " " * (pad - left)
        else:
            return s + " " * pad

    @staticmethod
    def _fmt_time(raw_time) -> str:
        """将 xtquant 时间戳转为 HH:MM:SS 格式"""
        from datetime import datetime as dt
        try:
            ts = int(raw_time)
            if ts <= 0:
                return ""
            # xtquant 时间戳是秒级 UNIX 时间 (取决于 SDK 版本)
            if ts > 1e15:       # 纳秒
                ts = ts / 1e9
            elif ts > 1e12:     # 毫秒
                ts = ts / 1e3
            return dt.fromtimestamp(ts).strftime("%H:%M:%S")
        except (ValueError, TypeError, OSError):
            return str(raw_time) if raw_time else ""

    def _fmt_table(self, headers: list[tuple[str, int, str]], rows: list[list[str]]) -> str:
        """
        格式化表格字符串 (CJK 对齐)

        Parameters
        ----------
        headers : list[(列名, 宽度, 对齐方式)]
            对齐方式: "<" 左对齐, ">" 右对齐, "^" 居中
        rows : list[list[str]]
            每行数据 (已转为字符串)
        """
        lines = []
        # 表头
        header_line = " | ".join(self._pad(h, w, "^") for h, w, _ in headers)
        lines.append(header_line)
        lines.append("-+-".join("-" * w for _, w, _ in headers))
        # 数据行
        for row in rows:
            parts = []
            for (_, w, align), val in zip(headers, row):
                parts.append(self._pad(val, w, align))
            lines.append(" | ".join(parts))
        return "\n".join(lines)

    # ------------------------------------------------------------------
    # 查询并格式化输出
    # ------------------------------------------------------------------
    def _dump_orders(self, client) -> None:
        """查询并输出当日全部委托"""
        orders = client.query_orders()
        if not orders:
            print("\n" + "=" * 100)
            print("【当日委托】 无委托记录")
            print("=" * 100)
            return

        # 分类统计
        status_counter: dict[str, int] = {}
        rows = []
        for o in orders:
            order_status = getattr(o, "order_status", None)
            status_label = ORDER_STATUS_LABELS.get(order_status, f"未知({order_status})")
            status_counter[status_label] = status_counter.get(status_label, 0) + 1

            order_type = getattr(o, "order_type", None)
            type_label = ORDER_TYPE_LABELS.get(order_type, f"({order_type})")

            price = float(getattr(o, "price", 0.0) or 0.0)
            order_vol = int(getattr(o, "order_volume", 0) or 0)
            traded_price = float(getattr(o, "traded_price", 0.0) or 0.0)
            traded_vol = int(getattr(o, "traded_volume", 0) or 0)
            order_time = getattr(o, "order_time", "")

            rows.append([
                str(getattr(o, "order_id", "")),
                str(getattr(o, "stock_code", "")),
                type_label,
                f"{price:.2f}" if price > 0 else "-",
                f"{order_vol:,}",
                f"{traded_price:.4f}" if traded_vol > 0 else "-",
                f"{traded_vol:,}" if traded_vol > 0 else "-",
                status_label,
                self._fmt_time(order_time),
            ])

        headers = [
            ("委托编号",   10, "<"),
            ("证券代码",   12, "<"),
            ("方向",       4,  "^"),
            ("委托价",     8,  ">"),
            ("委托量",     8,  ">"),
            ("成交均价",   10, ">"),
            ("成交量",     8,  ">"),
            ("状态",       8,  "^"),
            ("委托时间",   8,  "^"),
        ]

        summary_parts = [f"{s}: {c}" for s, c in status_counter.items()]

        print("\n" + "=" * 100)
        print(f"【当日委托】 共 {len(rows)} 条")
        print(f"状态统计: {' | '.join(summary_parts)}")
        print("-" * 100)
        print(self._fmt_table(headers, rows))
        print("=" * 100)

    def _dump_trades(self, client) -> None:
        """查询并输出当日全部成交"""
        trades = client.query_trades()
        if not trades:
            print("\n" + "=" * 100)
            print("【当日成交】 无成交记录")
            print("=" * 100)
            return

        rows = []
        total_amount = 0.0
        total_commission = 0.0

        for t in trades:
            price = float(getattr(t, "traded_price", 0.0) or 0.0)
            vol = int(getattr(t, "traded_volume", 0) or 0)
            amount = float(getattr(t, "traded_amount", 0.0) or 0.0)
            commission = float(getattr(t, "commission", 0.0) or 0.0)
            traded_time = getattr(t, "traded_time", "")

            total_amount += amount
            total_commission += commission

            rows.append([
                str(getattr(t, "traded_id", "")),
                str(getattr(t, "order_id", "")),
                str(getattr(t, "stock_code", "")),
                f"{price:.4f}",
                f"{vol:,}",
                f"{amount:,.2f}",
                f"{commission:.2f}",
                self._fmt_time(traded_time),
            ])

        headers = [
            ("成交编号",     18, "<"),
            ("委托编号",     10, "<"),
            ("证券代码",     12, "<"),
            ("成交价格",     10, ">"),
            ("成交数量",     10, ">"),
            ("成交金额",     14, ">"),
            ("手续费",       8,  ">"),
            ("成交时间",     8,  "^"),
        ]

        print("\n" + "=" * 100)
        print(f"【当日成交】 共 {len(rows)} 笔")
        print(f"成交总额: {total_amount:,.2f} 元 | 手续费合计: {total_commission:,.2f} 元")
        print("-" * 100)
        print(self._fmt_table(headers, rows))
        print("=" * 100)


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

instrument_provider = ThinkTraderInstrumentProviderConfig(load_contracts_on_start=True)

config_node = TradingNodeConfig(
    trader_id=TraderId("TT-ORDER-QUERY"),
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

    strategy = QueryAllOrdersStrategy(
        config=QueryAllOrdersConfig(
            account_id=account_id_raw,
            query_delay_secs=3.0,
            auto_stop_secs=10.0,
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
