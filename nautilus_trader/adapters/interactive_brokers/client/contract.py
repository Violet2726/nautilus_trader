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

import functools
from typing import Any

from ibapi.common import SetOfFloat
from ibapi.common import SetOfString
from ibapi.contract import ContractDetails

from nautilus_trader.adapters.interactive_brokers.client.common import BaseMixin
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.common import IBContractDetails


class InteractiveBrokersClientContractMixin(BaseMixin):
    """
    处理 InteractiveBrokersClient 的合约（证券/资产）。

    此类提供请求合约详情、匹配合约以及期权链的方法。
    InteractiveBrokersInstrumentProvider 类使用此类中定义的方法来请求其需要的
    数据。

    """

    async def get_contract_details(self, contract: IBContract) -> list[IBContractDetails] | None:
        """
        请求特定合约的详情。

        参数
        ----------
        contract : IBContract
            请求详情的合约。

        返回
        -------
        IBContractDetails | ``None``

        """
        name = str(contract)

        if not (request := self._requests.get(name=name)):
            req_id = self._next_req_id()
            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=functools.partial(
                    self._eclient.reqContractDetails,
                    reqId=req_id,
                    contract=contract,
                ),
            )

            if not request:
                return None

            request.handle()

            return await self._await_request(request, 10, suppress_timeout_warning=True)
        else:
            return await self._await_request(request, 10, suppress_timeout_warning=True)

    async def get_matching_contracts(self, pattern: str) -> list[IBContract] | None:
        """
        请求与特定模式匹配的合约。

        参数
        ----------
        pattern : str
            用于匹配合约代码（symbol）的模式。

        返回
        -------
        list[IBContract] | ``None``

        """
        name = f"MatchingSymbols-{pattern}"

        if not (request := self._requests.get(name=name)):
            req_id = self._next_req_id()
            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=functools.partial(
                    self._eclient.reqMatchingSymbols,
                    reqId=req_id,
                    pattern=pattern,
                ),
            )

            if not request:
                return None

            request.handle()

            return await self._await_request(request, 20)
        else:
            self._log.info(f"请求已存在于 {request}")
            return None

    async def get_option_chains(self, underlying: IBContract) -> Any | None:
        """
        请求特定标的合约的期权链。

        参数
        ----------
        underlying : IBContract
            请求其期权链的标的合约。

        返回
        -------
        list[IBContractDetails] | ``None``

        """
        name = f"OptionChains-{underlying!s}"

        if not (request := self._requests.get(name=name)):
            req_id = self._next_req_id()
            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=functools.partial(
                    self._eclient.reqSecDefOptParams,
                    reqId=req_id,
                    underlyingSymbol=underlying.symbol,
                    futFopExchange=underlying.exchange if underlying.secType == "FUT" else "",
                    underlyingSecType=underlying.secType,
                    underlyingConId=underlying.conId,
                ),
            )

            if not request:
                return None

            request.handle()

            return await self._await_request(request, 20)
        else:
            self._log.info(f"请求已存在于 {request}")
            return None

    async def process_contract_details(
        self,
        *,
        req_id: int,
        contract_details: ContractDetails,
    ) -> None:
        """
        接收完整的合约定义。此方法将返回所有通过 EClientSocket::reqContractDetails 
        请求匹配的合约。例如，可以通过它获取整个期权链。
        """
        if not (request := self._requests.get(req_id=req_id)):
            return

        request.result.append(contract_details)

    async def process_contract_details_end(self, *, req_id: int) -> None:
        """
        在所有与请求匹配的合约都返回后，此方法将标记接收结束。
        """
        self._end_request(req_id)

    async def process_security_definition_option_parameter(
        self,
        *,
        req_id: int,
        exchange: str,
        underlying_con_id: int,
        trading_class: str,
        multiplier: str,
        expirations: SetOfString,
        strikes: SetOfFloat,
    ) -> None:
        """
        由于 reqSecDefOptParams 请求，返回某个交易所在特定标的上的期权链。
        如果在 reqSecDefOptParams 中指定了多个交易所，则会有多次 
        securityDefinitionOptionParameter 回调。
        """
        if request := self._requests.get(req_id=req_id):
            request.result.append((exchange, expirations))

    async def process_security_definition_option_parameter_end(self, *, req_id: int) -> None:
        """
        当所有 securityDefinitionOptionParameter 回调完成时调用。
        """
        self._end_request(req_id)

    async def process_symbol_samples(
        self,
        *,
        req_id: int,
        contract_descriptions: list,
    ) -> None:
        """
        返回一个样本合约描述数组。
        """
        if request := self._requests.get(req_id=req_id):
            for contract_description in contract_descriptions:
                request.result.append(IBContract(**contract_description.contract.__dict__))

            self._end_request(req_id)
