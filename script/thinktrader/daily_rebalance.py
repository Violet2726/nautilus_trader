import os
import random
import time
import hashlib
import requests
import warnings
import logging
import threading
import pandas as pd
from datetime import datetime, timedelta
from pathlib import Path

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import (
    ThinkTraderDataClientConfig,
    ThinkTraderExecClientConfig,
    ThinkTraderInstrumentProviderConfig,
)
from nautilus_trader.adapters.thinktrader.factories import (
    ThinkTraderLiveDataClientFactory,
    ThinkTraderLiveExecClientFactory,
)
from nautilus_trader.config import (
    LoggingConfig,
    RoutingConfig,
    StrategyConfig,
    TradingNodeConfig,
)
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.identifiers import InstrumentId, TraderId
from nautilus_trader.trading.strategy import Strategy
from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient

# --- 全局配置与告警屏蔽 ---
warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)

def _load_dotenv() -> None:
    """加载 .env 环境变量文件"""
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

# ================= 工具类与辅助函数 =================

class TableFormatter:
    """针对中英文混排终端对齐优化的表格工具"""
    
    @staticmethod
    def get_visual_width(s: str) -> int:
        """计算字符串在终端的视觉宽度（中文计2，英文计1）"""
        return sum(2 if ord(char) > 127 else 1 for char in str(s))

    @classmethod
    def pad_text(cls, text: str, target_width: int, align: str = 'left') -> str:
        """根据视觉宽度填充对齐"""
        text = str(text)
        padding = max(0, target_width - cls.get_visual_width(text))
        if align == 'left': return text + (' ' * padding)
        elif align == 'right': return (' ' * padding) + text
        return text

    @classmethod
    def format_df(cls, df: pd.DataFrame) -> str:
        """将 DataFrame 转换为带有 ASCII 边框且对齐完美的字符串"""
        if df.empty: return "(空数据集)"
        
        cols = list(df.columns)
        str_data = df.values.astype(str).tolist()
        
        # 计算每一列所需的最大视觉宽度
        widths = []
        for i, col in enumerate(cols):
            max_w = max([cls.get_visual_width(row[i]) for row in str_data] + [cls.get_visual_width(col)])
            widths.append(max_w + 2) # 两边各留一个空格
            
        sep = "+" + "+".join(["-" * w for w in widths]) + "+"
        lines = [sep]
        
        # 表头行
        header = "|" + "|".join([cls.pad_text(f" {cols[i]} ", widths[i], 'left') for i in range(len(cols))]) + "|"
        lines.extend([header, sep])
        
        # 数据行
        for row in str_data:
            row_str = "|"
            for i, val in enumerate(row):
                # 简单启发式：包含数字且有点的通常是金额/价格，采用右对齐
                align = 'right' if ('.' in val and any(c.isdigit() for c in val)) else 'left'
                row_str += cls.pad_text(f" {val} ", widths[i], align) + "|"
            lines.append(row_str)
            
        lines.append(sep)
        return "\n".join(lines)

def generate_md5(params: dict, api_key: str = "noesis_2025") -> str:
    """生成 API 请求所需的 MD5 签名"""
    md5_params = params.copy()
    md5_params['apiKey'] = api_key
    md5_params.pop('md5', None)
    sorted_str = '&'.join([f"{k}={v}" for k, v in sorted(md5_params.items())])
    return hashlib.md5(sorted_str.encode('utf-8')).hexdigest().upper()

def get_market_prices(nautilus_ids: list[str], client: HistoricThinkTraderClient) -> tuple[dict, dict]:
    """多级抓取策略：快照 -> 昨收 -> 历史K线，并补全名称"""
    qmt_codes = [cid.replace('.SZSE', '.SZ').replace('.SSE', '.SH') for cid in nautilus_ids]
    client.subscribe_whole_quote(qmt_codes)
    
    ticks = client.get_full_tick(qmt_codes)
    prices, names = {}, {}
    fallback_qmt = []

    for nid in nautilus_ids:
        qid = nid.replace('.SZSE', '.SZ').replace('.SSE', '.SH')
        detail = client.get_instrument_detail(qid)
        names[nid] = detail.get('InstrumentName', '未知') if detail else "未知"
        
        # 优先级一：实时/盘后快照
        raw_tick = ticks.get(qid, {})
        p = float(raw_tick.get('lastPrice', 0) or raw_tick.get('last_price', 0) or 
                  raw_tick.get('preClose', 0) or raw_tick.get('pre_close', 0) or 0)
        
        if p > 0:
            prices[nid] = p
        else:
            fallback_qmt.append(qid)
            prices[nid] = 0.0

    # 优先级二：历史日线数据兜底
    if fallback_qmt:
        try:
            daily = client.get_market_data(field_list=["close"], stock_list=fallback_qmt, period="1d", count=5)
            for qid in fallback_qmt:
                nid = qid.replace('.SZ', '.SZSE').replace('.SH', '.SSE')
                if qid in daily and not daily[qid].empty:
                    prices[nid] = float(daily[qid]['close'].dropna().iloc[-1])
        except Exception: pass
            
    return prices, names

# ================= 调仓策略实现 =================

class DailyRebalanceStrategyConfig(StrategyConfig, frozen=True):
    target_pool: tuple[dict, ...]  # 包含 code, name, score
    simulation_mode: bool = True   # 是否为模拟运行

class DailyRebalanceStrategy(Strategy):
    """每日自动化调仓策略，支持多级价格抓取与详细报表输出"""
    
    def __init__(self, config: DailyRebalanceStrategyConfig, ext_logger: logging.Logger, historic_client: HistoricThinkTraderClient) -> None:
        super().__init__(config)
        self.pool_map = {item['code']: item for item in config.target_pool}
        self.simulation_mode = config.simulation_mode
        self.ext_logger = ext_logger
        self.historic_client = historic_client

    def on_start(self) -> None:
        """策略启动：预订阅行情并开启同步定时器"""
        self.ext_logger.info("交易节点已连接。正在执行10秒数据同步宽限...")
        
        # 抢跑订阅预测池代码
        qmt_pool = [c.replace('.SZSE', '.SZ').replace('.SSE', '.SH') for c in self.pool_map.keys()]
        self.historic_client.subscribe_whole_quote(qmt_pool)
        
        self.clock.set_time_alert('init_task', self.clock.utc_now() + timedelta(seconds=10), callback=self.on_timer)

    def on_timer(self, event) -> None:
        if event.name == 'init_task': self.execute_rebalance()
        elif event.name == 'shutdown': shutdown_event.set()

    def execute_rebalance(self) -> None:
        """调仓核心逻辑"""
        try:
            # 1. 资产对账
            account = list(self.cache.accounts())[0] if self.cache.accounts() else None
            if not account:
                self.ext_logger.warning("⏰ 账户缓存未就绪，5秒后重试...")
                self.clock.set_time_alert('init_task', self.clock.utc_now() + timedelta(seconds=5), callback=self.on_timer)
                return

            # 获取 QMT 内部推送的持仓字典（最准确的价格与市值来源）
            qmt_info = account.last_event.info if (hasattr(account, 'last_event') and account.last_event) else {}
            pos_report = qmt_info.get("positions_map", {})
            
            # 2. 统计当前持仓
            holding_map = {str(p.instrument_id): float(p.quantity) for p in self.cache.positions() if float(p.quantity) > 0}
            all_involved = list(set(self.pool_map.keys()) | set(holding_map.keys()))
            
            # 3. 价格聚合
            snap_prices, names = get_market_prices(all_involved, self.historic_client)
            final_prices, final_mv = {}, {}
            
            for nid in all_involved:
                p_info = pos_report.get(nid, {})
                mv = float(p_info.get("market_value", 0))
                price = (mv / holding_map[nid]) if (nid in holding_map and mv > 0) else snap_prices.get(nid, 0)
                
                final_prices[nid] = price
                final_mv[nid] = mv if nid in holding_map else 0

            # 4. 财务总览
            bal = account.balance()
            cash_free = float(bal.free) if bal else 0.0
            total_mv = sum(final_mv.values())
            total_assets = cash_free + float(bal.locked or 0) + total_mv
            budget_per_stock = total_assets / len(self.pool_map) if self.pool_map else 0

            # --- 结果打印：持仓明细 ---
            self.ext_logger.info("\n" + "="*20 + " 【1/4 当前实盘持仓明细】 " + "="*20)
            if not holding_map: self.ext_logger.info("当前无持仓。")
            else:
                df_h = pd.DataFrame([{
                    "代码": c, "名称": names.get(c, "未知"), "数量": f"{q:,.0f}",
                    "现价": f"{final_prices[c]:,.2f}", "市值": f"{final_mv[c]:,.2f}",
                    "状态": "保留" if c in self.pool_map else "需清理"
                } for c, q in holding_map.items()])
                self.ext_logger.info("\n" + TableFormatter.format_df(df_h))

            # --- 结果打印：AI预测池 ---
            self.ext_logger.info("\n" + "="*20 + " 【2/4 预测池】 " + "="*20)
            df_p = pd.DataFrame([{
                "代码": c, "名称": names.get(c, m['name']), "分数": m['score'],
                "现价": f"{final_prices[c]:,.2f}", "备注": "持仓中" if c in holding_map else "待调入"
            } for c, m in self.pool_map.items()])
            self.ext_logger.info("\n" + TableFormatter.format_df(df_p))

            # 5. 生成交易指令
            orders = []
            # 卖出逻辑
            for nid, qty in holding_map.items():
                if nid not in self.pool_map:
                    orders.append({"动作": "SELL", "代码": nid, "数量": qty, "理由": "策略调出"})
                else:
                    px = final_prices[nid]
                    if px > 0:
                        target_q = (budget_per_stock // px // 100) * 100
                        if target_q < qty - 99: orders.append({"动作": "SELL", "代码": nid, "数量": qty - target_q, "理由": "超盈减仓"})
            
            # 买入逻辑
            for nid in self.pool_map.keys():
                px = final_prices[nid]
                if px <= 0: continue
                target_q = (budget_per_stock // px // 100) * 100
                curr_q = holding_map.get(nid, 0)
                if target_q > curr_q + 99:
                    orders.append({"动作": "BUY", "代码": nid, "数量": target_q - curr_q, "理由": "低配加仓"})

            # --- 结果打印：指令执行 ---
            self.ext_logger.info("\n" + "="*20 + " 【3/4 拟定调仓指令】 " + "="*20)
            if self.simulation_mode: self.ext_logger.warning("⚠️ 模拟模式运行，不会真实下单。")
            if not orders: self.ext_logger.info("组合已达最优状态，无需操作。")
            else:
                df_o = pd.DataFrame(orders)
                df_o["数量"] = df_o["数量"].map("{:,.0f}".format)
                self.ext_logger.info("\n" + TableFormatter.format_df(df_o))
                
                if not self.simulation_mode:
                    for o in orders:
                        instr = InstrumentId.from_str(o['代码'])
                        side = OrderSide.BUY if o['动作'] == 'BUY' else OrderSide.SELL
                        qty = self.cache.instrument(instr).make_qty(float(o['数量']))
                        self.submit_order(self.order_factory.market(instr, side, qty))

            self.ext_logger.info(f"\n✨ 执行完毕。总资产: {total_assets:,.2f} 元。")
            self.clock.set_time_alert('shutdown', self.clock.utc_now() + timedelta(seconds=10), callback=self.on_timer)

        except Exception as e:
            self.ext_logger.error(f"💥 调仓致命故障: {e}", exc_info=True)
            self.clock.set_time_alert('shutdown', self.clock.utc_now() + timedelta(seconds=5), callback=self.on_timer)

# ================= 启动准备与主入口 =================

def fetch_noesis_pool(logger: logging.Logger) -> list[dict]:
    """获取云端股票池"""
    payload = {
        "capital": "1000000", "date": (datetime.now() - timedelta(days=1)).strftime("%Y%m%d"),
        "initDate": datetime.now().strftime("%Y%m%d"), "license": "c5ddbf5f-2917-44d4-9b66-d4ab239cbac5",
        "macAddress": "00:16:3E:41:EA:AF", "positionStockCodes": "", "securitiesAccount": "693", "sign": "noesis_2025"
    }
    payload['md5'] = generate_md5(payload)
    try:
        resp = requests.post("https://noesisai.cn/quantization-ai/api/portal/stockNamePrediction", json=payload, timeout=20)
        res = resp.json().get('result', {}).get('nameList', [])
        logger.info(f"API 获取成功: {len(res)} 只股票")
        return [{'code': qid.replace('.SZ', '.SZSE').replace('.SH', '.SSE'), 'name': item['name'], 'score': str(item['score'])} 
                for item in res for qid in [item['code']]]
    except Exception as e:
        logger.error(f"池获取失败: {e}"); return []

def main():
    script_path = Path(__file__).resolve().parent
    (script_path / "output").mkdir(exist_ok=True)
    log_file = script_path / "output" / f"{datetime.now().strftime('%Y-%m-%d')}.log"

    logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(levelname)s - %(message)s',
                        handlers=[logging.FileHandler(log_file, encoding='utf-8'), logging.StreamHandler()])
    logger = logging.getLogger("Rebalancer")

    # 配置准备
    m_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
    acc_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
    s_id = random.randint(100000, 999999)

    pool = fetch_noesis_pool(logger)
    if not pool: return

    # 启动节点
    historic = HistoricThinkTraderClient(miniqmt_path=m_path)
    node = TradingNode(config=TradingNodeConfig(
        trader_id=TraderId("AUTO-REBAL"),
        data_clients={TT: ThinkTraderDataClientConfig(miniqmt_path=m_path, session_id=s_id, instrument_provider=ThinkTraderInstrumentProviderConfig(load_all=True))},
        exec_clients={TT: ThinkTraderExecClientConfig(miniqmt_path=m_path, account_id=acc_id, session_id=s_id, routing=RoutingConfig(default=True))}
    ))
    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
    
    node.trader.add_strategy(DailyRebalanceStrategy(
        config=DailyRebalanceStrategyConfig(target_pool=tuple(pool), simulation_mode=True),
        ext_logger=logger, historic_client=historic
    ))

    node.build()
    threading.Thread(target=lambda: (shutdown_event.wait(), node.stop()), daemon=True).start()

    try: node.run()
    finally: node.dispose()

if __name__ == "__main__":
    main()
