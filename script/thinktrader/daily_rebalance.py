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
        if align == 'left':
            return text + (' ' * padding)
        elif align == 'right':
            return (' ' * padding) + text
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
            widths.append(max_w + 2)  # 两边各留一个空格

        sep = "+" + "+".join(["-" * w for w in widths]) + "+"
        lines = [sep]

        # 表头行
        header = "|" + "|".join([cls.pad_text(f" {cols[i]} ", widths[i], 'left') for i in range(len(cols))]) + "|"
        lines.extend([header, sep])

        # 数据行
        for row in str_data:
            row_str = "|"
            for i, val in enumerate(row):
                # 凡是能转换为数值或仅包含数字及常见数值符号(,-.%)的均视为数字类，采用右对齐
                clean_val = val.replace(',', '').replace('%', '').replace('-', '').strip()
                is_num = clean_val.replace('.', '', 1).isdigit() and any(c.isdigit() for c in clean_val)
                align = 'right' if is_num else 'left'
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


def _run_async_coro(coro, ref_loop):
    import asyncio
    import concurrent.futures
    if ref_loop and ref_loop.is_running():
        future = asyncio.run_coroutine_threadsafe(coro, ref_loop)
        return future.result(timeout=30)
    if ref_loop:
        return ref_loop.run_until_complete(coro)
    new_loop = asyncio.new_event_loop()
    try:
        return new_loop.run_until_complete(coro)
    finally:
        new_loop.close()


def get_market_prices(nautilus_ids: list[str], client: HistoricThinkTraderClient) -> tuple[dict, dict]:
    """
    多级抓取策略获取市价并补全名称：
    1. 快照兜底 (client.get_full_tick)
    2. Tick明细查询 (最近6秒报盘数据兜底，处理停牌/非交易时间段)
    3. 历史日线昨收兜底
    """
    qmt_codes = [cid.replace('.SZSE', '.SZ').replace('.SSE', '.SH') for cid in nautilus_ids]
    client.subscribe_whole_quote(qmt_codes)
    
    ticks = client.get_full_tick(qmt_codes)
    prices, names = {}, {}
    fallback_qmt = []

    for nid in nautilus_ids:
        qid = nid.replace('.SZSE', '.SZ').replace('.SSE', '.SH')
        detail = client.get_instrument_detail(qid)
        names[nid] = detail.get('InstrumentName', '未知') if detail else "未知"

        # [优先级1] 快照读取 (从 XTData 本地缓存获取)
        raw_tick = ticks.get(qid, {})
        # 兼容不同版本的 XTData 字段名（大写/小写下划线）
        p = float(raw_tick.get('lastPrice') or raw_tick.get('last_price') or 
                  raw_tick.get('preClose') or raw_tick.get('pre_close') or 
                  raw_tick.get('close') or 0.0)

        if p > 0:
            prices[nid] = p
        else:
            fallback_qmt.append(qid)
            prices[nid] = 0.0

    # [优先级2] 获取近6秒内的最新 Tick 盘口兜底
    if fallback_qmt:
        now = datetime.now()
        end_time = now.replace(hour=15, minute=0, second=0, microsecond=0) if now.hour >= 15 else now
        start_time = end_time - timedelta(seconds=6)

        ref_loop = getattr(client._client, "loop", None)
        nid_list = [qid.replace('.SZ', '.SZSE').replace('.SH', '.SSE') for qid in fallback_qmt]

        try:
            client.log.info(f"正在尝试获取 {len(nid_list)} 只股票的最新Tick数据...")
            coro = client.request_ticks(
                tick_type="BID_ASK", instrument_ids=nid_list,
                start_date_time=start_time, end_date_time=end_time,
                tz_name="Asia/Shanghai", timeout=5, limit=0,
            )
            recent_ticks = _run_async_coro(coro, ref_loop)

            ticks_by_sym = {str(t.instrument_id): t for t in recent_ticks} if recent_ticks else {}
            successful_qmt = []

            for nid in nid_list:
                if nid in ticks_by_sym:
                    last_tick = ticks_by_sym[nid]
                    p = getattr(last_tick.ask_price, "as_double", lambda: 0.0)()
                    if p <= 0:
                        p = getattr(last_tick.bid_price, "as_double", lambda: 0.0)()
                    if p > 0:
                        prices[nid] = p
                        qid = nid.replace('.SZSE', '.SZ').replace('.SSE', '.SH')
                        successful_qmt.append(qid)

            fallback_qmt = [q for q in fallback_qmt if q not in successful_qmt]
            client.log.info(f"Tick 获取完成。成功 {len(successful_qmt)} 只，失败 {len(fallback_qmt)} 只。")
        except Exception as e:
            client.log.warning(f"Tick 获取异常或超时: {e}")

    # [优先级3] 历史日线数据兜底 (停牌极长股票)
    if fallback_qmt:
        try:
            daily = client.get_market_data(field_list=["close"], stock_list=fallback_qmt, period="1d", count=5)
            for qid in fallback_qmt:
                nid = qid.replace('.SZ', '.SZSE').replace('.SH', '.SSE')
                if qid in daily and not daily[qid].empty:
                    prices[nid] = float(daily[qid]['close'].dropna().iloc[-1])
        except Exception:
            pass

    return prices, names


# ================= 调仓策略实现 =================

class DailyRebalanceStrategyConfig(StrategyConfig, frozen=True):
    target_pool: tuple[dict, ...]
    simulation_mode: bool = True


class DailyRebalanceStrategy(Strategy):
    """每日自动化调仓策略"""

    def __init__(self, config: DailyRebalanceStrategyConfig, ext_logger: logging.Logger,
                 historic_client: HistoricThinkTraderClient) -> None:
        super().__init__(config)
        self.pool_map = {item['code']: item for item in config.target_pool}
        self.simulation_mode = config.simulation_mode
        self.ext_logger = ext_logger
        self.historic_client = historic_client

    def on_start(self) -> None:
        self.ext_logger.info("交易节点已连接。正在执行10秒数据同步宽限...")
        qmt_pool = [c.replace('.SZSE', '.SZ').replace('.SSE', '.SH') for c in self.pool_map.keys()]
        self.historic_client.subscribe_whole_quote(qmt_pool)
        self.clock.set_time_alert('init_task', self.clock.utc_now() + timedelta(seconds=10), callback=self.on_timer)

    def on_timer(self, event) -> None:
        if event.name == 'init_task':
            self.execute_rebalance()
        elif event.name == 'shutdown':
            shutdown_event.set()

    def execute_rebalance(self) -> None:
        try:
            # 1. 资产检查与持仓同步
            account = next(iter(self.cache.accounts()), None)
            if not account:
                self.ext_logger.warning("⏰ 账户缓存未就绪，5秒后重试...")
                self.clock.set_time_alert('init_task', self.clock.utc_now() + timedelta(seconds=5),
                                          callback=self.on_timer)
                return

            qmt_info = account.last_event.info if getattr(account, 'last_event', None) else {}
            pos_report = qmt_info.get("positions_map", {})

            # 2. 统计当前持仓与成本 (合并缓存与QMT实时报表)
            holding_map, cost_map, pnl_map = {}, {}, {}
            for p in self.cache.positions():
                q = float(p.quantity)
                if q > 0:
                    nid = str(p.instrument_id)
                    holding_map[nid] = q
                    cost_px = getattr(p, 'avg_px_open', None) or getattr(p, 'avg_px', None) or getattr(p,
                                                                                                       'average_price',
                                                                                                       0)
                    cost_map[nid] = float(cost_px.as_double() if hasattr(cost_px, 'as_double') else cost_px) or 0.0

            # 同步补全 QMT 报表中的实时持仓
            for nid, p_info in pos_report.items():
                q_qmt = float(p_info.get("volume", 0))
                if q_qmt > 0:
                    holding_map[nid] = q_qmt
                    if nid not in cost_map or cost_map[nid] <= 0:
                        c_px = float(p_info.get("open_price") or p_info.get("average_price") or 0)
                        if c_px > 0: cost_map[nid] = c_px

            all_involved = list(set(self.pool_map.keys()) | set(holding_map.keys()) | set(pos_report.keys()))

            # 2. 价格缓存与估值归集
            snap_prices, names = get_market_prices(all_involved, self.historic_client)
            final_prices, final_mv = {}, {}

            for nid in all_involved:
                p_info = pos_report.get(nid, {})
                q = holding_map.get(nid, 0)

                # 优先从 QMT 报表取价格，取不到则用快照价格
                mv_qmt = float(p_info.get("market_value", 0))
                price = (mv_qmt / q) if (q > 0 and mv_qmt > 0) else snap_prices.get(nid, 0)

                final_prices[nid] = price
                # 市值必须用 q * price，确保持仓市值始终反映最新行情，即使 QMT 报表还没更新
                final_mv[nid] = q * price if q > 0 else 0

                # 浮动盈亏直接取自 QMT (float_pnl)
                raw_pnl = float(p_info.get("float_pnl", 0.0))
                pnl_map[nid] = raw_pnl

                # 成本计算优选：
                # 1. 如果有 raw_pnl，反推成本: (mv - pnl) / q
                # 2. 否则取 QMT 字段: open_price 
                # 3. 否则取 Nautilus 缓存
                c_price = 0.0
                if q > 0:
                    if abs(raw_pnl) > 0.0001:
                        # 用当日快照市值的估算可能会导致成本波动，但逻辑上最实时
                        # 这里优先用 p_info 里的 market_value 反推，那才是 QMT 计算盈亏的基准
                        curr_mv = mv_qmt if mv_qmt > 0 else (q * price)
                        c_price = (curr_mv - raw_pnl) / q
                    if c_price <= 0:
                        c_price = float(p_info.get("open_price") or p_info.get("average_price") or 0)
                    if c_price <= 0 and p_info.get("position_cost"):
                        c_price = float(p_info.get("position_cost")) / q

                if c_price <= 0:
                    c_price = cost_map.get(nid, 0)

                # 兜底：如果还是0，取现价
                cost_map[nid] = c_price if c_price > 0 else price

            # 3. 财务资产总览
            bal = account.balance()
            cash_free = float(bal.free) if bal else 0.0
            total_mv = sum(final_mv.values())
            # 总资产 = 可用现金 + 冻结资金 (挂单保证金) + 股票市值
            total_assets = cash_free + float(bal.locked or 0) + total_mv
            # 预留 0.5% 的资金作为手续费和滑点缓冲，防止买入单因金额不足被拒
            investable_assets = total_assets * 0.995
            budget_per_stock = investable_assets / len(self.pool_map) if self.pool_map else 0

            self.ext_logger.info(f"\n<<< 调仓前账户状态 >>>\n"
                                 f"总资产: {total_assets:,.2f} | 总股票市值: {total_mv:,.2f} | 可用余额: {cash_free:,.2f} | 冻结资金: {float(bal.locked or 0):,.2f}\n"
                                 f"可投分配金额: {investable_assets:,.2f} | 每只标的预算: {budget_per_stock:,.2f}")

            # 4. 指令生成计算
            orders = []

            for nid, qty in holding_map.items():
                px = final_prices[nid]
                if nid not in self.pool_map:
                    orders.append(
                        {"代码": nid,
                         "名称": names.get(nid, "未知"),
                         "动作": "SELL",
                         "成交价格": px,
                         "成交数量": qty,
                         "成交金额": px * qty})
                elif px > 0:
                    target_q = (budget_per_stock // px // 100) * 100
                    if target_q < qty - 99:
                        orders.append({"代码": nid,
                                       "名称": names.get(nid, "未知"),
                                       "动作": "SELL",
                                       "成交价格": px,
                                       "成交数量": qty - target_q,
                                       "成交金额": px * (qty - target_q)})

            for nid in self.pool_map.keys():
                px = final_prices[nid]
                if px <= 0: continue
                target_q = (budget_per_stock // px // 100) * 100
                curr_q = holding_map.get(nid, 0)
                if target_q > curr_q + 99:
                    orders.append({"代码": nid,
                                   "名称": names.get(nid, "未知"),
                                   "动作": "BUY",
                                   "成交价格": px,
                                   "成交数量": target_q - curr_q,
                                   "成交金额": px * (target_q - curr_q)})

            # 5. 期末持仓预览推算
            target_holding, target_cost = holding_map.copy(), cost_map.copy()
            for o in orders:
                nid, q, px = o["代码"], float(o["成交数量"]), float(o["成交价格"])
                if o["动作"] == "BUY":
                    old_q, old_c = target_holding.get(nid, 0), target_cost.get(nid, 0)
                    new_q = old_q + q
                    if new_q > 0: target_cost[nid] = (old_q * old_c + q * px) / new_q
                    target_holding[nid] = new_q
                else:
                    target_holding[nid] = max(0, target_holding.get(nid, 0) - q)
            target_holding = {k: v for k, v in target_holding.items() if v > 0}

            # =================== 格式化日志输出区域 ===================

            def _build_holding_row(c, q, cost, use_raw_pnl=True):
                """辅助构建仓位表格Row"""
                px, mv = final_prices[c], q * final_prices[c]
                # 优先使用 QMT 原始盈亏数据 (仅用于表1)，否则根据成本计算 (表4)
                pnl = pnl_map.get(c, 0.0) if use_raw_pnl else 0.0
                if abs(pnl) < 0.0001:
                    pnl = (px - cost) * q if abs(cost) > 0.0001 else 0

                ret = (pnl / (q * cost)) if (q > 0 and abs(cost) > 0.0001) else 0
                return {
                    "代码": c.split('.')[0],
                    "名称": names.get(c, "未知"),
                    "数量": f"{q:,.0f}",
                    "市值": f"{mv:,.2f}",
                    "最新价": f"{px:,.2f}",
                    "成本价": f"{cost:,.3f}",
                    "盈亏": f"{pnl:,.2f}",
                    "盈亏比例": f"{ret:.2%}"
                }

            # 【表1】当前实盘明细
            self.ext_logger.info("\n" + "=" * 20 + " 【1/4 当前实盘持仓明细】 " + "=" * 20)
            if not holding_map:
                self.ext_logger.info("当前无持仓。")
            else:
                df_h = pd.DataFrame(
                    [_build_holding_row(c, q, cost_map[c], use_raw_pnl=True) for c, q in holding_map.items()])
                df_h.insert(0, "序号", range(1, len(df_h) + 1))
                self.ext_logger.info("\n" + TableFormatter.format_df(df_h))

            # 【表2】预测池
            self.ext_logger.info("\n" + "=" * 20 + " 【2/4 预测池】 " + "=" * 20)
            df_p = pd.DataFrame([{
                "代码": c.split('.')[0],
                "名称": names.get(c, m['name']),
                "分数": m['score'],
                "现价": f"{final_prices[c]:,.2f}",
                "备注": "持仓中" if c in holding_map else "待调入"
            } for c, m in self.pool_map.items()])
            df_p.insert(0, "序号", range(1, len(df_p) + 1))
            self.ext_logger.info("\n" + TableFormatter.format_df(df_p))

            # 【表3】调仓信指表
            self.ext_logger.info("\n" + "=" * 20 + " 【3/4 拟定调仓指令】 " + "=" * 20)
            if self.simulation_mode: self.ext_logger.warning("⚠️ 模拟模式运行，不会真实下单。")

            if not orders:
                self.ext_logger.info("组合已达最优状态，无需操作。")
            else:
                df_o = pd.DataFrame(orders)[["代码", "名称", "动作", "成交价格", "成交数量", "成交金额"]]
                df_o["代码"] = df_o["代码"].apply(lambda x: x.split('.')[0])
                for col in ["成交数量", "成交价格", "成交金额"]:
                    df_o[col] = df_o[col].map("{:,.2f}".format if col != "成交数量" else "{:,.0f}".format)
                df_o.insert(0, "序号", range(1, len(df_o) + 1))
                self.ext_logger.info("\n" + TableFormatter.format_df(df_o))

                # 触发实盘策略订单
                if not self.simulation_mode:
                    for o in orders:
                        instr = InstrumentId.from_str(o['代码'])
                        side = OrderSide.BUY if o['动作'] == 'BUY' else OrderSide.SELL
                        qty = self.cache.instrument(instr).make_qty(float(o['成交数量']))
                        self.submit_order(self.order_factory.market(instr, side, qty))

            # 【表4】拟合推演表
            self.ext_logger.info("\n" + "=" * 20 + " 【4/4 目标组合明细】 " + "=" * 20)
            if not target_holding:
                self.ext_logger.info("调仓后组合为空。")
            else:
                df_t = pd.DataFrame(
                    [_build_holding_row(c, q, target_cost[c], use_raw_pnl=False) for c, q in target_holding.items()])
                df_t.insert(0, "序号", range(1, len(df_t) + 1))
                self.ext_logger.info("\n" + TableFormatter.format_df(df_t))

            # 结论结算
            expected_mv = sum((q * final_prices[c]) for c, q in target_holding.items())
            expected_cash = total_assets - expected_mv - float(bal.locked or 0)
            self.ext_logger.info(
                f"\n<<< 调仓后预期状态 >>>\n总资产 (忽略滑点/手续费): {total_assets:,.2f} | 目标总股票市值: {expected_mv:,.2f} | 预估目标可用余额: {expected_cash:,.2f}")

            self.ext_logger.info("\n✨ 调仓轮次执行完毕。")
            self.clock.set_time_alert('shutdown', self.clock.utc_now() + timedelta(seconds=10), callback=self.on_timer)

        except Exception as e:
            self.ext_logger.error(f"💥 调仓致命故障: {e}", exc_info=True)
            self.clock.set_time_alert('shutdown', self.clock.utc_now() + timedelta(seconds=5), callback=self.on_timer)


# ================= 启动准备与主入口 =================

def fetch_noesis_pool(logger: logging.Logger, max_retry_days: int = 10) -> list[dict]:
    """获取股票池 (支持非交易日自动向前追溯)"""
    base_payload = {
        "capital": "1000000",
        "initDate": datetime.now().strftime("%Y%m%d"),
        "license": "c5ddbf5f-2917-44d4-9b66-d4ab239cbac5",
        "macAddress": "00:16:3E:41:EA:AF",
        "positionStockCodes": "",
        "securitiesAccount": "179",
        "sign": "noesis_2025"
    }

    for i in range(1, max_retry_days + 1):
        target_date = (datetime.now() - timedelta(days=i)).strftime("%Y%m%d")
        payload = base_payload.copy()
        payload["date"] = target_date
        payload['md5'] = generate_md5(payload)
        
        try:
            logger.info(f"正在尝试获取日期 {target_date} 的股票池 (尝试 {i}/{max_retry_days})...")
            resp = requests.post("https://noesisai.cn/quantization-ai/api/portal/stockNamePrediction", 
                                 json=payload, timeout=20)
            resp.raise_for_status()
            
            data = resp.json()
            # 健壮性检查：确保 result 和 nameList 存在且非空
            result = data.get('result')
            if result is None:
                logger.warning(f"日期 {target_date} API 返回结果为空 (result is None)")
                continue
                
            res = result.get('nameList', [])
            if not res:
                logger.warning(f"日期 {target_date} 获取到 0 只股票，准备尝试前一天...")
                continue

            logger.info(f"成功获取日期 {target_date} 的股票池: {len(res)} 只股票")
            return [{'code': qid.replace('.SZ', '.SZSE').replace('.SH', '.SSE'),
                     'name': item['name'],
                     'score': str(item['score'])}
                    for item in res for qid in [item['code']]]
                    
        except Exception as e:
            logger.error(f"日期 {target_date} 请求发生错误: {e}")
            if i == max_retry_days: break
            continue

    logger.error(f"❌ 经过 {max_retry_days} 天追溯，仍未能获取到有效的股票池。")
    return []


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
        data_clients={TT: ThinkTraderDataClientConfig(miniqmt_path=m_path,
                                                      session_id=s_id,
                                                      instrument_provider=ThinkTraderInstrumentProviderConfig(
                                                          load_all=True))},
        exec_clients={TT: ThinkTraderExecClientConfig(miniqmt_path=m_path,
                                                      account_id=acc_id,
                                                      session_id=s_id,
                                                      routing=RoutingConfig(default=True))}
    ))
    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)

    node.trader.add_strategy(DailyRebalanceStrategy(
        config=DailyRebalanceStrategyConfig(target_pool=tuple(pool), simulation_mode=True),
        ext_logger=logger,
        historic_client=historic
    ))

    node.build()
    threading.Thread(target=lambda: (shutdown_event.wait(), node.stop()), daemon=True).start()

    try:
        node.run()
    finally:
        node.dispose()


if __name__ == "__main__":
    main()
