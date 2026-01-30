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

from decimal import Decimal
from functools import partial
from typing import TYPE_CHECKING

from ibapi.commission_and_fees_report import CommissionAndFeesReport
from ibapi.common import BarData
from ibapi.common import FaDataType
from ibapi.common import HistogramData
from ibapi.common import ListOfContractDescription
from ibapi.common import ListOfDepthExchanges
from ibapi.common import ListOfFamilyCode
from ibapi.common import ListOfHistoricalSessions
from ibapi.common import ListOfHistoricalTick
from ibapi.common import ListOfHistoricalTickBidAsk
from ibapi.common import ListOfHistoricalTickLast
from ibapi.common import ListOfNewsProviders
from ibapi.common import ListOfPriceIncrements
from ibapi.common import OrderId
from ibapi.common import SetOfFloat
from ibapi.common import SetOfString
from ibapi.common import SmartComponentMap
from ibapi.common import TickAttrib
from ibapi.common import TickAttribBidAsk
from ibapi.common import TickAttribLast
from ibapi.common import TickerId
from ibapi.contract import Contract
from ibapi.contract import ContractDetails
from ibapi.contract import DeltaNeutralContract
from ibapi.execution import Execution
from ibapi.order import Order
from ibapi.order_state import OrderState
from ibapi.ticktype import TickType
from ibapi.utils import current_fn_name
from ibapi.wrapper import EWrapper

from nautilus_trader.common.component import Logger


if TYPE_CHECKING:
    from nautilus_trader.adapters.interactive_brokers.client.client import InteractiveBrokersClient


class InteractiveBrokersEWrapper(EWrapper):
    def __init__(
        self,
        nautilus_logger: Logger,
        client: InteractiveBrokersClient,
    ) -> None:
        super().__init__()
        self._log = nautilus_logger
        self._client = client

    def logAnswer(self, fnName, fnParams) -> None:
        """
        覆盖 EWrapper.logAnswer 的日志记录。
        """
        if "self" in fnParams:
            prms = dict(fnParams)
            del prms["self"]
        else:
            prms = fnParams

        self._log.debug(f"Msg handled: function={fnName} data={prms}")

    def error(
        self,
        reqId: TickerId,
        errorTime: int,
        errorCode: int,
        errorString: str,
        advancedOrderRejectJson="",
    ) -> None:
        """
        当发生通信错误或 TWS 需要向客户端发送消息时，调用此事件。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_error,
            req_id=reqId,
            error_time=errorTime,
            error_code=errorCode,
            error_string=errorString,
            advanced_order_reject_json=advancedOrderRejectJson,
        )
        self._client.submit_to_msg_handler_queue(task)

    def winError(self, text: str, lastError: int) -> None:
        self.logAnswer(current_fn_name(), vars())

    def connectAck(self) -> None:
        """
        调用此回调以表示成功建立连接。
        """
        self.logAnswer(current_fn_name(), vars())

    def marketDataType(self, reqId: TickerId, marketDataType: int) -> None:
        """
        当市场数据类型发生变化时接收通知。

        当 TWS 向 API 发送 marketDataType(type) 回调且 type 设置为 Frozen（冻结）
        或 RealTime（实时）时，将调用此方法，以告知市场数据已在冻结和实时之间切换。
        此通知仅在市场数据在这两种状态之间切换时发生。marketDataType() 回调接受 
        reqId 参数，并为每个订阅发送一次，因为不同的合约通常遵循不同的交易时间表。

        参数
        ----------
        reqId : TickerId
            请求的标识符。
        marketDataType : int
            接收到的市场数据类型。可能的值包括：1 表示实时流式数据，2 表示冻结的市场数据。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_market_data_type,
            req_id=reqId,
            market_data_type=marketDataType,
        )
        self._client.submit_to_msg_handler_queue(task)

    def tickPrice(
        self,
        reqId: TickerId,
        tickType: TickType,
        price: float,
        attrib: TickAttrib,
    ) -> None:
        """
        市场数据行情价格（Tick Price）回调。

        参数
        ----------
        reqId : TickerId
            请求的标识符。
        tickType : TickType
            收到的行情类型。
        price : float
            行情价格。
        attrib : TickAttrib
            行情属性。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_tick_price,
            req_id=reqId,
            tick_type=tickType,
            price=price,
            attrib=attrib,
        )
        self._client.submit_to_msg_handler_queue(task)

    def tickSize(self, reqId: TickerId, tickType: TickType, size: Decimal) -> None:
        """
        处理与行情大小（Tick Size）相关的市场数据。

        此方法负责处理来自市场数据的所有与大小（成交量/深度）相关的行情。
        每个行情代表特定类型数据的市场大小变化。

        参数
        ----------
        reqId : TickerId
            请求的标识符。
        tickType : TickType
            收到的行情类型。
        size : Decimal
            行情大小。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_tick_size,
            req_id=reqId,
            tick_type=tickType,
            size=size,
        )
        self._client.submit_to_msg_handler_queue(task)

    def tickSnapshotEnd(self, reqId: int) -> None:
        """
        请求市场数据快照时，此方法将指示快照接收已完成。
        """
        self.logAnswer(current_fn_name(), vars())

    def tickGeneric(self, reqId: TickerId, tickType: TickType, value: float) -> None:
        self.logAnswer(current_fn_name(), vars())

    def tickString(self, reqId: TickerId, tickType: TickType, value: str) -> None:
        self.logAnswer(current_fn_name(), vars())

    def tickEFP(
        self,
        reqId: TickerId,
        tickType: TickType,
        basisPoints: float,
        formattedBasisPoints: str,
        totalDividends: float,
        holdDays: int,
        futureLastTradeDate: str,
        dividendImpact: float,
        dividendsToLastTradeDate: float,
    ) -> None:
        """
        现货换期货（Exchange for Physical, EFP）的市场数据回调。

        参数
        ----------
        reqId : TickerId
            请求的标识符。
        tickType : TickType
            收到的行情类型。
        basisPoints : float
            年化基点，代表可以与经纪商利率直接比较的融资利率。
        formattedBasisPoints : str
            格式化为百分比形式的年化基点字符串。
        totalDividends : float
            总股息。
        holdDays : int
            持有天数，直到 EFP 的 lastTradeDate（最后交易日）。
        futureLastTradeDate : str
            单只股票期货的到期日。
        dividendImpact : float
            股息对年化基点利率的影响。
        dividendsToLastTradeDate : float
            单只股票期货到期前预计的股息。

        """
        self.logAnswer(current_fn_name(), vars())

    def orderStatus(
        self,
        orderId: OrderId,
        status: str,
        filled: Decimal,
        remaining: Decimal,
        avgFillPrice: float,
        permId: int,
        parentId: int,
        lastFillPrice: float,
        clientId: int,
        whyHeld: str,
        mktCapPrice: float,
    ) -> None:
        """
        每当订单状态发生变化时调用此事件。此外，如果客户端有任何未平仓订单，
        在重新连接到 TWS 后也会触发此事件。

        参数
        ----------
        orderId: OrderId
            之前在调用 placeOrder() 时指定的订单 ID。
        status: str
            订单状态。可能的值包括：
            PendingSubmit, PendingCancel, PreSubmitted, Submitted, Cancelled, Filled, Inactive。
        filled: int
            指定已成交的股数。
        remaining: int
            指定尚未成交的股数。
        avgFillPrice: float
            已成交股份的平均价格。
        permId: int
            用于标识订单的 TWS ID。在不同的 TWS 会话中保持不变。
        parentId: int
            父订单的订单 ID，用于 parent 衍生订单和自动跟踪止损订单。
        lastFillPrice: float
            最后一次成交的价格。
        clientId: int
            下达该订单的客户端（或 TWS）的 ID。
        whyHeld: str
            当 TWS 正在尝试查找用于融券卖出的股票时，此字段用于标识暂持订单。
            用于表示此情况的值为 'locate'。
        mktCapPrice: float
            计算市值价格（market cap price）时的价格。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_order_status,
            order_id=orderId,
            status=status,
            filled=filled,
            remaining=remaining,
            avg_fill_price=avgFillPrice,
            perm_id=permId,
            parent_id=parentId,
            last_fill_price=lastFillPrice,
            client_id=clientId,
            why_held=whyHeld,
            mkt_cap_price=mktCapPrice,
        )
        self._client.submit_to_msg_handler_queue(task)

    def openOrder(
        self,
        orderId: OrderId,
        contract: Contract,
        order: Order,
        orderState: OrderState,
    ) -> None:
        """
        调用此函数以传入未平仓订单。

        参数
        ----------
        orderId: OrderId
            由 TWS 分配的订单 ID。用于取消或更新 TWS 订单。
        contract: Contract
            Contract 类的属性描述了合约信息。
        order: Order
            Order 类给出了未平仓订单的详情。
        orderState: OrderState
            orderState 类包含了交易前和交易后的保证金及佣金数据等属性。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_open_order,
            order_id=orderId,
            contract=contract,
            order=order,
            order_state=orderState,
        )
        self._client.submit_to_msg_handler_queue(task)

    def openOrderEnd(self) -> None:
        """
        在针对未平仓订单的给定请求结束时调用此方法。
        """
        self.logAnswer(current_fn_name(), vars())
        self._client.submit_to_msg_handler_queue(
            self._client.process_open_order_end,
        )

    def connectionClosed(self) -> None:
        """
        当 TWS 关闭与 ActiveX 控件的套接字连接，或者当 TWS 关闭时，调用此函数。
        """
        self.logAnswer(current_fn_name(), vars())
        self._client.process_connection_closed()

    def updateAccountValue(
        self,
        key: str,
        val: str,
        currency: str,
        accountName: str,
    ) -> None:
        """
        仅在 EEClientSocket 对象上调用了 ReqAccountUpdates 后，才调用此函数。
        """
        self.logAnswer(current_fn_name(), vars())

    def updatePortfolio(
        self,
        contract: Contract,
        position: Decimal,
        marketPrice: float,
        marketValue: float,
        averageCost: float,
        unrealizedPNL: float,
        realizedPNL: float,
        accountName: str,
    ) -> None:
        """
        仅在 EEClientSocket 对象上调用了 reqAccountUpdates 后，才调用此函数。
        """
        self.logAnswer(current_fn_name(), vars())

    def updateAccountTime(self, timeStamp: str) -> None:
        self.logAnswer(current_fn_name(), vars())

    def accountDownloadEnd(self, accountName: str) -> None:
        """
        在批量发送 updateAccountValue() 和 updatePortfolio() 后调用此方法。
        """
        self.logAnswer(current_fn_name(), vars())

    def nextValidId(self, orderId: int) -> None:
        """
        接收下一个有效订单 ID。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_next_valid_id,
            order_id=orderId,
        )
        self._client.submit_to_msg_handler_queue(task)

    def contractDetails(self, reqId: int, contractDetails: ContractDetails) -> None:
        """
        接收完整的合约定义。

        此方法将返回所有通过 EEClientSocket::reqContractDetails 请求匹配的合约。
        例如，可以通过它获取整个期权链。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_contract_details,
            req_id=reqId,
            contract_details=contractDetails,
        )
        self._client.submit_to_msg_handler_queue(task)

    def bondContractDetails(self, reqId: int, contractDetails: ContractDetails) -> None:
        """
        当针对债券调用了 reqContractDetails 函数时，调用此函数。
        """
        self.logAnswer(current_fn_name(), vars())

    def contractDetailsEnd(self, reqId: int) -> None:
        """
        一旦收到给定请求的所有合约详情，即调用此函数。

        这有助于定义期权链的结束。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_contract_details_end,
            req_id=reqId,
        )
        self._client.submit_to_msg_handler_queue(task)

    def execDetails(self, reqId: int, contract: Contract, execution: Execution) -> None:
        """
        当调用 reqExecutions() 函数或订单成交时，触发此事件。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_exec_details,
            req_id=reqId,
            contract=contract,
            execution=execution,
        )
        self._client.submit_to_msg_handler_queue(task)

    def execDetailsEnd(self, reqId: int) -> None:
        """
        响应 reqExecutions() 时，一旦所有成交执行都已经发送并返回，即调用此函数。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_exec_details_end,
            req_id=reqId,
        )
        self._client.submit_to_msg_handler_queue(task)

    def updateMktDepth(
        self,
        reqId: TickerId,
        position: int,
        operation: int,
        side: int,
        price: float,
        size: Decimal,
    ) -> None:
        """
        返回订单簿。

        参数
        ----------
        reqId : TickerId
            请求的标识符。
        position : int
            正在更新的订单簿行。
        operation : int
            如何刷新该行：
            - 0: insert（在 'position' 标识的行中插入此新订单）
            - 1: update（更新 'position' 标识的行中的现有订单）
            - 2: delete（删除 'position' 标识的行中的现有订单）。
        side : int
            0 表示 ask（卖出），1 表示 bid（买入）。
        price : float
            订单价格。
        size : Decimal
            订单大小。

        """
        self.logAnswer(current_fn_name(), vars())

    def updateMktDepthL2(
        self,
        reqId: TickerId,
        position: int,
        marketMaker: str,
        operation: int,
        side: int,
        price: float,
        size: Decimal,
        isSmartDepth: bool,
    ) -> None:
        """
        返回订单簿。

        参数
        ----------
        reqId : TickerId
            请求的标识符。
        position : int
            正在更新的订单簿行。
        marketMaker : str
            如果 isSmartDepth 为 True，则为持有订单的交易所；
            否则为做市商的 MPID。
        operation : int
            如何刷新该行：
            - 0: insert（在 'position' 标识的行中插入此新订单）
            - 1: update（更新 'position' 标识的行中的现有订单）
            - 2: delete（删除 'position' 标识的行中的现有订单）
        side : int
            0 表示 ask（卖出），1 表示 bid（买入）。
        price : float
            订单价格。
        size : Decimal
            订单大小。
        isSmartDepth : bool
            是否为 SMART 深度请求。

        """
        self.logAnswer(current_fn_name(), vars())

        task = partial(
            self._client.process_update_mkt_depth_l2,
            req_id=reqId,
            position=position,
            market_maker=marketMaker,
            operation=operation,
            side=side,
            price=price,
            size=size,
            is_smart_depth=isSmartDepth,
        )
        self._client.submit_to_msg_handler_queue(task)

    def updateNewsBulletin(
        self,
        msgId: int,
        msgType: int,
        newsMessage: str,
        originExch: str,
    ) -> None:
        """
        提供 IB 的公告（bulletins）。

        参数
        ----------
        msgId: int
            公告的标识符。
        msgType: int
            以下之一：
            - 1: 常规新闻公告
            - 2: 交易所不再提供交易
            - 3: 交易所可供交易
        newsMessage: str
            消息内容。
        originExch: str
            消息来源交易所。

        """
        self.logAnswer(current_fn_name(), vars())

    def managedAccounts(self, accountsList: str) -> None:
        """
        接收包含受管理账户 ID 的逗号分隔字符串。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_managed_accounts,
            accounts_list=accountsList,
        )
        self._client.submit_to_msg_handler_queue(task)

    def receiveFA(self, faData: FaDataType, cxml: str) -> None:
        """
        接收 TWS 中可用的财务顾问（Financial Advisor）配置。

        参数
        ----------
        faData : str
            以下之一：
            - Groups（组）：为交易者提供一种创建账户组并对组内所有账户应用单一分配方法的方式。
            - Account Aliases（账户别名）：让你可以通过有意义的名称而不是账号来轻松识别账户。
        cxml : str
            XML 格式的配置。

        """
        self.logAnswer(current_fn_name(), vars())

    def historicalData(self, reqId: int, bar: BarData) -> None:
        """
        返回请求的历史数据 K 线。

        参数
        ----------
        reqId : int
            请求的标识符。
        bar : BarData
            K 线数据。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_historical_data,
            req_id=reqId,
            bar=bar,
        )
        self._client.submit_to_msg_handler_queue(task)

    def historicalDataEnd(self, reqId: int, start: str, end: str) -> None:
        """
        标记历史 K 线数据接收结束。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_historical_data_end,
            req_id=reqId,
            start=start,
            end=end,
        )
        self._client.submit_to_msg_handler_queue(task)

    def scannerParameters(self, xml: str) -> None:
        """
        提供可用于创建市场扫描仪（market scanner）的 XML 格式参数。

        参数
        ----------
        xml : str
            包含可用参数的 XML 格式字符串。

        """
        self.logAnswer(current_fn_name(), vars())

    def scannerData(
        self,
        reqId: int,
        rank: int,
        contractDetails: ContractDetails,
        distance: str,
        benchmark: str,
        projection: str,
        legsStr: str,
    ) -> None:
        """
        提供市场扫描仪请求导致的数据。

        参数
        ----------
        reqId : int
            请求的标识符。
        rank : int
            此 K 线在响应中的排名。
        contractDetails : ContractDetails
            该数据的 ContractDetails。
        distance : str
            基于查询。
        benchmark : str
            基于查询。
        projection : str
            基于查询。
        legsStr : str
            当扫描仪返回 EFP 时，描述组合腿（combo legs）。

        """
        self.logAnswer(current_fn_name(), vars())

    def scannerDataEnd(self, reqId: int) -> None:
        """
        指示扫描仪数据接收已终止。

        参数
        ----------
        reqId : int
            请求的标识符。

        """
        self.logAnswer(current_fn_name(), vars())

    def realtimeBar(
        self,
        reqId: TickerId,
        time: int,
        open_: float,
        high: float,
        low: float,
        close: float,
        volume: Decimal,
        wap: Decimal,
        count: int,
    ) -> None:
        """
        更新实时 5 秒 K 线。

        参数
        ----------
        reqId : int
            请求的标识符。
        time : int
            K 线的开始时间，Unix（或 'epoch'）时间。
        open_ : float
            K 线的开盘价。
        high : float
            K 线的最高价。
        low : float
            K 线的最低价。
        close : float
            K 线的收盘价。
        volume : int
            该 K 线的交易量（如果可用）。
        wap : float
            加权平均价格（Weighted Average Price）。
        count : int
            该 K 线时段内的交易次数（仅适用于成交记录 TRADES）。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_realtime_bar,
            req_id=reqId,
            time=time,
            open_=open_,
            high=high,
            low=low,
            close=close,
            volume=volume,
            wap=wap,
            count=count,
        )
        self._client.submit_to_msg_handler_queue(task)

    def currentTime(self, time: int) -> None:
        """
        通过调用 `reqCurrentTime` 方法来获取 IB 服务器的系统时间。
        """
        self.logAnswer(current_fn_name(), vars())

    def fundamentalData(self, reqId: TickerId, data: str) -> None:
        """
        调用此函数以接收基本面市场数据（fundamental market data）。

        在尝试接收此数据之前，请确保已在账户管理（Account Management）中设置了
        相应的市场数据订阅。

        """
        self.logAnswer(current_fn_name(), vars())

    def deltaNeutralValidation(
        self,
        reqId: int,
        deltaNeutralContract: DeltaNeutralContract,
    ) -> None:
        """
        当接受 Delta 中性报价请求（Delta-Neutral RFQ）时，服务器会发送一条包含 
        DeltaNeutralContract 结构的 deltaNeutralValidation() 消息。

        如果原始请求中的 delta 和价格字段为空，确认信息将包含来自服务器的当前值。
        这些值在处理 RFQ 时被锁定，直到 RFQ 被取消。

        """
        self.logAnswer(current_fn_name(), vars())

    def commissionAndFeesReport(self, commissionAndFeesReport: CommissionAndFeesReport) -> None:
        """
        Trigger this callback in the following scenarios:

        - Immediately after a trade execution.
        - By calling reqExecutions().

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_commission_report,
            commission_report=commissionAndFeesReport,
        )
        self._client.submit_to_msg_handler_queue(task)

    def position(
        self,
        account: str,
        contract: Contract,
        position: Decimal,
        avgCost: float,
    ) -> None:
        """
        响应 reqPositions() 方法，返回所有账户的实时持仓。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_position,
            account_id=account,
            contract=contract,
            position=position,
            avg_cost=avgCost,
        )
        self._client.submit_to_msg_handler_queue(task)

    def positionEnd(self) -> None:
        """
        一旦收到给定请求的所有持仓数据即调用此方法，作为 position() 数据的结束标记。
        """
        self.logAnswer(current_fn_name(), vars())
        self._client.submit_to_msg_handler_queue(
            self._client.process_position_end,
        )

    def accountSummary(
        self,
        reqId: int,
        account: str,
        tag: str,
        value: str,
        currency: str,
    ) -> None:
        """
        响应 reqAccountSummary()，返回 TWS 账户窗口“Summary”（摘要）选项卡中的数据。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_account_summary,
            req_id=reqId,
            account_id=account,
            tag=tag,
            value=value,
            currency=currency,
        )
        self._client.submit_to_msg_handler_queue(task)

    def accountSummaryEnd(self, reqId: int) -> None:
        """
        当收到了给定请求的所有账户摘要数据时，调用此方法。
        """
        self.logAnswer(current_fn_name(), vars())

    def verifyCompleted(self, isSuccessful: bool, errorText: str) -> None:
        self.logAnswer(current_fn_name(), vars())

    def verifyAndAuthMessageAPI(self, apiData: str, xyzChallenge: str) -> None:
        self.logAnswer(current_fn_name(), vars())

    def verifyAndAuthCompleted(self, isSuccessful: bool, errorText: str) -> None:
        self.logAnswer(current_fn_name(), vars())

    def displayGroupList(self, reqId: int, groups: str) -> None:
        """
        接收对 queryDisplayGroups() 的一次性响应回调。

        参数
        ----------
        reqId : int
            在 queryDisplayGroups() 中指定的请求 ID。
        groups : str
            以 '|' 字符分隔的可见组 ID 列表，按最常用组排序。
            此列表在 TWS 会话期间保持不变（即用户无法添加新组；但排序可能会变）。

        """
        self.logAnswer(current_fn_name(), vars())

    def displayGroupUpdated(self, reqId: int, contractInfo: str) -> None:
        """
        通过 subscribeToGroupEvents() 订阅组事件后，接收从 TWS 发送到 API 客户端的通知。
        如果订阅的显示组中选择的合约发生变化，将重新发送此通知。

        参数
        ----------
        reqId : int
            在 subscribeToGroupEvents() 中指定的请求 ID。
        contractInfo : str
            在 IB 中唯一表示该合约的编码值。可能的值包括：
            - 'none': 未选择。
            - 'contractID@exchange': 适用于任何非组合合约。
                                     示例：IBM SMART 为 '8314@SMART'；IBM @ARCA 为 '8314@ARCA'。
            - 'combo': 如果选择了任何组合合约。

        """
        self.logAnswer(current_fn_name(), vars())

    def positionMulti(
        self,
        reqId: int,
        account: str,
        modelCode: str,
        contract: Contract,
        pos: Decimal,
        avgCost: float,
    ) -> None:
        """
        检索特定账户或模型的持仓，类似于 position() 函数。
        """
        self.logAnswer(current_fn_name(), vars())

    def positionMultiEnd(self, reqId: int) -> None:
        """
        终止特定账户或模型的持仓接收，类似于 positionEnd() 函数。
        """
        self.logAnswer(current_fn_name(), vars())

    def accountUpdateMulti(
        self,
        reqId: int,
        account: str,
        modelCode: str,
        key: str,
        value: str,
        currency: str,
    ) -> None:
        """
        更新特定账户或模型的值，类似于 updateAccountValue() 函数。
        """
        self.logAnswer(current_fn_name(), vars())

    def accountUpdateMultiEnd(self, reqId: int) -> None:
        """
        下载特定账户或模型的数据，类似于 accountDownloadEnd() 的功能。
        """
        self.logAnswer(current_fn_name(), vars())

    def tickOptionComputation(
        self,
        reqId: TickerId,
        tickType: TickType,
        tickAttrib: int,
        impliedVol: float,
        delta: float,
        optPrice: float,
        pvDividend: float,
        gamma: float,
        vega: float,
        theta: float,
        undPrice: float,
    ) -> None:
        """
        当下达期权或其标的资产的市场变动做出响应时，调用此函数。

        接收 TWS 的期权模型波动率、价格和 Delta 值，以及期权标的资产预期的股息现值。

        """
        self.logAnswer(current_fn_name(), vars())

    def securityDefinitionOptionParameter(
        self,
        reqId: int,
        exchange: str,
        underlyingConId: int,
        tradingClass: str,
        multiplier: str,
        expirations: SetOfString,
        strikes: SetOfFloat,
    ) -> None:
        """
        返回某个交易所在特定标的上的期权链。

        这是由调用 `reqSecDefOptParams` 触发的。如果在 `reqSecDefOptParams` 中
        指定了多个交易所，则会有多次 `securityDefinitionOptionParameter` 回调。

        参数
        ----------
        reqId : int
            启动回调的请求 ID。
        exchange : str
            请求期权链的交易所。
        underlyingConId : int
            标的证券的 conID。
        tradingClass : str
            期权交易分类。
        multiplier : str
            期权乘数。
        expirations : list[str]
            该交易所在该标的上的期权到期日列表。
        strikes : list[float]
            该交易所在该标的上的期权可能行权价列表。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_security_definition_option_parameter,
            req_id=reqId,
            exchange=exchange,
            underlying_con_id=underlyingConId,
            trading_class=tradingClass,
            multiplier=multiplier,
            expirations=expirations,
            strikes=strikes,
        )
        self._client.submit_to_msg_handler_queue(task)

    def securityDefinitionOptionParameterEnd(self, reqId: int) -> None:
        """
        在所有 securityDefinitionOptionParameter 回调完成后调用。

        参数
        ----------
        reqId : int
            初始调用 `securityDefinitionOptionParameter` 时使用的 ID。

        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_security_definition_option_parameter_end,
            req_id=reqId,
        )
        self._client.submit_to_msg_handler_queue(task)

    def softDollarTiers(self, reqId: int, tiers: list) -> None:
        """
        在收到软美元层级（Soft Dollar Tier）配置信息时调用。

        参数
        ----------
        reqId : int
            在调用 `EEClient::reqSoftDollarTiers` 时使用的请求 ID。
        tiers : list[SoftDollarTier]
            包含所有软美元层级信息的列表。

        """
        self.logAnswer(current_fn_name(), vars())

    def familyCodes(self, familyCodes: ListOfFamilyCode) -> None:
        """
        返回家族代码（family codes）数组。
        """
        self.logAnswer(current_fn_name(), vars())

    def symbolSamples(
        self,
        reqId: int,
        contractDescriptions: ListOfContractDescription,
    ) -> None:
        """
        返回样本合约描述数组。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_symbol_samples,
            req_id=reqId,
            contract_descriptions=contractDescriptions,
        )
        self._client.submit_to_msg_handler_queue(task)

    def mktDepthExchanges(self, depthMktDataDescriptions: ListOfDepthExchanges) -> None:
        """
        返回为 UpdateMktDepthL2 提供深度数据的交易所数组。
        """
        self.logAnswer(current_fn_name(), vars())

    def tickNews(
        self,
        tickerId: int,
        timeStamp: int,
        providerCode: str,
        articleId: str,
        headline: str,
        extraData: str,
    ) -> None:
        """
        返回新闻标题。
        """
        self.logAnswer(current_fn_name(), vars())

    def smartComponents(self, reqId: int, smartComponentMap: SmartComponentMap) -> None:
        """
        返回交易所组件映射。
        """
        self.logAnswer(current_fn_name(), vars())

    def tickReqParams(
        self,
        tickerId: int,
        minTick: float,
        bboExchange: str,
        snapshotPermissions: int,
    ) -> None:
        """
        返回特定合约的交易参数（Exchange map）。
        """
        self.logAnswer(current_fn_name(), vars())

    def newsProviders(self, newsProviders: ListOfNewsProviders) -> None:
        """
        返回可用且已订阅的 API 新闻提供商。
        """
        self.logAnswer(current_fn_name(), vars())

    def newsArticle(self, requestId: int, articleType: int, articleText: str) -> None:
        """
        返回新闻文章的正文。
        """
        self.logAnswer(current_fn_name(), vars())

    def historicalNews(
        self,
        requestId: int,
        time: str,
        providerCode: str,
        articleId: str,
        headline: str,
    ) -> None:
        """
        返回历史新闻标题。
        """
        self.logAnswer(current_fn_name(), vars())

    def historicalNewsEnd(self, requestId: int, hasMore: bool) -> None:
        """
        表示历史新闻结束。
        """
        self.logAnswer(current_fn_name(), vars())

    def headTimestamp(self, reqId: int, headTimestamp: str) -> None:
        """
        返回给定合约特定类型数据的最早可用时间戳。
        """
        self.logAnswer(current_fn_name(), vars())

    def histogramData(self, reqId: int, items: HistogramData) -> None:
        """
        返回合约的柱状图（histogram）数据。
        """
        self.logAnswer(current_fn_name(), vars())

    def historicalDataUpdate(self, reqId: int, bar: BarData) -> None:
        """
        当 keepUpToDate 设置为 True 时，返回实时更新。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_historical_data_update,
            req_id=reqId,
            bar=bar,
        )
        self._client.submit_to_msg_handler_queue(task)

    def rerouteMktDataReq(self, reqId: int, conId: int, exchange: str) -> None:
        """
        返回市场数据请求在重新路由后的 CFD 合约信息。
        """
        self.logAnswer(current_fn_name(), vars())

    def rerouteMktDepthReq(self, reqId: int, conId: int, exchange: str) -> None:
        """
        返回市场深度请求在重新路由后的 CFD 合约信息。
        """
        self.logAnswer(current_fn_name(), vars())

    def marketRule(self, marketRuleId: int, priceIncrements: ListOfPriceIncrements) -> None:
        """
        返回特定市场规则 ID 的最小价格变动结构。
        """
        self.logAnswer(current_fn_name(), vars())

    def pnl(self, reqId: int, dailyPnL: float, unrealizedPnL: float, realizedPnL: float) -> None:
        """
        返回账户的当日盈亏 (PnL)。
        """
        self.logAnswer(current_fn_name(), vars())

    def pnlSingle(
        self,
        reqId: int,
        pos: Decimal,
        dailyPnL: float,
        unrealizedPnL: float,
        realizedPnL: float,
        value: float,
    ) -> None:
        """
        返回账户中单个持仓的当日盈亏 (PnL)。
        """
        self.logAnswer(current_fn_name(), vars())

    def historicalTicks(self, reqId: int, ticks: ListOfHistoricalTick, done: bool) -> None:
        """
        当 whatToShow 设置为 MIDPOINT 时，返回历史逐笔成交（tick）数据。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_historical_ticks,
            req_id=reqId,
            ticks=ticks,
            done=done,
        )
        self._client.submit_to_msg_handler_queue(task)

    def historicalTicksBidAsk(
        self,
        reqId: int,
        ticks: ListOfHistoricalTickBidAsk,
        done: bool,
    ) -> None:
        """
        当 whatToShow 设置为 BID_ASK 时，返回历史逐笔成交（tick）数据。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_historical_ticks_bid_ask,
            req_id=reqId,
            ticks=ticks,
            done=done,
        )
        self._client.submit_to_msg_handler_queue(task)

    def historicalTicksLast(self, reqId: int, ticks: ListOfHistoricalTickLast, done: bool) -> None:
        """
        当 whatToShow 设置为 TRADES 时，返回历史逐笔成交（tick）数据。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_historical_ticks_last,
            req_id=reqId,
            ticks=ticks,
            done=done,
        )
        self._client.submit_to_msg_handler_queue(task)

    def tickByTickAllLast(
        self,
        reqId: int,
        tickType: int,
        time: int,
        price: float,
        size: Decimal,
        tickAttribLast: TickAttribLast,
        exchange: str,
        specialConditions: str,
    ) -> None:
        """
        当 tickType 设置为 "Last" 或 "AllLast" 时，返回逐笔成交数据。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_tick_by_tick_all_last,
            req_id=reqId,
            tick_type=tickType,
            time=time,
            price=price,
            size=size,
            tick_attrib_last=tickAttribLast,
            exchange=exchange,
            special_conditions=specialConditions,
        )
        self._client.submit_to_msg_handler_queue(task)

    def tickByTickBidAsk(
        self,
        reqId: int,
        time: int,
        bidPrice: float,
        askPrice: float,
        bidSize: Decimal,
        askSize: Decimal,
        tickAttribBidAsk: TickAttribBidAsk,
    ) -> None:
        """
        当 tickType 设置为 "BidAsk" 时，返回逐笔成交数据。
        """
        self.logAnswer(current_fn_name(), vars())
        task = partial(
            self._client.process_tick_by_tick_bid_ask,
            req_id=reqId,
            time=time,
            bid_price=bidPrice,
            ask_price=askPrice,
            bid_size=bidSize,
            ask_size=askSize,
            tick_attrib_bid_ask=tickAttribBidAsk,
        )
        self._client.submit_to_msg_handler_queue(task)

    def tickByTickMidPoint(self, reqId: int, time: int, midPoint: float) -> None:
        """
        当 tickType 设置为 "MidPoint" 时，返回逐笔成交数据。
        """
        self.logAnswer(current_fn_name(), vars())

    def orderBound(self, permId: int, clientId: int, orderId: int) -> None:
        """
        返回 orderBound 通知。
        """
        self.logAnswer(current_fn_name(), vars())

    def completedOrder(self, contract: Contract, order: Order, orderState: OrderState) -> None:
        """
        传入已完成订单。

        参数
        ----------
        contract : Contract
            使用 Contract 类的属性描述合约。
        order : Order
            由 Order 类定义的已完成订单详情。
        orderState : OrderState
            包含 OrderState 类中指定的已完成订单状态详情。

        """
        self.logAnswer(current_fn_name(), vars())

    def completedOrdersEnd(self) -> None:
        """
        在完成已完成订单的请求时调用。
        """
        self.logAnswer(current_fn_name(), vars())

    def replaceFAEnd(self, reqId: int, text: str) -> None:
        """
        在财务顾问 (FA) 替换操作完成时调用。
        """
        self.logAnswer(current_fn_name(), vars())

    def wshMetaData(self, reqId: int, dataJson: str) -> None:
        self.logAnswer(current_fn_name(), vars())

    def wshEventData(self, reqId: int, dataJson: str) -> None:
        self.logAnswer(current_fn_name(), vars())

    def historicalSchedule(
        self,
        reqId: int,
        startDateTime: str,
        endDateTime: str,
        timeZone: str,
        sessions: ListOfHistoricalSessions,
    ) -> None:
        """
        当 whatToShow=SCHEDULE 时，返回历史数据请求的历史时间表。
        """
        self.logAnswer(current_fn_name(), vars())

    def userInfo(self, reqId: int, whiteBrandingId: str) -> None:
        """
        返回用户信息。
        """
        self.logAnswer(current_fn_name(), vars())
