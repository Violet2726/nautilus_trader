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

from libc.stdint cimport uint64_t

from nautilus_trader.core.uuid cimport UUID4


cdef class Command:
    cdef readonly UUID4 id
    """指令消息 ID。\n\n:returns: `UUID4`"""
    cdef readonly uint64_t ts_init
    """对象初始化时的 UNIX 时间戳（纳秒）。\n\n:returns: `uint64_t`"""
    cdef readonly UUID4 correlation_id
    """指令关联 ID。\n\n:returns: `UUID4` 或 ``None``"""


cdef class Document:
    cdef readonly UUID4 id
    """凭证消息 ID。\n\n:returns: `UUID4`"""
    cdef readonly uint64_t ts_init
    """对象初始化时的 UNIX 时间戳（纳秒）。\n\n:returns: `uint64_t`"""


cdef class Event:
    pass


cdef class Request:
    cdef readonly UUID4 id
    """请求消息 ID。\n\n:returns: `UUID4`"""
    cdef readonly uint64_t ts_init
    """对象初始化时的 UNIX 时间戳（纳秒）。\n\n:returns: `uint64_t`"""
    cdef readonly object callback
    """响应的回调函数。\n\n:returns: `Callable`"""
    cdef readonly UUID4 correlation_id
    """请求关联 ID。\n\n:returns: `UUID4` 或 ``None``"""


cdef class Response:
    cdef readonly UUID4 id
    """响应消息 ID。\n\n:returns: `UUID4`"""
    cdef readonly uint64_t ts_init
    """对象初始化时的 UNIX 时间戳（纳秒）。\n\n:returns: `uint64_t`"""
    cdef readonly UUID4 correlation_id
    """响应关联 ID。\n\n:returns: `UUID4`"""
