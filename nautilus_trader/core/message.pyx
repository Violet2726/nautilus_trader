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

from typing import Any
from typing import Callable

import cython

from nautilus_trader.core.uuid cimport UUID4


cdef class Command:
    """
    所有指令消息（Command message）的基类。

    参数
    ----------
    command_id : UUID4
        指令 ID。
    ts_init : uint64_t
        对象初始化时的 UNIX 时间戳（纳秒）。
    correlation_id : UUID4, 可选
        关联 ID。如果提供，此指令将与其他指令或请求相关联。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。
    """

    def __init__(
        self,
        UUID4 command_id not None,
        uint64_t ts_init,
        UUID4 correlation_id = None,
    ):
        self.id = command_id
        self.ts_init = ts_init
        self.correlation_id = correlation_id

    def __getstate__(self):
        return (
            self.id.to_str(),
            self.ts_init,
            self.correlation_id.to_str() if self.correlation_id is not None else None,
        )

    def __setstate__(self, state):
        self.id = UUID4.from_str_c(state[0])
        self.ts_init = state[1]
        self.correlation_id = UUID4.from_str_c(state[2]) if state[2] is not None else None

    def __eq__(self, Command other) -> bool:
        if other is None:
            return False
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        correlation_str = f", correlation_id={self.correlation_id}" if self.correlation_id is not None else ""
        return f"{type(self).__name__}(id={self.id}, ts_init={self.ts_init}{correlation_str})"


cdef class Document:
    """
    所有凭证消息（Document message）的基类。

    参数
    ----------
    document_id : UUID4
        指令 ID。
    ts_init : uint64_t
        对象初始化时的 UNIX 时间戳（纳秒）。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。
    """

    def __init__(
        self,
        UUID4 document_id not None,
        uint64_t ts_init,
    ):
        self.id = document_id
        self.ts_init = ts_init

    def __getstate__(self):
        return (
            self.id.to_str(),
            self.ts_init,
        )

    def __setstate__(self, state):
        self.id = UUID4.from_str_c(state[0])
        self.ts_init = state[1]

    def __eq__(self, Document other) -> bool:
        if other is None:
            return False
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        return f"{type(self).__name__}(id={self.id}, ts_init={self.ts_init})"


@cython.auto_pickle(False)
cdef class Event:
    """
    所有事件消息（Event message）的抽象基类。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。
    """

    @property
    def id(self) -> UUID4:
        """
        事件消息标识符。

        返回
        -------
        UUID4

        """
        raise NotImplementedError("必须实现抽象属性")

    @property
    def ts_event(self) -> int:
        """
        事件发生时的 UNIX 时间戳（纳秒）。

        返回
        -------
        int

        """
        raise NotImplementedError("必须实现抽象属性")

    @property
    def ts_init(self) -> int:
        """
        对象初始化时的 UNIX 时间戳（纳秒）。

        返回
        -------
        int

        """
        raise NotImplementedError("必须实现抽象属性")


cdef class Request:
    """
    所有请求消息（Request message）的基类。

    参数
    ----------
    callback : Callable[[Any], None]
        用于接收响应的回调委托。
    request_id : UUID4
        请求 ID。
    ts_init : uint64_t
        对象初始化时的 UNIX 时间戳（纳秒）。
    correlation_id : UUID4, 可选
        关联 ID。如果提供，此请求将与其他请求相关联。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。
    """

    def __init__(
        self,
        callback: Callable[[Any], None] | None,
        UUID4 request_id not None,
        uint64_t ts_init,
        UUID4 correlation_id = None,
    ):
        self.callback = callback
        self.id = request_id
        self.ts_init = ts_init
        self.correlation_id = correlation_id

    def __getstate__(self):
        return (
            self.callback,
            self.id.to_str(),
            self.ts_init,
            self.correlation_id.to_str() if self.correlation_id is not None else None,
        )

    def __setstate__(self, state):
        self.callback = state[0]
        self.id = UUID4.from_str_c(state[1])
        self.ts_init = state[2]
        self.correlation_id = UUID4.from_str_c(state[3]) if state[3] is not None else None

    def __eq__(self, Request other) -> bool:
        if other is None:
            return False
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        correlation_str = f", correlation_id={self.correlation_id}" if self.correlation_id is not None else ""

        return f"{type(self).__name__}(id={self.id}, callback={self.callback}, ts_init={self.ts_init}{correlation_str})"


cdef class Response:
    """
    所有响应消息（Response message）的基类。

    参数
    ----------
    correlation_id : UUID4
        关联 ID。
    response_id : UUID4
        响应 ID。
    ts_init : uint64_t
        对象初始化时的 UNIX 时间戳（纳秒）。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。
    """

    def __init__(
        self,
        UUID4 correlation_id not None,
        UUID4 response_id not None,
        uint64_t ts_init,
    ):
        self.correlation_id = correlation_id
        self.id = response_id
        self.ts_init = ts_init

    def __getstate__(self):
        return (
            self.correlation_id.to_str(),
            self.id.to_str(),
            self.ts_init,
        )

    def __setstate__(self, state):
        self.correlation_id = UUID4.from_str_c(state[0])
        self.id = UUID4.from_str_c(state[1])
        self.ts_init = state[2]

    def __eq__(self, Response other) -> bool:
        if other is None:
            return False
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        return (
            f"{type(self).__name__}("
            f"correlation_id={self.correlation_id}, "
            f"id={self.id}, "
            f"ts_init={self.ts_init})"
        )
