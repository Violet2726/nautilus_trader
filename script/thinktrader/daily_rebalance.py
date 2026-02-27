import os
import random
import time
import json
import hashlib
import requests
import warnings
import logging
import threading
import pandas as pd
from decimal import Decimal
from datetime import datetime, timedelta
from pathlib import Path

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.trading.strategy import Strategy
from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient

# 屏蔽依赖项的常见警告
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

_load_dotenv()


shutdown_event = threading.Event()

# ================= 工具函数 =================

def generate_md5(params, api_key="noesis_2025"):
    md5_params = params.copy()
    md5_params['apiKey'] = api_key
    md5_params = {k: v for k, v in md5_params.items() if not k.lower() == 'md5'}
    sorted_items = sorted(md5_params.items(), key=lambda x: x[0])
    param_string = '&'.join([f"{k}={v}" for k, v in sorted_items])
    md5_obj = hashlib.md5(param_string.encode('utf-8'))
    return md5_obj.hexdigest().upper()

def get_visual_width(s):
    """计算字符串在终端的视觉宽度（中文计2）"""
    width = 0
    for char in s:
        if ord(char) > 127: width += 2
        else: width += 1
    return width

def pad_text(text, target_width, align='left'):
    """根据视觉宽度进行填充对齐"""
    text = str(text)
    current_w = get_visual_width(text)
    padding = max(0, target_width - current_w)
    if align == 'left': return text + (' ' * padding)
    elif align == 'right': return (' ' * padding) + text
    return text  # 默认不处理居中

def to_nautilus_id(api_code: str) -> str:
    if api_code.endswith(".SZ"):
        return api_code.replace(".SZ", ".SZSE")
    elif api_code.endswith(".SH"):
        return api_code.replace(".SH", ".SSE")
    return api_code

def get_snapshot_prices(nautilus_codes, historic_client):
    """使用 HistoricThinkTraderClient 获取快照价格，并补充中文名称"""
    qmt_codes = []
    for code in nautilus_codes:
        if code.endswith('.SZSE'): qmt_codes.append(code.replace('.SZSE', '.SZ'))
        if code.endswith('.SSE'): qmt_codes.append(code.replace('.SSE', '.SH'))
    
    # 通过 adapter 订阅全推快照行情，确保 get_full_tick 能取到数据
    historic_client.subscribe_whole_quote(qmt_codes)
    ticks = historic_client.get_full_tick(qmt_codes)
    
    prices = {}
    names = {}
    for code in nautilus_codes:
        qmt = code.replace('.SZSE', '.SZ').replace('.SSE', '.SH')
        # 使用 adapter 获取名称
        detail = historic_client.get_instrument_detail(qmt)
        names[code] = detail.get('InstrumentName', '未知') if detail else "未知"
        
        # 获取价格
        if qmt in ticks and 'lastPrice' in ticks[qmt] and ticks[qmt]['lastPrice'] > 0:
            prices[code] = float(ticks[qmt]['lastPrice'])
        elif qmt in ticks and 'preClose' in ticks[qmt] and ticks[qmt]['preClose'] > 0:
            prices[code] = float(ticks[qmt]['preClose'])
        else:
            prices[code] = 0.0
    return prices, names

def fetch_target_pool(logger):
    """调用 API 获取今日推荐的目标股票池"""
    BASE_URL = "https://noesisai.cn"
    API_ENDPOINT = "/quantization-ai/api/portal/stockNamePrediction"
    
    HEADERS = {
        "Content-Type": "application/json",
        "tenant-id": "YOUR_TENANT_ID",
        "X-Access-Token": "YOUR_ACCESS_TOKEN"
    }
    
    PAYLOAD = {
        "capital": "1000000",
        "date": (datetime.now() - timedelta(days=1)).strftime("%Y%m%d"),
        "initDate": datetime.now().strftime("%Y%m%d"),
        "license": "c5ddbf5f-2917-44d4-9b66-d4ab239cbac5",
        "macAddress": "00:16:3E:41:EA:AF",
        "positionStockCodes": "",
        "securitiesAccount": "693",
        "sign": "noesis_2025"
    }
    
    PAYLOAD['md5'] = generate_md5(PAYLOAD)
    
    try:
        # 为了避免用户配置的假的占位符失败报错退出，这里做了简单的容错兜底
        response = requests.post(f"{BASE_URL}{API_ENDPOINT}", headers=HEADERS, json=PAYLOAD, timeout=30)
        response.raise_for_status()
        result = response.json()
        
        stocks = result.get('result', {}).get('nameList', [])
        logger.info(f"API请求成功，返回了 {len(stocks)} 只预测股票。")
        # 返回带元数据的列表
        target_list = []
        for item in stocks:
            target_list.append({
                'code': to_nautilus_id(item['code']), 
                'name': str(item.get('name', '未知')),
                'score': str(item.get('score', '0')) # 保留原始字符串分值
            })
        return target_list
    except Exception as e:
        logger.error(f"API 请求失败: {e}")
        return []

# ================= 调仓策略核心 =================

class DailyRebalanceStrategyConfig(StrategyConfig, frozen=True):
    target_pool: tuple[dict, ...] # 包含 code, name, score 的元组
    simulation_mode: bool = True 

class DailyRebalanceStrategy(Strategy):
    def __init__(self, config: DailyRebalanceStrategyConfig, ext_logger, historic_client) -> None:
        super().__init__(config)
        self.target_pool_data = {item['code']: item for item in config.target_pool}
        self.target_pool_codes = list(self.target_pool_data.keys())
        self.simulation_mode = config.simulation_mode
        self.ext_logger = ext_logger
        self.historic_client = historic_client

    def on_start(self) -> None:
        self.ext_logger.info("策略系统和底层节点已启动。延迟5秒等待数据同步...")
        # 显式传递 callback，确保定时器到点能触发 on_timer
        self.clock.set_time_alert('init', self.clock.utc_now() + timedelta(seconds=5), callback=self.on_timer)

    def on_timer(self, event) -> None:
        self.ext_logger.info(f"收到定时器中断事件: {event.name}")
        if event.name == 'init':
            self.execute_rebalance()
        elif event.name == 'shutdown':
            shutdown_event.set()

    def execute_rebalance(self) -> None:
        try:
            # =============== 1. 账户资产获取 (市值 + 可用) ===============
            accounts = list(self.cache.accounts())
            if not accounts:
                self.ext_logger.warning("⏰ 账户数据未就绪，重试中...")
                self.clock.set_time_alert('init', self.clock.utc_now() + timedelta(seconds=5), callback=self.on_timer)
                return
            account = accounts[0]
            
            info = account.last_event.info if hasattr(account, "last_event") else {}
            market_val = float(info.get("m_dMarketValue", 0))
            available = float(info.get("m_dAvailable", 0))
            total_asset = float(info.get("m_dTotalAsset", market_val + available))

            target_count = len(self.target_pool_codes)
            stock_budget = total_asset / target_count if target_count > 0 else 0
            
            self.ext_logger.info("┌────────────────────────────────────────────────────────────────────────┐")
            self.ext_logger.info(f"│ 账户概览: 市值 {market_val:,.2f} + 可用 {available:,.2f} = 总资产 {total_asset:,.2f} 元 │")
            self.ext_logger.info(f"│ 配置目标: {target_count} 只股票 | 单股经费预算: {stock_budget:,.2f} 元               │")
            self.ext_logger.info("└────────────────────────────────────────────────────────────────────────┘")

            # =============== 阶段一：当前持仓明细 ===============
            current_positions = self.cache.positions()
            holding_map = {} # code -> qty
            cost_map = {}    # code -> avg_px
            for pos in current_positions:
                q = float(pos.quantity)
                if q > 0:
                    code = str(pos.instrument_id)
                    holding_map[code] = q
                    cost_map[code] = float(pos.avg_px_open) if pos.avg_px_open else 0.0

            # 统一预取所有相关股票价格和名称
            all_involved = list(set(self.target_pool_codes) | set(holding_map.keys()))
            prices, names_qmt = get_snapshot_prices(all_involved, self.historic_client)

            self.ext_logger.info("\n" + "╔════════════════════════════ 【1/4 当前持仓详情明细表】 ════════════════════════════╗")
            header = f"║ {'代码':<14} | {'名称':<12} | {'持仓':>8} | {'成本':>8} | {'现价':>8} | {'市值':>12} | {'状态':<8} ║"
            self.ext_logger.info(header)
            self.ext_logger.info("╟────────────────────────────────────────────────────────────────────────────────────╢")
            for code, q in holding_map.items():
                name = names_qmt.get(code, "未知")
                p = prices.get(code, 0.0)
                mkt = q * p
                cost = cost_map.get(code, 0.0)
                status = "留在池内" if code in self.target_pool_codes else "需清理"
                
                c_code = pad_text(code, 14)
                c_name = pad_text(name, 12)
                row = f"║ {c_code} | {c_name} | {int(q):>8d} | {cost:>8.2f} | {p:>8.2f} | {mkt:>12,.2f} | {status:<8} ║"
                self.ext_logger.info(row)
            if not holding_map: self.ext_logger.info("║ (目前该账号无持仓股票)                                                           ║")
            self.ext_logger.info("╚════════════════════════════════════════════════════════════════════════════════════╝")

            # =============== 阶段二：预测股票池展现 ===============
            self.ext_logger.info("\n" + "╔════════════════════════════ 【2/4 预测股票池】 ════════════════════════════╗")
            self.ext_logger.info(f"║ {'排名':<4} | {'代码':<14} | {'名称':<12} | {'预测分/原始':<18} | {'现价':>8} | {'备注':<10} ║")
            self.ext_logger.info("╟────────────────────────────────────────────────────────────────────────────────────╢")
            for i, (code, meta) in enumerate(self.target_pool_data.items(), 1):
                p = prices.get(code, 0.0)
                name = names_qmt.get(code, meta['name'])
                note = "已有持仓" if code in holding_map else "等待建仓"
                
                c_code = pad_text(code, 14)
                c_name = pad_text(name, 12)
                c_score = pad_text(meta['score'], 18)
                row = f"║ {str(i):<4} | {c_code} | {c_name} | {c_score} | {p:>8.2f} | {note:<10} ║"
                self.ext_logger.info(row)
            self.ext_logger.info("╚════════════════════════════════════════════════════════════════════════════════════╝")
            
            sell_orders = []
            buy_orders = []
            
            # 卖出/减仓逻辑
            for code, qty in holding_map.items():
                if code not in self.target_pool_codes:
                    sell_orders.append((code, qty, prices.get(code, 0.0), "策略剔除，全额清仓"))
                else:
                    price = prices.get(code, 0.0)
                    if price <= 0: continue
                    target_qty = (stock_budget // price // 100) * 100
                    if target_qty < qty:
                        diff = qty - target_qty
                        if diff >= 100:
                            sell_orders.append((code, diff, price, "市值超限，部分减仓"))
                            
            # 买入/补仓逻辑
            for code in self.target_pool_codes:
                price = prices.get(code, 0.0)
                if price <= 0: continue
                target_qty = (stock_budget // price // 100) * 100
                current_qty = holding_map.get(code, 0.0)
                if target_qty > current_qty:
                    diff = target_qty - current_qty
                    if diff >= 100:
                        buy_orders.append((code, diff, price, "建仓买入" if current_qty == 0 else "追加补仓"))

            # =============== 阶段三：打印调仓指令明细 ===============
            self.ext_logger.info("\n" + "╔════════════════════════════ 【3/4 拟定调仓交易指令】 ════════════════════════════╗")
            if self.simulation_mode:
                self.ext_logger.warning("║ 🔸 [注意] 模拟模式激活：以下指令仅作演示，不会真实下单。                        ║")
            self.ext_logger.info(f"║ {'动作':<6} | {'代码':<14} | {'数量':>10} | {'参考价':>10} | {'理由':<18} ║")
            self.ext_logger.info("╟────────────────────────────────────────────────────────────────────────────────────╢")
            
            for code, qty, price, reason in sell_orders:
                row = f"║ SELL   | {code:<14} | {int(qty):>10d} | {price:>10.2f} | {reason:<18} ║"
                self.ext_logger.info(row)
                if not self.simulation_mode:
                    instr = InstrumentId.from_str(code)
                    instrument = self.cache.instrument(instr)
                    if instrument: self.submit_order(self.order_factory.market(instr, OrderSide.SELL, instrument.make_qty(qty)))
            
            for code, qty, price, reason in buy_orders:
                row = f"║ BUY    | {code:<14} | {int(qty):>10d} | {price:>10.2f} | {reason:<18} ║"
                self.ext_logger.info(row)
                if not self.simulation_mode:
                    instr = InstrumentId.from_str(code)
                    instrument = self.cache.instrument(instr)
                    if instrument: self.submit_order(self.order_factory.market(instr, OrderSide.BUY, instrument.make_qty(qty)))

            if not sell_orders and not buy_orders:
                self.ext_logger.info("║ 💨 组合已达目标配置，今日无需任何操作。                                          ║")
            self.ext_logger.info("╚════════════════════════════════════════════════════════════════════════════════════╝")

            # =============== 阶段四：预估调整后终态 ===============
            final_map = holding_map.copy()
            for c, q, _, _ in sell_orders: final_map[c] = final_map.get(c, 0) - q
            for c, q, _, _ in buy_orders: final_map[c] = final_map.get(c, 0) + q
            final_map = {k: v for k, v in final_map.items() if v > 0}
            
            self.ext_logger.info("\n" + "╔═════════════════════════ 【4/4 调仓后终态持仓(预估)】 ═════════════════════════╗")
            self.ext_logger.info(f"║ {'排名':<4} | {'代码':<14} | {'名称':<12} | {'预测分/原始':<18} | {'预计持仓':>12} ║")
            self.ext_logger.info("╟────────────────────────────────────────────────────────────────────────────────╢")
            for i, (code, q) in enumerate(final_map.items(), 1):
                name = names_qmt.get(code, "未知")
                score = self.target_pool_data.get(code, {}).get('score', '--')
                c_code = pad_text(code, 14)
                c_name = pad_text(name, 12)
                c_score = pad_text(score, 18)
                row = f"║ {str(i):<4} | {c_code} | {c_name} | {c_score} | {int(q):>12d} ║"
                self.ext_logger.info(row)
            self.ext_logger.info("╚════════════════════════════════════════════════════════════════════════════════╝")

            self.ext_logger.info("\n✨ 换仓任务全流程分析完毕。")
            self.clock.set_time_alert('shutdown', self.clock.utc_now() + timedelta(seconds=15), callback=self.on_timer)
            
        except Exception as e:
             print(f"💥 调仓运行故障: {e}", flush=True)
             self.ext_logger.error(f"调仓故障详细信息: {e}", exc_info=True)
             self.clock.set_time_alert('shutdown', self.clock.utc_now() + timedelta(seconds=5), callback=self.on_timer)


# ================= 程序入口 =================

def main():
    # 1. 设置格式化日志配置
    script_dir = Path(__file__).resolve().parent
    output_dir = script_dir / "output"
    output_dir.mkdir(parents=True, exist_ok=True)
    date_str = datetime.now().strftime("%Y-%m-%d")
    log_path = output_dir / f"{date_str}.log"

    logger = logging.getLogger("DailyRebalance")
    logger.setLevel(logging.INFO)
    fmt = logging.Formatter('%(asctime)s - %(levelname)s - %(message)s')

    fh = logging.FileHandler(log_path, encoding='utf-8')
    fh.setFormatter(fmt)
    logger.addHandler(fh)

    ch = logging.StreamHandler()
    ch.setFormatter(fmt)
    logger.addHandler(ch)

    logger.info("="*60)
    logger.info("=== 自动化换仓脚本 (ThinkTrader Adapter) 主流程开启 ===")

    # 2. 调用API获取目标股票
    target_pool_metadata = fetch_target_pool(logger)
    if not target_pool_metadata:
        logger.warning("并未获取到目标股票池数据。如果您遇到 401 权限问题，请在脚本内填入真实的 TOKEN。脚本已安全中止。")
        return

    nautilus_target_pool = tuple(target_pool_metadata)

    # 3. MiniQMT 与节点配置
    miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
    account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
    account_type = "STOCK"
    session_id = random.randint(100000, 999999)

    logger.info("正在初始化 HistoricThinkTraderClient 提供快照支持...")
    historic_client = HistoricThinkTraderClient(miniqmt_path=miniqmt_path, log_level="INFO")

    logger.info("正在初始化 Nautilus 交易节点。尝试加载全市场合约数据(可能需要数秒钟)...")

    # load_all=True 为懒人模式，全加载保证策略里的当前持仓也能成功拿到合约；
    instrument_provider = ThinkTraderInstrumentProviderConfig(
        load_all=True
    )

    config_node = TradingNodeConfig(
        trader_id=TraderId("DAILY-REBAL-NODE"),
        logging=LoggingConfig(log_level="INFO"),  # 调回 INFO，确保看到报表
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
                account_type=account_type,
                session_id=session_id,
                instrument_provider=instrument_provider,
                routing=RoutingConfig(default=True),
            ),
        }
    )

    node = TradingNode(config=config_node)
    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)

    # 业务决定配置：是否仅模拟打印不发单 
    # (设为 False，策略就会向真实账号的柜台发送真实验证和买卖委托)
    ENABLE_SIMULATION_MODE = True
    if ENABLE_SIMULATION_MODE:
        logger.warning("当前以 模拟状态 (simulation_mode) 启动，仅生成并打印目标调仓清单，绝不会有真实委托发送！")

    # 挂载调仓策略
    strategy_config = DailyRebalanceStrategyConfig(
        target_pool=nautilus_target_pool,
        simulation_mode=ENABLE_SIMULATION_MODE
    )
    strategy = DailyRebalanceStrategy(config=strategy_config, ext_logger=logger, historic_client=historic_client)
    node.trader.add_strategy(strategy)

    # 构建并启动底层
    logger.info("正在连接到交易柜台，准备开始运行...")
    node.build()

    # 启动后台线程监听停止信号，并调用 node.stop() 来解除 node.run() 的阻塞
    def shutdown_waiter():
        shutdown_event.wait()
        logger.info("接收到策略停止信号，正在请求节点优雅退出...")
        node.stop()

    shutdown_thread = threading.Thread(target=shutdown_waiter, daemon=True)
    shutdown_thread.start()

    try:
        node.run() 
    except KeyboardInterrupt:
        logger.info("用户主动中断 (Ctrl+C)")
    finally:
        logger.info("清理并销毁交易节点连接...")
        node.dispose()
        logger.info("交易节点已成功关闭，任务退出。")
        logger.info("="*60)


if __name__ == "__main__":
    main()
