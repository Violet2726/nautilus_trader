import logging
import os
import random
import sys
import warnings
from datetime import timedelta
from pathlib import Path

# 抑制烦人的警告信息
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
        raise SystemExit(1) from exc

    return
_nt()

from nautilus_trader.adapters.fix.config import FixExecClientConfig
from nautilus_trader.adapters.fix.constants import FIX
from nautilus_trader.adapters.fix.constants import FIX_CLIENT_ID
from nautilus_trader.adapters.fix.factories import FixLiveExecClientFactory
from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LiveExecEngineConfig
from nautilus_trader.config import LiveRiskEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import OrderSide, OrderStatus, PositionSide
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.objects import Price
from nautilus_trader.trading.strategy import Strategy

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

def _get_env_bool(name: str, default: bool) -> bool:
    value = os.environ.get(name)
    if value is None:
        return default
    value = value.strip().lower()
    return value in {"1", "true", "t", "yes", "y", "on"}

class LiveThinkTraderFixConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    account_id: str
    order_qty: int = 100

class LiveThinkTraderFixStrategy(Strategy):
    """
    使用 ThinkTrader 获取行情, 使用 FIX 柜台交易。
    跟踪指定股票, 启动10秒后发出买入请求, 之后10秒后发出卖出请求, 1分钟后终止策略。
    """
    def __init__(self, config: LiveThinkTraderFixConfig):
        super().__init__(config)
        self.instrument_id = config.instrument_id
        self._account_id = AccountId(f"{FIX}-{config.account_id}") if config.account_id else None
        self._client_id = FIX_CLIENT_ID
        self._order_qty = config.order_qty
        
        self._has_bought = False
        self._has_sold = False
        self._last_quote: QuoteTick | None = None

    def on_start(self):
        """策略启动"""
        if self._account_id is None:
            self.log.error("未配置 FIX_ACCOUNT_ID, 策略无法启动")
            self.stop()
            return
            
        self.subscribe_quote_ticks(self.instrument_id)
        self.log.info(f"已订阅 {self.instrument_id} 行情")
        
        # 10秒后买入
        self.clock.set_time_alert(
            name="buy_alert",
            alert_time=self.clock.utc_now() + timedelta(seconds=10),
            callback=self._on_buy_alert
        )
        
        # 每30秒查询一次持仓和订单情况
        self.clock.set_time_alert(
            name="query_status_alert",
            alert_time=self.clock.utc_now() + timedelta(seconds=30),
            callback=self._on_query_status_alert
        )
        
        # 60秒后停止
        self.clock.set_time_alert(
            name="stop_alert",
            alert_time=self.clock.utc_now() + timedelta(seconds=60),
            callback=lambda _: self.stop()
        )

    def on_stop(self):
        self.unsubscribe_quote_ticks(self.instrument_id)
        self.log.info("策略已停止")

    def _on_query_status_alert(self, event):
        positions = self.cache.positions_open()
        if self._account_id:
            positions = [p for p in positions if p.account_id == self._account_id]
        
        pos_msg = f"\n{'='*50}\n[当前持仓情况] (共 {len(positions)} 个)\n{'-'*50}\n"
        if not positions:
            pos_msg += "  (无持仓)\n"
        for p in positions:
            side_str = "多头" if p.side == PositionSide.LONG else "空头" if p.side == PositionSide.SHORT else "平仓"
            pos_msg += f"  - 合约: {p.instrument_id} | 方向: {side_str} | 数量: {p.quantity} | 均价: {p.avg_px_open:.2f}\n"
        pos_msg += f"{'='*50}"
        self.log.info(pos_msg)

        orders = self.cache.orders()
        if self._account_id:
            orders = [o for o in orders if o.account_id == self._account_id]
        
        status_map = {
            OrderStatus.INITIALIZED: "初始化",
            OrderStatus.SUBMITTED: "已提交",
            OrderStatus.ACCEPTED: "已接受",
            OrderStatus.PARTIALLY_FILLED: "部分成交",
            OrderStatus.FILLED: "完全成交",
            OrderStatus.CANCELED: "已撤销",
            OrderStatus.REJECTED: "已拒绝",
            OrderStatus.PENDING_CANCEL: "待撤销",
            OrderStatus.PENDING_UPDATE: "待更新",
            OrderStatus.EXPIRED: "已过期",
        }
        
        ord_msg = f"\n{'='*50}\n[当前所有订单情况] (共 {len(orders)} 个)\n{'-'*50}\n"
        if not orders:
            ord_msg += "  (无订单)\n"
        for o in orders:
            side_str = "买入" if getattr(o, "side", getattr(o, "order_side", None)) == OrderSide.BUY else "卖出"
            status_str = status_map.get(o.status, str(o.status).split('.')[-1])
            ord_msg += f"  - 订单号: {o.client_order_id} | 方向: {side_str} | 状态: {status_str} | 数量: {o.quantity} | 价格: {o.price}\n"
        ord_msg += f"{'='*50}"
        self.log.info(ord_msg)

        self.clock.set_time_alert(
            name=f"query_status_alert_{self.clock.timestamp_ns()}",
            alert_time=self.clock.utc_now() + timedelta(seconds=30),
            callback=self._on_query_status_alert
        )

    def on_quote_tick(self, tick: QuoteTick):
        self._last_quote = tick
        self.log.info(
            f"[实时行情] {tick.instrument_id} | "
            f"买价: {tick.bid_price.as_double():.2f} (量: {tick.bid_size}) | "
            f"卖价: {tick.ask_price.as_double():.2f} (量: {tick.ask_size})"
        )

    def _on_buy_alert(self, event):
        if self._last_quote is None or self._last_quote.ask_price.as_double() == 0:
            self.log.warning("买入时间已到, 但尚未收到有效行情报价, 延迟1秒...")
            self.clock.set_time_alert(
                name=f"buy_alert_retry_{self.clock.timestamp_ns()}",
                alert_time=self.clock.utc_now() + timedelta(seconds=1),
                callback=self._on_buy_alert
            )
            return

        instrument = self.cache.instrument(self.instrument_id)
        if instrument is None:
            self.log.warning("缓存中尚未包含合约信息 (可能还在加载中), 延迟1秒...")
            self.clock.set_time_alert(
                name=f"buy_alert_retry_inst_{self.clock.timestamp_ns()}",
                alert_time=self.clock.utc_now() + timedelta(seconds=1),
                callback=self._on_buy_alert
            )
            return

        price = Price.from_str(f"{self._last_quote.ask_price.as_double():.2f}")
        qty = instrument.make_qty(self._order_qty)

        # 查询并打印可用资金
        account = self.cache.account(self._account_id)
        if account is not None:
            balances = account.balances()
            if balances:
                self.log.info(f"当前账户余额情况: {balances}")
            else:
                self.log.info("当前账户余额情况: [由于尚未同步或无持仓为0]")
        else:
            self.log.info("当前账户对象尚未在缓存中建立...")

        order = self.order_factory.limit(
            instrument_id=self.instrument_id,
            order_side=OrderSide.BUY,
            quantity=qty,
            price=price,
        )
        self.submit_order(order, client_id=self._client_id)
        self._has_bought = True
        self.log.info(f"已发出买入单: {order.client_order_id}, 价格: {price}, 数量: {qty}")
        
    def _on_sell_alert(self, event):
        if self._last_quote is None or self._last_quote.bid_price.as_double() == 0:
            self.log.warning("卖出时间已到, 但尚未收到有效行情报价, 延迟1秒...")
            self.clock.set_time_alert(
                name=f"sell_alert_retry_{self.clock.timestamp_ns()}",
                alert_time=self.clock.utc_now() + timedelta(seconds=1),
                callback=self._on_sell_alert
            )
            return

        instrument = self.cache.instrument(self.instrument_id)
        if instrument is None:
            self.log.warning("缓存中尚未包含合约信息 (可能还在加载中), 延迟1秒...")
            self.clock.set_time_alert(
                name=f"sell_alert_retry_inst_{self.clock.timestamp_ns()}",
                alert_time=self.clock.utc_now() + timedelta(seconds=1),
                callback=self._on_sell_alert
            )
            return

        price = Price.from_str(f"{self._last_quote.bid_price.as_double():.2f}")
        qty = instrument.make_qty(self._order_qty)

        order = self.order_factory.limit(
            instrument_id=self.instrument_id,
            order_side=OrderSide.SELL,
            quantity=qty,
            price=price,
        )
        self.submit_order(order, client_id=self._client_id)
        self._has_sold = True
        self.log.info(f"已发出卖出单: {order.client_order_id}, 价格: {price}, 数量: {qty}")

    def on_order_filled(self, event):
        self.log.info(f"订单已成交: {event}")
        if event.order_side == OrderSide.BUY:
            self.log.info("买入已成交, 将在10秒后执行卖出...")
            self.clock.set_time_alert(
                name="sell_alert",
                alert_time=self.clock.utc_now() + timedelta(seconds=10),
                callback=self._on_sell_alert
            )

    def on_order_rejected(self, event):
        self.log.error(f"订单被柜台拒绝: {event}")

    def on_order_canceled(self, event):
        self.log.warning(f"订单已撤销: {event}")

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
        return "690"
    return account_id

def main():
    _load_dotenv()
    
    # ---------------------------------------------------------
    # 行情配置 (ThinkTrader)
    # ---------------------------------------------------------
    miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
    session_id = random.randint(100000, 999999)
    ticker_str = os.environ.get("TARGET_INSTRUMENT", "000001.SZSE")
    instrument_id = InstrumentId.from_str(ticker_str)

    instrument_provider = ThinkTraderInstrumentProviderConfig(
        load_all=False,
        load_ids=frozenset([instrument_id]),
    )

    # ---------------------------------------------------------
    # 交易配置 (FIX)
    # ---------------------------------------------------------
    default_settings_path, default_dictionary_path = _default_fix_demo_paths()
    fix_settings_path = _resolve_path(os.environ.get("FIX_SETTINGS_PATH", default_settings_path))
    fix_dictionary_path = _resolve_path(os.environ.get("FIX_DICTIONARY_PATH", default_dictionary_path))

    username = os.environ.get("FIX_USERNAME", "")
    password = os.environ.get("FIX_PASSWORD", "")
    account_id = os.environ.get("FIX_TRADE_ACCOUNT_ID", "") or os.environ.get("FIX_ACCOUNT_ID", "")
    account_id = _normalize_trade_account_id(fix_settings_path, account_id)

    use_tls_tunnel = _get_env_bool("FIX_USE_TLS_TUNNEL", True)
    remote_host = os.environ.get("FIX_REMOTE_HOST", "114.80.213.49")
    remote_port = int(os.environ.get("FIX_REMOTE_PORT", "16669"))
    tls_local_host = os.environ.get("FIX_TLS_LOCAL_HOST", "127.0.0.1")
    tls_local_port = int(os.environ.get("FIX_TLS_LOCAL_PORT", "16670"))

    if not username or not password or not account_id:
        print("未检测到有效的 FIX 连接环境变量, 将不可连接至柜台。")
        print("设置 .env 包含 FIX_USERNAME, FIX_PASSWORD, FIX_TRADE_ACCOUNT_ID")
        sys.exit(1)

    config_dir = Path(fix_settings_path).parent.resolve()
    (config_dir / "store").mkdir(exist_ok=True)
    (config_dir / "log").mkdir(exist_ok=True)
    # 切换工作目录到配置文件所在目录，确保 QuickFIX 能正确解析相对路径
    os.chdir(config_dir)

    config_node = TradingNodeConfig(
        trader_id=TraderId("MIXED-LIVE-TESTER-001"),
        logging=LoggingConfig(
            log_level="INFO",
            log_colors=True,
            use_pyo3=True,
        ),
        risk_engine=LiveRiskEngineConfig(bypass=True),
        exec_engine=LiveExecEngineConfig(
            reconciliation=True,
            reconciliation_lookback_mins=1440,
            open_check_interval_secs=5.0,
        ),
        data_clients={
            TT: ThinkTraderDataClientConfig(
                miniqmt_path=miniqmt_path,
                session_id=session_id,
                instrument_provider=instrument_provider,
            ),
        },
        exec_clients={
            FIX: FixExecClientConfig(
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
                routing=RoutingConfig(default=True),
            ),
        },
        data_engine=LiveDataEngineConfig(validate_data_sequence=False),
        timeout_connection=90.0,
    )

    node = TradingNode(config=config_node)
    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(FIX, FixLiveExecClientFactory)

    strategy_config = LiveThinkTraderFixConfig(
        instrument_id=instrument_id,
        account_id=account_id,
        order_qty=100
    )
    strategy = LiveThinkTraderFixStrategy(config=strategy_config)
    
    node.trader.add_strategy(strategy)
    node.build()
    
    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户中断")
    finally:
        node.dispose()

if __name__ == "__main__":
    main()
