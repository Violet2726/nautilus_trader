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

//! 为 PyO3 提供的宏生成枚举工具。

use ::strum::{IntoEnumIterator, ParseError};
use pyo3::PyResult;

use super::to_pyvalue_err;

/// 将原始字符串转换为枚举 `E`；如果字符串与任何变体都不匹配，
/// 则返回一个格式美观的 `PyValueError`。
///
/// 此辅助程序旨在用于仍在使用普通 `&str` 参数的 Python 模块暴露出的函数：
/// 请调用 `parse_enum`，而不是自行编写冗余的 `str::parse()` 加上错误格式化逻辑。
///
/// # Errors
///
/// 如果 `input` 与枚举 `E` 的任何已知变体都不匹配，则返回错误。
pub fn parse_enum<E>(input: &str, param: &str) -> PyResult<E>
where
    E: std::str::FromStr<Err = ParseError> + IntoEnumIterator + ToString,
{
    input.parse::<E>().map_err(|_| {
        let allowed = E::iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        to_pyvalue_err(format!(
            "未知 {param} `{input}`；有效值为: {allowed}"
        ))
    })
}
