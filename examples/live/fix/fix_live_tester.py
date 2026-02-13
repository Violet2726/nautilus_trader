from __future__ import annotations

import http.client
import json
import os
import re
import socket
import sys
import warnings
from datetime import timedelta
from pathlib import Path
from types import SimpleNamespace
from urllib.parse import urlparse


warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def _nt():
    repo_root = _repo_root()
    sys.path.insert(0, str(repo_root))

    try:
        import nautilus_trader  # noqa: F401
    except Exception as exc:
        print("无法导入 nautilus_trader.")
        print("请先在仓库根目录构建扩展模块:")
        print("  uv run --active --no-sync build.py")
        print("然后再运行:")
        print("  uv run --active --no-sync python examples/live/fix/fix_live_tester.py")
        raise SystemExit(1) from exc

    from nautilus_trader.adapters.fix.config import FixExecClientConfig
    from nautilus_trader.adapters.fix.constants import FIX
    from nautilus_trader.adapters.fix.constants import FIX_CLIENT_ID
    from nautilus_trader.adapters.fix.factories import FixLiveExecClientFactory
    from nautilus_trader.config import LiveExecEngineConfig
    from nautilus_trader.config import LiveRiskEngineConfig
    from nautilus_trader.config import LoggingConfig
    from nautilus_trader.config import RoutingConfig
    from nautilus_trader.config import StrategyConfig
    from nautilus_trader.config import TradingNodeConfig
    from nautilus_trader.live.node import TradingNode
    from nautilus_trader.model.enums import OrderSide
    from nautilus_trader.model.identifiers import AccountId
    from nautilus_trader.model.identifiers import InstrumentId
    from nautilus_trader.model.identifiers import TraderId
    from nautilus_trader.model.objects import Price
    from nautilus_trader.model.objects import Quantity
    from nautilus_trader.trading.strategy import Strategy

    return SimpleNamespace(
        AccountId=AccountId,
        FixExecClientConfig=FixExecClientConfig,
        FixLiveExecClientFactory=FixLiveExecClientFactory,
        FIX=FIX,
        FIX_CLIENT_ID=FIX_CLIENT_ID,
        InstrumentId=InstrumentId,
        LiveExecEngineConfig=LiveExecEngineConfig,
        LiveRiskEngineConfig=LiveRiskEngineConfig,
        LoggingConfig=LoggingConfig,
        OrderSide=OrderSide,
        Price=Price,
        Quantity=Quantity,
        RoutingConfig=RoutingConfig,
        Strategy=Strategy,
        StrategyConfig=StrategyConfig,
        TraderId=TraderId,
        TradingNode=TradingNode,
        TradingNodeConfig=TradingNodeConfig,
    )


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


def _get_public_ip(timeout_secs: float = 3.0) -> str:
    sources = (
        "https://icanhazip.com/",
        "https://checkip.amazonaws.com/",
        "https://ipv4.icanhazip.com/",
    )
    ipv4_re = re.compile(r"(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)")
    for source in sources:
        parsed = urlparse(source)
        if parsed.scheme != "https" or not parsed.netloc:
            continue

        conn = http.client.HTTPSConnection(parsed.netloc, timeout=timeout_secs)
        try:
            path = parsed.path or "/"
            if parsed.query:
                path = f"{path}?{parsed.query}"
            conn.request("GET", path)
            response = conn.getresponse()
            if response.status != 200:
                continue
            text = response.read().decode("utf-8", errors="ignore").strip()
        except Exception:
            text = ""
        finally:
            conn.close()

        match = ipv4_re.search(text)
        if match:
            return match.group(0)
    return "Unknown"


def _get_local_ip() -> str:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sock.connect(("8.8.8.8", 80))
        ip = sock.getsockname()[0]
        return ip
    finally:
        sock.close()


def _get_env_bool(name: str, default: bool) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    value = value.strip().lower()
    return value in {"1", "true", "t", "yes", "y", "on"}


nt = _nt()

AccountId = nt.AccountId
FIX = nt.FIX
FIX_CLIENT_ID = nt.FIX_CLIENT_ID
InstrumentId = nt.InstrumentId
OrderSide = nt.OrderSide
Price = nt.Price
Quantity = nt.Quantity
Strategy = nt.Strategy
StrategyConfig = nt.StrategyConfig


class FixLiveTesterConfig(StrategyConfig, frozen=True):
    account_id: str = ""
    instrument_id: str = "000002.SZ"
    limit_price: str = "4.95"
    order_qty: int = 100
    send_test_order: bool = False
    stop_after_secs: float = 0.0


class FixLiveTester(Strategy):
    def __init__(self, config: FixLiveTesterConfig) -> None:
        super().__init__(config)
        self._client_id = FIX_CLIENT_ID
        self._account_id = AccountId(f"{FIX}-{config.account_id}") if config.account_id else None
        self._instrument_id = InstrumentId.from_str(config.instrument_id)
        self._limit_price = Price.from_str(config.limit_price)
        self._order_qty = Quantity.from_int(config.order_qty)
        self._send_test_order = config.send_test_order
        self._stop_after_secs = float(config.stop_after_secs)
        self._order = None

    def on_start(self) -> None:
        if self._account_id is None:
            self.log.error("未配置 FIX_ACCOUNT_ID, 无法启动示例")
            self.stop()
            return

        public_ip = _get_public_ip()
        local_ip = _get_local_ip()
        expected_public_ip = os.environ.get("FIX_BOUND_PUBLIC_IP", "124.160.32.18")

        print("\n" + "=" * 64)
        print("FIX Live Tester")
        print(f"Public IP: {public_ip}")
        print(f"Local  IP: {local_ip}")
        print(f"Expected Public IP (FIX_BOUND_PUBLIC_IP): {expected_public_ip}")
        if expected_public_ip and public_ip not in {"Unknown", expected_public_ip}:
            print("WARNING: public IP 与预期不一致, 可能导致柜台拒绝连接")
        print("=" * 64 + "\n")

        # 启动后延迟 5 秒再查询和打印, 给与 FIX 连接和对账足够时间
        self.clock.set_time_alert(
            name="query_account_after_start",
            alert_time=self.clock.utc_now() + timedelta(seconds=5),
            callback=lambda _: self._query_and_dump(),
        )

        if self._send_test_order:
            self.clock.set_time_alert(
                name="submit_test_order",
                alert_time=self.clock.utc_now() + timedelta(seconds=3),
                callback=lambda _: self._submit_limit_buy(),
            )
            self.clock.set_time_alert(
                name="cancel_test_order",
                alert_time=self.clock.utc_now() + timedelta(seconds=10),
                callback=lambda _: self._cancel_if_open(),
            )
            self.clock.set_time_alert(
                name="dump_after_order",
                alert_time=self.clock.utc_now() + timedelta(seconds=12),
                callback=lambda _: self._query_and_dump(),
            )

        if self._stop_after_secs > 0:
            self.clock.set_time_alert(
                name="stop_after",
                alert_time=self.clock.utc_now() + timedelta(seconds=self._stop_after_secs),
                callback=lambda _: self.stop(),
            )

    def on_stop(self) -> None:
        return

    def _query_and_dump(self) -> None:
        if self._account_id is None:
            return

        self.query_account(account_id=self._account_id, client_id=self._client_id)

        # 延迟 2 秒再打印, 等待 FIX 响应写入缓存
        self.clock.set_time_alert(
            name=f"dump_state_{self.clock.timestamp_ns()}",
            alert_time=self.clock.utc_now() + timedelta(seconds=2),
            callback=lambda _: self._dump_cache_state(),
        )

    def _dump_cache_state(self) -> None:
        if self._account_id is None:
            return

        account = self.cache.account(self._account_id)
        positions = self.cache.positions(account_id=self._account_id)
        open_orders = self.cache.orders_open(account_id=self._account_id)
        closed_orders = self.cache.orders_closed(account_id=self._account_id)

        # 构造类似 Demo 的 JSON 输出结构
        account_data = {}
        if account:
            for curr, bal in account.balances().items():
                account_data[curr.code] = {
                    "balance": float(bal.total),
                    "available": float(bal.free),
                    "locked": float(bal.locked),
                }

        position_data = []
        for pos in positions:
            position_data.append(
                {
                    "instrument_id": str(pos.instrument_id),
                    "side": str(pos.side),
                    "volume": float(pos.quantity),
                    "avg_price": float(pos.avg_px_open) if pos.avg_px_open else 0.0,
                }
            )

        order_data = []
        # 合并未完成和最近 5 笔已完成的订单
        all_relevant_orders = open_orders + closed_orders[-5:]
        for order in all_relevant_orders:
            order_data.append(
                {
                    "client_order_id": str(order.client_order_id),
                    "instrument_id": str(order.instrument_id),
                    "side": str(order.side),
                    "qty": float(order.quantity),
                    "filled": float(order.filled_qty),
                    "status": str(order.status),
                }
            )

        print("\n" + "=" * 20 + f" 状态快照 ({self.clock.utc_now()}) " + "=" * 20)
        print(f"account:{json.dumps(account_data, indent=4, ensure_ascii=False)}")
        print(f"position:{json.dumps(position_data, indent=4, ensure_ascii=False)}")
        print(f"order:{json.dumps(order_data, indent=4, ensure_ascii=False)}")
        print("=" * 64 + "\n")

    def _submit_limit_buy(self) -> None:
        if self._account_id is None:
            return

        order = self.order_factory.limit(
            instrument_id=self._instrument_id,
            order_side=OrderSide.BUY,
            quantity=self._order_qty,
            price=self._limit_price,
        )
        self._order = order
        self.submit_order(order=order, client_id=self._client_id)
        self.log.info(f"已提交测试限价单: {order.client_order_id} {order.instrument_id}")

    def _cancel_if_open(self) -> None:
        if self._order is None:
            return
        if not self.cache.is_order_open(self._order.client_order_id):
            self.log.info(f"订单不处于 OPEN, 跳过撤单: {self._order.client_order_id}")
            return
        self.cancel_order(order=self._order, client_id=self._client_id)
        self.log.info(f"已提交撤单: {self._order.client_order_id}")

    def on_order_status_report(self, report) -> None:
        self.log.info(
            f"订单状态报告: {report.client_order_id} status={report.order_status} "
            f"filled={report.filled_qty} leaves={report.leaves_qty}",
        )

    def on_position_status_report(self, report) -> None:
        self.log.info(f"持仓状态报告: {report.instrument_id} {report.side} qty={report.quantity}")

    def on_account_status_report(self, report) -> None:
        self.log.info(f"账户状态报告: {report.account_id}")


_load_dotenv()


def _default_fix_demo_paths() -> tuple[str, str]:
    base_dir = Path(__file__).parent.resolve()
    settings = base_dir / "wind_fix_config.cfg"
    dictionary = base_dir / "FIX44.xml"
    return str(settings), str(dictionary)


def _resolve_path(path: str) -> str:
    p = Path(path)
    if not p.is_absolute():
        p = _repo_root() / p
    return str(p.resolve())


def _read_comp_ids(settings_path: str) -> tuple[str, str]:
    sender = ""
    target = ""
    try:
        with open(settings_path, encoding="utf-8", errors="ignore") as f:
            for raw_line in f:
                line = raw_line.strip()
                if not line or line.startswith(("#", ";", "[")):
                    continue
                if line.startswith("SenderCompID="):
                    sender = line.split("=", 1)[1].strip()
                elif line.startswith("TargetCompID="):
                    target = line.split("=", 1)[1].strip()
    except Exception:
        return "", ""
    return sender, target


def _normalize_trade_account_id(fix_settings_path: str, account_id: str) -> str:
    if not account_id:
        return ""

    sender_comp_id, _ = _read_comp_ids(fix_settings_path)
    if (
        sender_comp_id
        and account_id == sender_comp_id
        and not os.environ.get("FIX_TRADE_ACCOUNT_ID")
    ):
        print("检测到 FIX_ACCOUNT_ID 与 wind_fix_config.cfg 中 SenderCompID 相同。")
        print("该值通常是登录/会话标识, 而不是资金账号(例如 690)。")
        print(
            '建议设置: $env:FIX_TRADE_ACCOUNT_ID="690"  (并保留 cfg 中 SenderCompID/TargetCompID)'
        )
        return "690"

    return account_id


def _print_missing_env_and_exit(missing: list[str]) -> None:
    print(f"缺少环境变量: {', '.join(missing)}")
    print("示例:")
    print('$env:FIX_USERNAME="..."')
    print('$env:FIX_PASSWORD="..."')
    print('$env:FIX_TRADE_ACCOUNT_ID="690"')


def _abspath_if_relative(base_dir: Path, value: str) -> tuple[str, bool]:
    p = Path(value)
    if p.is_absolute():
        return value, False
    return str((base_dir / value).resolve()), True


def _patch_quickfix_cfg_line(
    config_dir: Path, raw: str, patch_keys: set[str]
) -> tuple[str, str | None, str | None, bool]:
    line = raw.strip()
    if not line or line.startswith(("#", ";", "[")) or "=" not in line:
        return raw, None, None, False

    key, value = line.split("=", 1)
    if key not in patch_keys:
        return raw, None, None, False

    value, changed = _abspath_if_relative(config_dir, value.strip())
    newline = "\n" if raw.endswith("\n") else ""
    store_path = value if key == "FileStorePath" else None
    log_path = value if key == "FileLogPath" else None
    return f"{key}={value}{newline}", store_path, log_path, changed


def _patch_quickfix_paths(settings_path: str) -> None:
    config_dir = Path(settings_path).parent.resolve()
    patch_keys = {"FileStorePath", "FileLogPath", "DataDictionary"}
    patched_store_path: str | None = None
    patched_log_path: str | None = None

    try:
        raw_lines = (
            Path(settings_path)
            .read_text(encoding="utf-8", errors="ignore")
            .splitlines(keepends=True)
        )
    except Exception:
        return

    changed_any = False
    new_lines: list[str] = []
    for raw in raw_lines:
        patched, store_path, log_path, changed = _patch_quickfix_cfg_line(
            config_dir, raw, patch_keys
        )
        new_lines.append(patched)
        if store_path:
            patched_store_path = store_path
        if log_path:
            patched_log_path = log_path
        if changed:
            changed_any = True

    if patched_store_path:
        store_p = Path(patched_store_path)
        store_p.mkdir(parents=True, exist_ok=True)
        # 清理旧的 QuickFIX 会话文件，防止 "Could not open body file" 错误
        for f in store_p.iterdir():
            if f.is_file():
                f.unlink(missing_ok=True)
    if patched_log_path:
        Path(patched_log_path).mkdir(parents=True, exist_ok=True)

    if not changed_any:
        return

    try:
        Path(settings_path).write_text("".join(new_lines), encoding="utf-8")
    except Exception:
        return


def _build_node():
    default_settings_path, default_dictionary_path = _default_fix_demo_paths()
    fix_settings_path = _resolve_path(os.environ.get("FIX_SETTINGS_PATH", default_settings_path))
    fix_dictionary_path = _resolve_path(
        os.environ.get("FIX_DICTIONARY_PATH", default_dictionary_path)
    )

    username = os.environ.get("FIX_USERNAME", "")
    password = os.environ.get("FIX_PASSWORD", "")
    account_id = os.environ.get("FIX_TRADE_ACCOUNT_ID", "") or os.environ.get("FIX_ACCOUNT_ID", "")
    account_id = _normalize_trade_account_id(fix_settings_path, account_id)

    use_tls_tunnel = _get_env_bool("FIX_USE_TLS_TUNNEL", True)
    remote_host = os.environ.get("FIX_REMOTE_HOST", "114.80.213.49")
    remote_port = int(os.environ.get("FIX_REMOTE_PORT", "16669"))
    tls_local_host = os.environ.get("FIX_TLS_LOCAL_HOST", "127.0.0.1")
    tls_local_port = int(os.environ.get("FIX_TLS_LOCAL_PORT", "16670"))
    reconciliation = _get_env_bool("FIX_RECONCILIATION", False)

    missing = []
    if not username:
        missing.append("FIX_USERNAME")
    if not password:
        missing.append("FIX_PASSWORD")
    if not account_id:
        missing.append("FIX_TRADE_ACCOUNT_ID (或 FIX_ACCOUNT_ID)")
    if missing:
        _print_missing_env_and_exit(missing)
        return None

    # 切换工作目录到配置文件所在目录，确保 QuickFIX 能正确解析相对路径
    config_dir = Path(fix_settings_path).parent.resolve()
    (config_dir / "store").mkdir(exist_ok=True)
    (config_dir / "log").mkdir(exist_ok=True)
    # 清理旧的 QuickFIX 会话文件，防止 "Could not open body file" 错误
    for f in (config_dir / "store").iterdir():
        try:
            if f.is_file():
                f.unlink()
        except OSError:
            pass
    os.chdir(config_dir)

    config_node = nt.TradingNodeConfig(
        trader_id=nt.TraderId("FIX-LIVE-TESTER-001"),
        logging=nt.LoggingConfig(
            log_level="INFO",
            log_colors=True,
            use_pyo3=True,
        ),
        risk_engine=nt.LiveRiskEngineConfig(bypass=True),
        exec_engine=nt.LiveExecEngineConfig(
            reconciliation=reconciliation,
            reconciliation_lookback_mins=1440,
            open_check_interval_secs=5.0,
            open_check_open_only=False,
            position_check_interval_secs=30.0,
            reconciliation_startup_delay_secs=3.0 if reconciliation else 0.0,
        ),
        exec_clients={
            nt.FIX: nt.FixExecClientConfig(
                fix_settings_path=fix_settings_path,
                fix_dictionary_path=fix_dictionary_path,
                username=username,
                password=password,
                account_id=account_id,
                remote_host=remote_host if use_tls_tunnel else None,
                remote_port=remote_port if use_tls_tunnel else None,
                use_tls_tunnel=use_tls_tunnel,
                tls_local_host=tls_local_host,
                tls_local_port=tls_local_port,
                routing=nt.RoutingConfig(default=True),
            ),
        },
    )

    node = nt.TradingNode(config=config_node)
    node.add_exec_client_factory(nt.FIX, nt.FixLiveExecClientFactory)
    return node, account_id


if __name__ == "__main__":
    result = _build_node()
    if result is None:
        raise SystemExit(2)
    node, account_id = result

    strategy = FixLiveTester(
        config=FixLiveTesterConfig(
            account_id=account_id,
            instrument_id=os.environ.get("FIX_INSTRUMENT_ID", "000002.SZ"),
            limit_price=os.environ.get("FIX_LIMIT_PRICE", "4.95"),
            order_qty=int(os.environ.get("FIX_ORDER_QTY", "100")),
            send_test_order=_get_env_bool("FIX_SEND_TEST_ORDER", False),
            stop_after_secs=float(os.environ.get("FIX_STOP_AFTER_SECS", "0")),
        ),
    )

    node.trader.add_strategy(strategy)
    node.build()
    node.run()
