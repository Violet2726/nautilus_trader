from __future__ import annotations

import http.client
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

# 自动切换到脚本所在目录，确保配置文件的相对路径有效
os.chdir(Path(__file__).parent.resolve())


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def _nt():
    # 先尝试直接导入已安装的 nautilus_trader (Cython 版本)
    # 仅当直接导入失败时，才尝试从 python/ 源码目录导入 (PyO3 版本)
    try:
        import nautilus_trader  # noqa: F401
    except ImportError:
        python_src = _repo_root() / "python"
        if python_src.is_dir():
            sys.path.insert(0, str(python_src))

    try:
        import nautilus_trader  # noqa: F401, F811
    except Exception as exc:
        print("无法导入 nautilus_trader.")
        print("推荐从仓库根目录运行: uv run --active --no-sync python examples/live/fix/fix_live_tester.py")
        print("或者使用: .\\.venv\\Scripts\\python.exe examples\\live\\fix\\fix_live_tester.py")
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

        self.clock.set_time_alert(
            name="query_account_on_start",
            alert_time=self.clock.utc_now(),
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
        self._dump_cache_state()

    def _dump_cache_state(self) -> None:
        if self._account_id is None:
            return

        account = self.cache.account(self._account_id)
        positions = self.cache.positions(account_id=self._account_id)
        open_orders = self.cache.orders_open(account_id=self._account_id)
        closed_orders = self.cache.orders_closed(account_id=self._account_id)

        print("\n" + "-" * 64)
        print(f"时间: {self.clock.utc_now()}")
        print(f"AccountId: {self._account_id}")
        if account is None:
            print("账户: 缓存中暂未找到")
        else:
            balances = account.balances()
            print(f"账户余额条目数: {len(balances) if balances else 0}")
            if balances:
                for currency, balance in balances.items():
                    print(
                        f"{currency.code}: total={float(balance.total)} "
                        f"free={float(balance.free)} locked={float(balance.locked)}",
                    )

        print(f"持仓条目数: {len(positions)}")
        for pos in positions:
            print(
                f"{pos.instrument_id} {pos.side} qty={float(pos.quantity)} "
                f"avg_open={float(pos.avg_px_open) if pos.avg_px_open else 0.0}",
            )

        print(f"未完成订单: {len(open_orders)}")
        for order in open_orders:
            print(
                f"{order.client_order_id} {order.instrument_id} {order.side} "
                f"qty={float(order.quantity)} filled={float(order.filled_qty)} status={order.status}",
            )

        print(f"已关闭订单: {len(closed_orders)}")
        for order in closed_orders[-10:]:
            print(
                f"{order.client_order_id} {order.instrument_id} {order.side} "
                f"qty={float(order.quantity)} filled={float(order.filled_qty)} status={order.status}",
            )
        print("-" * 64 + "\n")

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


def _build_node():
    default_settings_path, default_dictionary_path = _default_fix_demo_paths()

    fix_settings_path = os.environ.get("FIX_SETTINGS_PATH", default_settings_path)
    fix_dictionary_path = os.environ.get("FIX_DICTIONARY_PATH", default_dictionary_path)

    username = os.environ.get("FIX_USERNAME", "")
    password = os.environ.get("FIX_PASSWORD", "")
    account_id = os.environ.get("FIX_ACCOUNT_ID", "")

    use_tls_tunnel = _get_env_bool("FIX_USE_TLS_TUNNEL", True)
    remote_host = os.environ.get("FIX_REMOTE_HOST", "114.80.213.49")
    remote_port = int(os.environ.get("FIX_REMOTE_PORT", "16669"))
    tls_local_host = os.environ.get("FIX_TLS_LOCAL_HOST", "127.0.0.1")
    tls_local_port = int(os.environ.get("FIX_TLS_LOCAL_PORT", "16670"))

    missing = []
    if not username:
        missing.append("FIX_USERNAME")
    if not password:
        missing.append("FIX_PASSWORD")
    if not account_id:
        missing.append("FIX_ACCOUNT_ID")
    if missing:
        print(f"缺少环境变量: {', '.join(missing)}")
        print("示例:")
        print('$env:FIX_USERNAME="..."')
        print('$env:FIX_PASSWORD="..."')
        print('$env:FIX_ACCOUNT_ID="..."')
        return None

    config_node = nt.TradingNodeConfig(
        trader_id=nt.TraderId("FIX-LIVE-TESTER-001"),
        logging=nt.LoggingConfig(
            log_level="INFO",
            log_colors=True,
            use_pyo3=True,
        ),
        risk_engine=nt.LiveRiskEngineConfig(bypass=True),
        exec_engine=nt.LiveExecEngineConfig(
            reconciliation=True,
            reconciliation_lookback_mins=1440,
            open_check_interval_secs=5.0,
            open_check_open_only=False,
            position_check_interval_secs=30.0,
            reconciliation_startup_delay_secs=3.0,
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
    return node


if __name__ == "__main__":
    node = _build_node()
    if node is None:
        raise SystemExit(2)

    strategy = FixLiveTester(
        config=FixLiveTesterConfig(
            account_id=os.environ.get("FIX_ACCOUNT_ID", ""),
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
