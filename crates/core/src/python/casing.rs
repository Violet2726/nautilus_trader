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

//! 字符串命名格式转换辅助工具 (`CamelCase` ⇄ `snake_case`)。

use heck::ToSnakeCase;
use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyfunction;

/// 将给定的字符串从任何常见格式（PascalCase、camelCase、kebab-case 等）
/// 转换为*小写* `snake_case`。
///
/// 此函数在底层使用了 Rust 的 `heck` crate。
///
/// 参数 (Parameters)
/// ----------
/// input : str
///     待转换的输入字符串。
///
/// 返回 (Returns)
/// -------
/// str
#[must_use]
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "convert_to_snake_case")]
pub fn py_convert_to_snake_case(input: &str) -> String {
    input.to_snake_case()
}
