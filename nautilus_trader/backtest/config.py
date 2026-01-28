# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------

from __future__ import annotations

import sys
from typing import Any

import msgspec
import pandas as pd

from nautilus_trader.cache.config import CacheConfig
from nautilus_trader.common import Environment
from nautilus_trader.common.config import ActorConfig
from nautilus_trader.common.config import ImportableActorConfig
from nautilus_trader.common.config import NautilusConfig
from nautilus_trader.common.config import NonNegativeInt
from nautilus_trader.common.config import msgspec_encoding_hook
from nautilus_trader.common.config import resolve_config_path
from nautilus_trader.common.config import resolve_path
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.datetime import dt_to_unix_nanos
from nautilus_trader.data.config import DataEngineConfig
from nautilus_trader.execution.config import ExecEngineConfig
from nautilus_trader.live.config import LiveDataClientConfig
from nautilus_trader.model.data import Bar
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import BookType
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.enums import OtoTriggerMode
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.persistence.funcs import parse_filters_expr
from nautilus_trader.risk.config import RiskEngineConfig
from nautilus_trader.system.config import NautilusKernelConfig


class BacktestVenueConfig(NautilusConfig, frozen=True):
    """
    表示特定回测引擎的交易场所配置。

    参数
    ----------
    name : str
        交易场所名称。
    oms_type : OmsType | str
        交易所的订单管理系统类型。如果为 ``HEDGING``（对冲），将生成新的持仓 ID。
    account_type : AccountType | str
        交易所的账户类型。
    starting_balances : list[Money | str]
        起始账户余额（单一资产账户只需指定一个）。
    base_currency : Currency | str, 可选
        交易所的账户基础货币。多币种账户使用 ``None``。
    default_leverage : float, 可选
        账户默认杠杆（用于保证金账户）。
    leverages : dict[str, float], 可选
        合约特定的杠杆配置（用于保证金账户）。
    margin_model : MarginModelConfig, 可选
        保证金计算模型配置。默认为 'leveraged'。
    modules : list[ImportableActorConfig], 可选
        交易场所的模拟模块。
    fill_model : ImportableFillModelConfig, 可选
        交易场所的成交模型。
    latency_model : ImportableLatencyModelConfig, 可选
        交易场所的延迟模型。
    fee_model : ImportableFeeModelConfig, 可选
        交易场所的手续费模型。
    book_type : str, 默认 'L1_MBP'
        默认订单簿类型。
    routing : bool, 默认 False
        是否为执行客户端启用多场所路由。
    reject_stop_orders : bool, 默认 True
        如果触发价格在市场内，止损订单是否在提交时被拒绝。
    support_gtd_orders : bool, 默认 True
        交易场所是否支持 GTD（Good-Till-Date）时效的订单。
    support_contingent_orders : bool, 默认 True
        交易场所是否支持/遵守条件订单。
        如果为 False，则期望策略自己管理条件订单。
    oto_trigger_mode : OtoTriggerMode | str, 默认 "PARTIAL"
        条件订单的 OTO（One-Triggers-Other）触发模式：
        - ``PARTIAL``：按每次部分成交比例释放子订单（默认）。
        - ``FULL``：仅在父订单完全成交后才释放子订单。
    use_position_ids : bool, 默认 True
        是否在订单成交时生成交易场所持仓 ID。
    use_random_ids : bool, 默认 False
        是否所有交易场所生成的标识符都使用随机 UUID4。
    use_reduce_only : bool, 默认 True
        是否遵守订单上的 `reduce_only` 执行指令。
    use_market_order_acks : bool, 默认 False
        是否在市价单成交前生成 OrderAccepted 事件。
    bar_execution : bool, 默认 True
        K 线数据是否应由撮合引擎处理（并驱动市场）。
    bar_adaptive_high_low_ordering : bool, 默认 False
        确定 K 线价格的处理顺序是否基于启发式自适应。
        此设置仅在 `bar_execution` 为 True 时有效。
        如果为 False，K 线价格始终按固定顺序处理：开盘、最高、最低、收盘。
        如果为 True，处理顺序会自适应启发式规则：
        - 如果最高价比最低价更接近开盘价，则顺序为：开盘、最高、最低、收盘。
        - 如果最低价比最高价更接近开盘价，则顺序为：开盘、最低、最高、收盘。
    trade_execution : bool, 默认 False
        成交数据是否应由撮合引擎处理（并驱动市场）。
    liquidity_consumption : bool, 默认 False
        是否跟踪每个价格档位的流动性消耗。启用时，成交会消耗可用流动性，
        当该档位有新数据到达时重置。禁用时，每次迭代可以独立地对整个订单簿
        流动性进行成交。
    allow_cash_borrowing : bool, 默认 False
        现金账户是否允许借款（负余额）。
    frozen_account : bool, 默认 False
        此交易所的账户是否冻结（余额不会变化）。
    price_protection_points : int, 默认 0
        定义交易所计算的价格边界（以点数为单位），防止可成交订单
        以过于激进的价格执行。

    """

    name: str
    oms_type: OmsType | str
    account_type: AccountType | str
    starting_balances: list[str]
    base_currency: str | None = None
    default_leverage: float = 1.0
    leverages: dict[str, float] | None = None
    margin_model: MarginModelConfig | None = None
    modules: list[ImportableActorConfig] | None = None
    fill_model: ImportableFillModelConfig | None = None
    latency_model: ImportableLatencyModelConfig | None = None
    fee_model: ImportableFeeModelConfig | None = None
    book_type: BookType | str = "L1_MBP"
    routing: bool = False
    reject_stop_orders: bool = True
    support_gtd_orders: bool = True
    support_contingent_orders: bool = True
    oto_trigger_mode: OtoTriggerMode | str = "PARTIAL"
    use_position_ids: bool = True
    use_random_ids: bool = False
    use_reduce_only: bool = True
    use_market_order_acks: bool = False
    bar_execution: bool = True
    bar_adaptive_high_low_ordering: bool = False
    trade_execution: bool = False
    liquidity_consumption: bool = False
    allow_cash_borrowing: bool = False
    frozen_account: bool = False
    price_protection_points: int = 0


class BacktestDataConfig(NautilusConfig, frozen=True):
    """
    表示特定回测运行的数据配置。

    参数
    ----------
    catalog_path : str
        数据目录的路径。
    data_cls : str
        配置的数据类型。
    catalog_fs_protocol : str, 可选
        目录的 `fsspec` 文件系统协议。
    catalog_fs_storage_options : dict, 可选
        `fsspec` 存储选项。
    catalog_fs_rust_storage_options : dict, 可选
        Rust 后端的 `fsspec` 存储选项。
    instrument_id : InstrumentId | str, 可选
        数据配置的合约 ID。
    start_time : str 或 int, 可选
        数据配置的开始时间。
        可以是 ISO 8601 格式的日期时间字符串，或 UNIX 纳秒整数。
    end_time : str 或 int, 可选
        数据配置的结束时间。
        可以是 ISO 8601 格式的日期时间字符串，或 UNIX 纳秒整数。
    filter_expr : str, 可选
        使用 pyarrow 进行数据目录查询时的额外过滤表达式。
    client_id : str, 可选
        数据配置的客户端 ID。
    metadata : dict 或 callable, 可选
        数据目录查询的元数据。
    bar_spec : BarSpecification | str, 可选
        数据目录查询的 K 线规格。
    instrument_ids : list[InstrumentId | str], 可选
        数据目录查询的合约 ID 列表。
        当未指定 instrument_id 时可使用。
        如果指定了 bar_spec，将构建等效的 bar_types 列表。
    bar_types : list[BarType | str], 可选
        数据目录查询的 K 线类型列表。
        当未指定 instrument_id 时可使用。

    """

    catalog_path: str
    data_cls: str
    catalog_fs_protocol: str | None = None
    catalog_fs_storage_options: dict | None = None
    catalog_fs_rust_storage_options: dict | None = None
    instrument_id: InstrumentId | None = None
    start_time: str | int | None = None
    end_time: str | int | None = None
    filter_expr: str | None = None
    client_id: str | None = None
    metadata: dict | Any | None = None
    bar_spec: str | None = None
    instrument_ids: list[str] | None = None
    bar_types: list[str] | None = None

    @property
    def data_type(self) -> type:
        """
        根据配置的 `data_cls` 返回对应的 `type`。

        返回
        -------
        type

        """
        if isinstance(self.data_cls, str):
            return resolve_path(self.data_cls)
        else:
            return self.data_cls

    @property
    def query(self) -> dict[str, Any]:
        """
        返回配置的目录查询对象。

        返回
        -------
        dict[str, Any]

        """
        identifiers = []

        if self.data_cls is Bar:
            if self.bar_types:
                identifiers = [str(bar_type) for bar_type in self.bar_types]
            elif self.instrument_id and self.bar_spec:
                identifiers = [f"{self.instrument_id}-{self.bar_spec}-EXTERNAL"]
            elif self.instrument_ids and self.bar_spec:
                identifiers = [
                    f"{instrument_id}-{self.bar_spec}-EXTERNAL"
                    for instrument_id in self.instrument_ids
                ]

        if not identifiers:
            if self.instrument_id:
                identifiers = [self.instrument_id]
            elif self.instrument_ids:
                identifiers = self.instrument_ids

        return {
            "data_cls": self.data_type,
            "identifiers": identifiers,
            "start": self.start_time,
            "end": self.end_time,
            "filter_expr": parse_filters_expr(self.filter_expr),
            "metadata": self.metadata,
        }

    @property
    def start_time_nanos(self) -> int:
        """
        返回数据配置的开始时间（UNIX 纳秒）。

        如果未指定 `start_time`，则返回零。

        返回
        -------
        int

        """
        if self.start_time is None:
            return 0

        return dt_to_unix_nanos(self.start_time)

    @property
    def end_time_nanos(self) -> int:
        """
        返回数据配置的结束时间（UNIX 纳秒）。

        如果未指定 `end_time`，则返回 sys.maxsize。

        返回
        -------
        int

        """
        if self.end_time is None:
            return sys.maxsize

        return dt_to_unix_nanos(self.end_time)


class BacktestEngineConfig(NautilusKernelConfig, frozen=True):
    """
    ``BacktestEngine`` 实例的配置。

    参数
    ----------
    trader_id : TraderId
        节点的交易者 ID（必须是由连字符分隔的名称和 ID 标签）。
    log_level : str, 默认 "INFO"
        节点的标准输出日志级别。
    loop_debug : bool, 默认 False
        asyncio 事件循环是否应处于调试模式。
    cache : CacheConfig, 可选
        缓存配置。
    data_engine : DataEngineConfig, 可选
        实时数据引擎配置。
    risk_engine : RiskEngineConfig, 可选
        实时风控引擎配置。
    exec_engine : ExecEngineConfig, 可选
        实时执行引擎配置。
    streaming : StreamingConfig, 可选
        流式输出到 feather 文件的配置。
    strategies : list[ImportableStrategyConfig]
        内核的策略配置列表。
    actors : list[ImportableActorConfig]
        内核的 Actor 配置列表。
    exec_algorithms : list[ImportableExecAlgorithmConfig]
        内核的执行算法配置列表。
    controller : ImportableControllerConfig, 可选
        内核的交易控制器。
    load_state : bool, 默认 True
        启动时是否从数据库加载交易策略状态。
    save_state : bool, 默认 True
        停止时是否将交易策略状态保存到数据库。
    bypass_logging : bool, 默认 False
        是否绕过日志记录。
    run_analysis : bool, 默认 True
        回测后是否运行绩效分析。

    """

    environment: Environment = Environment.BACKTEST
    trader_id: TraderId = "BACKTESTER-001"
    cache: CacheConfig | None = CacheConfig(drop_instruments_on_reset=False)
    data_engine: DataEngineConfig | None = DataEngineConfig()
    risk_engine: RiskEngineConfig | None = RiskEngineConfig()
    exec_engine: ExecEngineConfig | None = ExecEngineConfig()
    run_analysis: bool = True

    def __post_init__(self):
        if isinstance(self.trader_id, str):
            msgspec.structs.force_setattr(self, "trader_id", TraderId(self.trader_id))


class BacktestRunConfig(NautilusConfig, frozen=True):
    """
    表示特定回测运行的配置。

    包括带有 actors 和 strategies 的回测引擎，以及交易场所和数据的外部输入。

    参数
    ----------
    venues : list[BacktestVenueConfig]
        回测运行的交易场所配置列表。
    data : list[BacktestDataConfig]
        回测运行的数据配置列表。
    engine : BacktestEngineConfig
        回测引擎配置（核心系统内核）。
    chunk_size : int, 可选
        流式模式下每个块处理的数据点数量。
        如果为 `None`，回测将不使用流式模式，一次性加载所有数据。
    raise_exception : bool, 默认 False
        引擎构建或运行期间的异常是否应抛出以中断节点进程。
    dispose_on_completion : bool, 默认 True
        回测运行完成后是否销毁回测引擎。
        如果为 True，将丢弃数据和所有状态。
        如果为 False，将*仅*丢弃数据。
    start : datetime 或 str 或 int, 可选
        回测运行的开始日期时间（UTC）。
        如果为 ``None``，引擎从数据的开始处运行。
    end : datetime 或 str 或 int, 可选
        回测运行的结束日期时间（UTC）。
        如果为 ``None``，引擎运行到数据的结束处。
    data_clients : dict[str, type[LiveDataClientConfig]], 可选
        回测运行的数据客户端配置。

    注意
    -----
    有效的回测运行配置必须包括：
      - 至少一个 `venues` 配置。
      - 至少一个 `data` 配置。

    """

    venues: list[BacktestVenueConfig]
    data: list[BacktestDataConfig]
    engine: BacktestEngineConfig | None = None
    chunk_size: int | None = None
    raise_exception: bool = False
    dispose_on_completion: bool = True
    start: str | int | None = None
    end: str | int | None = None
    data_clients: dict[str, type[LiveDataClientConfig]] | None = None


class SimulationModuleConfig(ActorConfig, frozen=True):
    """
    ``SimulationModule`` 实例的配置。
    """


class FillModelConfig(NautilusConfig, frozen=True):
    """
    ``FillModel`` 实例的配置。

    参数
    ----------
    prob_fill_on_limit : float, 默认 1.0
        当市场价格停留在限价单价格上时，限价单成交的概率。
    prob_slippage : float, 默认 0.0
        订单成交价格滑动一个 tick 的概率。
    random_seed : int, 可选
        随机种子（如果为 None 则不使用随机种子）。

    """

    prob_fill_on_limit: float = 1.0
    prob_slippage: float = 0.0
    random_seed: int | None = None


class ImportableFillModelConfig(NautilusConfig, frozen=True):
    """
    成交模型实例的配置。

    参数
    ----------
    fill_model_path : str
        成交模型类的完全限定名。
    config_path : str
        配置类的完全限定名。
    config : dict[str, Any]
        成交模型配置。

    """

    fill_model_path: str
    config_path: str
    config: dict[str, Any]


class FillModelFactory:
    """
    提供从可导入配置创建成交模型的功能。
    """

    @staticmethod
    def create(config: ImportableFillModelConfig):
        """
        从给定配置创建成交模型。

        参数
        ----------
        config : ImportableFillModelConfig
            构建步骤的配置。

        返回
        -------
        FillModel

        抛出
        ------
        TypeError
            如果 `config` 的类型不是 `ImportableFillModelConfig`。

        """
        PyCondition.type(config, ImportableFillModelConfig, "config")
        fill_model_cls = resolve_path(config.fill_model_path)
        config_cls = resolve_config_path(config.config_path)
        json = msgspec.json.encode(config.config, enc_hook=msgspec_encoding_hook)
        config_obj = config_cls.parse(json)
        return fill_model_cls(config=config_obj)


class LatencyModelConfig(NautilusConfig, frozen=True):
    """
    ``LatencyModel`` 实例的配置。

    参数
    ----------
    base_latency_nanos : int, 默认 1_000_000_000
        模型的基础延迟（纳秒）。
    insert_latency_nanos : int, 默认 0
        模型的订单提交延迟（纳秒）。
    update_latency_nanos : int, 默认 0
        模型的订单修改延迟（纳秒）。
    cancel_latency_nanos : int, 默认 0
        模型的订单取消延迟（纳秒）。

    """

    base_latency_nanos: NonNegativeInt = 1_000_000_000  # 1 毫秒（纳秒单位）
    insert_latency_nanos: NonNegativeInt = 0
    update_latency_nanos: NonNegativeInt = 0
    cancel_latency_nanos: NonNegativeInt = 0


class ImportableLatencyModelConfig(NautilusConfig, frozen=True):
    """
    延迟模型实例的配置。

    参数
    ----------
    latency_model_path : str
        延迟模型类的完全限定名。
    config_path : str
        配置类的完全限定名。
    config : dict[str, Any]
        延迟模型配置。

    """

    latency_model_path: str
    config_path: str
    config: dict[str, Any]


class LatencyModelFactory:
    """
    提供从可导入配置创建延迟模型的功能。
    """

    @staticmethod
    def create(config: ImportableLatencyModelConfig):
        """
        从给定配置创建延迟模型。

        参数
        ----------
        config : ImportableLatencyModelConfig
            构建步骤的配置。

        返回
        -------
        LatencyModel

        抛出
        ------
        TypeError
            如果 `config` 的类型不是 `ImportableLatencyModelConfig`。

        """
        PyCondition.type(config, ImportableLatencyModelConfig, "config")
        latency_model_cls = resolve_path(config.latency_model_path)
        config_cls = resolve_config_path(config.config_path)
        json = msgspec.json.encode(config.config, enc_hook=msgspec_encoding_hook)
        config_obj = config_cls.parse(json)
        return latency_model_cls(config=config_obj)


class FeeModelConfig(NautilusConfig, frozen=True):
    """
    ``FeeModel`` 实例的基础配置。
    """


class MakerTakerFeeModelConfig(FeeModelConfig, frozen=True):
    """
    ``MakerTakerFeeModel`` 实例的配置。

    此费用模型使用合约上定义的 maker/taker 费率。

    """


class FixedFeeModelConfig(FeeModelConfig, frozen=True):
    """
    ``FixedFeeModel`` 实例的配置。

    参数
    ----------
    commission : Money | str
        交易的固定佣金金额。
    charge_commission_once : bool, 默认 True
        是每个订单收取一次佣金，还是每次成交都收取。

    """

    commission: str
    charge_commission_once: bool = True


class PerContractFeeModelConfig(FeeModelConfig, frozen=True):
    """
    ``PerContractFeeModel`` 实例的配置。

    参数
    ----------
    commission : Money | str
        每份合约的佣金金额。

    """

    commission: str


class ImportableFeeModelConfig(NautilusConfig, frozen=True):
    """
    费用模型实例的配置。

    参数
    ----------
    fee_model_path : str
        费用模型类的完全限定名。
    config_path : str
        配置类的完全限定名。
    config : dict[str, Any]
        费用模型配置。

    """

    fee_model_path: str
    config_path: str
    config: dict[str, Any]


class FeeModelFactory:
    """
    提供从可导入配置创建费用模型的功能。
    """

    @staticmethod
    def create(config: ImportableFeeModelConfig):
        """
        从给定配置创建费用模型。

        参数
        ----------
        config : ImportableFeeModelConfig
            构建步骤的配置。

        返回
        -------
        FeeModel

        抛出
        ------
        TypeError
            如果 `config` 的类型不是 `ImportableFeeModelConfig`。

        """
        PyCondition.type(config, ImportableFeeModelConfig, "config")
        fee_model_cls = resolve_path(config.fee_model_path)
        config_cls = resolve_config_path(config.config_path)
        json = msgspec.json.encode(config.config, enc_hook=msgspec_encoding_hook)
        config_obj = config_cls.parse(json)
        return fee_model_cls(config=config_obj)


class FXRolloverInterestConfig(SimulationModuleConfig, frozen=True):
    """
    提供外汇展期利息模拟模块。

    参数
    ----------
    rate_data : pd.DataFrame
        内部展期利息计算器的利率数据。

    """

    rate_data: pd.DataFrame  # TODO: 这可能可以直接变成 JSON 数据


class MarginModelConfig(NautilusConfig, frozen=True):
    """
    保证金计算模型的配置。

    参数
    ----------
    model_type : str, 默认 'leveraged'
        要使用的保证金模型类型。选项：
        - "standard"：无杠杆除法的固定百分比（传统经纪商）
        - "leveraged"：保证金要求按杠杆减少（当前 Nautilus 行为）
        - 自定义模型的类路径
    config : dict, 可选
        自定义模型的额外配置参数。

    """

    model_type: str = "leveraged"
    config: dict = {}


class MarginModelFactory:
    """
    提供从配置创建保证金模型的功能。
    """

    @staticmethod
    def create(config: MarginModelConfig):
        """
        从给定配置创建保证金模型。

        参数
        ----------
        config : MarginModelConfig
            保证金模型的配置。

        返回
        -------
        MarginModel
            创建的保证金模型实例。

        抛出
        ------
        ValueError
            如果模型类型未知或无效。

        """
        from nautilus_trader.backtest.models import LeveragedMarginModel
        from nautilus_trader.backtest.models import StandardMarginModel

        model_type = config.model_type.lower()

        if model_type == "standard":
            return StandardMarginModel()
        elif model_type == "leveraged":
            return LeveragedMarginModel()
        else:
            # 尝试导入自定义模型
            try:
                from nautilus_trader.common.config import resolve_path

                model_cls = resolve_path(config.model_type)
                return model_cls(config)
            except Exception as e:
                raise ValueError(
                    f"未知的 `MarginModel` 类型 '{config.model_type}'。"
                    f"支持的类型：'standard'、'leveraged'，"
                    f"或完全限定的类路径。错误：{e}",
                ) from e
