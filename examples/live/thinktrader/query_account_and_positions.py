#!/usr/bin/env python3
from __future__ import annotations

import os
import secrets
import warnings
from datetime import timedelta
from pathlib import Path
import asyncio
import pandas as pd

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
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.trading.strategy import Strategy


warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)


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


class QueryAccountAndPositionsConfig(StrategyConfig, frozen=True):
    account_id: str = ""
    client_id: str = TT
    stop_after_secs: float = 15.0
    poll_interval_secs: float = 3.0
    max_polls: int = 100


class QueryAccountAndPositionsStrategy(Strategy):
    def __init__(self, config: QueryAccountAndPositionsConfig) -> None:
        super().__init__(config)
        self._client_id = ClientId(config.client_id)
        self._account_id = AccountId(f"{TT}-{config.account_id}") if config.account_id else None
        self._stop_after_secs = config.stop_after_secs
        self._poll_interval_secs = config.poll_interval_secs
        self._max_polls = config.max_polls
        self._polls_done = 0
        # self._dumped = False  # Allow repeated dumping

    def on_start(self) -> None:
        if self._account_id is None:
            self.log.error("未配置 MINIQMT_ACCOUNT_ID，无法查询账户信息")
            self.stop()
            return

        self.log.info(f"启动账户与持仓查询示例，account_id={self._account_id}")
        
        self.clock.set_time_alert(
            name="poll_account_positions",
            alert_time=self.clock.utc_now(),
            callback=lambda _: self._poll(),
        )
        self.clock.set_time_alert(
            name="stop_after_dump",
            alert_time=self.clock.utc_now() + timedelta(seconds=self._stop_after_secs),
            callback=lambda _: self._dump_and_stop(),
        )

    def on_stop(self) -> None:
        return

    def _poll(self) -> None:
        if self._account_id is None:
            return

        self._polls_done += 1
        self.query_account(account_id=self._account_id, client_id=self._client_id)

        # Force dump every poll
        self._dump_account_and_positions()

        if self._polls_done < self._max_polls:
            self.clock.set_time_alert(
                name=f"poll_account_positions_{self._polls_done}",
                alert_time=self.clock.utc_now()
                + timedelta(seconds=float(self._poll_interval_secs)),
                callback=lambda _: self._poll(),
            )

    def _dump_and_stop(self) -> None:
        self.stop()

    def _dump_account_and_positions(self) -> None:
        if self._account_id is None:
            return

        print("\n" + "=" * 50)
        print(f"查询时间: {self.clock.utc_now()}")
        
        account = self.cache.account(self._account_id)
        if account is None:
            self.log.warning(f"缓存中暂未找到账户对象: {self._account_id}")
        else:
            print(f"账户ID: {account.id}")
            balances = account.balances()
            if balances:
                data = []
                for currency, balance in balances.items():
                    data.append({
                        "Currency": currency.code,
                        "Total": float(balance.total),
                        "Free": float(balance.free),
                        "Locked": float(balance.locked),
                    })
                df_bal = pd.DataFrame(data)
                print("【账户资金】")
                print(df_bal.to_string(index=False))

        positions = self.cache.positions(account_id=self._account_id)
        if not positions:
            print("当前账户无任何持仓")
        else:
            print(f"【账户持仓】 共 {len(positions)} 条")
            pos_data = []
            for i, pos in enumerate(positions):
                if i == 0:
                    # Debug: print details to understand what keys are available
                    if hasattr(pos, "details"):
                        print(f"DEBUG: First position details: {pos.details}")
                    else:
                        print("DEBUG: Position has no 'details' attribute")

                try:
                    unrealized_pnl = 0.0
                    market_val = 0.0
                    
                    # Safer extraction
                    # Check if 'details' exists
                    if hasattr(pos, "details") and pos.details:
                        # Common QMT keys might be: 'mkt_val', 'float_pnl', 'market_value', 'floating_pnl'
                        # We try a few or just rely on what we see in debug output
                        raw_float_pnl = pos.details.get("float_pnl") or pos.details.get("floating_pnl")
                        raw_mkt_val = pos.details.get("mkt_val") or pos.details.get("market_value")
                        
                        try:
                            if raw_float_pnl is not None:
                                unrealized_pnl = float(raw_float_pnl)
                            if raw_mkt_val is not None:
                                market_val = float(raw_mkt_val)
                        except (ValueError, TypeError) as e:
                            print(f"Error converting details: {e}, float_pnl={raw_float_pnl}")

                    p = {
                        "Instrument": str(pos.instrument_id),
                        "Side": pos.side,
                        "Qty": float(pos.quantity),
                        "AvgOpen": float(pos.avg_px_open) if pos.avg_px_open else 0.0,
                        "RealizedPnL": float(pos.realized_pnl) if pos.realized_pnl else 0.0,
                        "UnrealizedPnL": unrealized_pnl,
                        "MarketValue": market_val,
                    }
                    pos_data.append(p)
                except Exception as e:
                    print(f"Error processing position {pos}: {e}")
            
            if pos_data:
                df_pos = pd.DataFrame(pos_data)
                print(df_pos.to_string(index=False))
        print("=" * 50 + "\n")


    def on_position_status_report(self, report) -> None:
        self.log.info(f"收到持仓报告: {report.instrument_id} {report.quantity} {report.side}")

    def on_account_status_report(self, report) -> None:
        self.log.info(f"收到账户报告: {report.account_id}")

_load_dotenv()

miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
print(f"MiniQMT Path: {miniqmt_path}")
account_id_raw = os.environ.get("MINIQMT_ACCOUNT_ID", "")
if not account_id_raw:
    print("WARNING: MINIQMT_ACCOUNT_ID not set in environment!")
else:
    print(f"Target Account: {account_id_raw}")
account_type = os.environ.get("MINIQMT_ACCOUNT_TYPE", "STOCK")

session_id_env = os.environ.get("MINIQMT_SESSION_ID")
session_id = int(session_id_env) if session_id_env else (secrets.randbelow(900_000) + 100_000)
print(f"Session ID: {session_id}")

# 加载合约配置：必须开启 load_contracts_on_start=True，否则 Nautilus 不知道标的 ID，会忽略持仓回报
instrument_provider = ThinkTraderInstrumentProviderConfig(load_contracts_on_start=True)

config_node = TradingNodeConfig(
    trader_id=TraderId("TT-ACCOUNT-QUERY"),
    logging=LoggingConfig(log_level="INFO"),
    data_engine=LiveDataEngineConfig(
        validate_data_sequence=True,
    ),
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
    timeout_disconnection=10.0,
    timeout_post_stop=5.0,
)


if __name__ == "__main__":
    node = TradingNode(config=config_node)

    strategy = QueryAccountAndPositionsStrategy(
        config=QueryAccountAndPositionsConfig(account_id=account_id_raw),
    )
    node.trader.add_strategy(strategy)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
    node.build()

    async def _stop_node_later() -> None:
        stop_after = float(os.environ.get("TT_STOP_AFTER_SECS", "20"))
        await asyncio.sleep(stop_after)
        node.stop()

    node.kernel.loop.create_task(_stop_node_later())

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("Stopped by user")
    finally:
        node.dispose()
