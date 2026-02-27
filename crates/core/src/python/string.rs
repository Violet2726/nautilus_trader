// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

#![allow(clippy::doc_markdown, reason = "Python docstrings")]

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyfunction;

/// 通过仅显示前 4 位和后 4 位字符来掩码 API 密钥。
///
/// 对于长度为 8 个字符或更短的密钥，仅返回星号。
///
/// 参数 (Parameters)
/// ----------
/// api_key : str
///     待掩码的 API 密钥。
///
/// 返回 (Returns)
/// -------
/// str
///
/// 示例 (Examples)
/// --------
/// >>> mask_api_key("abcdefghijklmnop")
/// 'abcd...mnop'
/// >>> mask_api_key("short")
/// '*****'
///
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "mask_api_key")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Python FFI 要求使用拥有的 (owned) 类型"
)]
pub fn py_mask_api_key(api_key: String) -> String {
    crate::string::mask_api_key(&api_key)
}
