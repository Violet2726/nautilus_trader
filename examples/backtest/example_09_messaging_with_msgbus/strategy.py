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

from dataclasses import dataclass

from nautilus_trader.common.enums import LogColor
from nautilus_trader.config import StrategyConfig
from nautilus_trader.core.datetime import unix_nanos_to_dt
from nautilus_trader.core.message import Event
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.trading.strategy import Strategy


@dataclass
class Each10thBarEvent(Event):
    """
    每隔 10 个 Bar 发布一次的自定义事件。

    通过继承 `Event` 类，我们会自动获得一些重要属性：
     - `id`: 每个事件的唯一字符串标识符（UUID 格式）
     - `ts_event`: 事件发生时的时间戳（用于事件排序）
     - `ts_init`: 事件初始化时的时间戳

    这些属性对于消息总线（message bus）中的正确事件处理和排序至关重要，
    特别是在事件时序非常重要的回测过程中。

    Event 类在属性方面提供了完全的灵活性：
    - 可以包含任何 Python 类型的属性（int、float、str、自定义对象等）

    """

    bar: Bar  # 与此事件相关的第 10 个 Bar
    TOPIC: str = "each_10th_bar_event"  # 消息总线发布/订阅的主题名称


@dataclass
class DemoStrategyConfig(StrategyConfig, frozen=True):
    """
    演示策略的配置。
    """

    instrument: Instrument
    bar_type: BarType


class DemoStrategy(Strategy):
    """
    演示如何使用自定义事件和消息总线的策略。
    """

    def __init__(self, config: DemoStrategyConfig):
        super().__init__(config)

        # 已处理 Bar 的计数器
        self.bars_processed = 0

    def on_start(self):
        # 订阅市场数据
        self.subscribe_bars(self.config.bar_type)
        self.log.info(f"Subscribed to {self.config.bar_type}", color=LogColor.YELLOW)

        # 消息总线实现了基于主题的发布/订阅模式：
        # - 发布者可以将事件发布到一个或多个命名主题
        # - 订阅者可以订阅一个或多个感兴趣的主题

        # 订阅我们的自定义事件
        # 第一个参数是要订阅的主题名称，第二个是自定义处理方法
        self.msgbus.subscribe(Each10thBarEvent.TOPIC, self.on_each_10th_bar)
        self.log.info(f"Subscribed to {Each10thBarEvent.TOPIC}", color=LogColor.YELLOW)

    def on_bar(self, bar: Bar):
        # 统计处理过的 Bar
        self.bars_processed += 1
        self.log.info(
            f"Bar #{self.bars_processed} | Bar: {bar} | Time={unix_nanos_to_dt(bar.ts_event)}",
        )

        # 每隔 10 个 Bar，发布一次我们的自定义事件
        if self.bars_processed % 10 == 0:
            # 记录我们的计划
            self.log.info(
                f"Going to publish event for topic: {Each10thBarEvent.TOPIC}",
                color=LogColor.GREEN,
            )

            # 创建并发布事件
            # 这演示了如何使用消息总线发送事件
            event = Each10thBarEvent(bar=bar)
            self.msgbus.publish(Each10thBarEvent.TOPIC, event)

    def on_each_10th_bar(self, event: Each10thBarEvent):
        """
        处理从消息总线接收到的每 10 个 Bar 的事件。
        """
        # 记录事件详情
        self.log.info(
            f"Received event for topic: {Each10thBarEvent.TOPIC} at bar # {self.bars_processed}| "
            f"Bar detail: {event.bar}",
            color=LogColor.RED,
        )

    def on_stop(self):
        self.log.info(f"Strategy stopped. Processed {self.bars_processed} bars.")
