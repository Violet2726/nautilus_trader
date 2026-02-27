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

//! 为 Python 输入提供的 JSON / 字符串解析辅助工具。

use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

use super::{to_pykey_err, to_pyvalue_err};

/// 从 Python 字典中获取必填字符串值的辅助函数。
///
/// # 返回 (Returns)
///
/// 返回提取到的字符串值；如果键缺失或提取失败，则返回 `PyErr`。
///
/// # 错误 (Errors)
///
/// 如果键缺失或值提取失败，则返回 `PyErr`。
pub fn get_required_string(dict: &Bound<'_, PyDict>, key: &str) -> PyResult<String> {
    dict.get_item(key)?
        .ok_or_else(|| to_pykey_err(format!("缺失必填键: {key}")))?
        .extract()
}

/// 从 Python 字典中获取必填值并将其提取的辅助函数。
///
/// # 返回 (Returns)
///
/// 返回提取到的值；如果键缺失或提取失败，则返回 `PyErr`。
///
/// # 错误 (Errors)
///
/// 如果键缺失或值提取失败，则返回 `PyErr`。
pub fn get_required<T>(dict: &Bound<'_, PyDict>, key: &str) -> PyResult<T>
where
    T: for<'a, 'py> FromPyObject<'a, 'py>,
    for<'a, 'py> PyErr: From<<T as FromPyObject<'a, 'py>>::Error>,
{
    dict.get_item(key)?
        .ok_or_else(|| to_pykey_err(format!("缺失必填键: {key}")))?
        .extract()
        .map_err(PyErr::from)
}

/// 从 Python 字典中获取可选值的辅助函数。
///
/// # 返回 (Returns)
///
/// 如果键存在且提取成功，返回 Some(value)；
/// 如果键缺失或值为 Python None，返回 None；
/// 如果提取失败，返回 `PyErr`。
///
/// # 错误 (Errors)
///
/// 如果值提取失败（但在键缺失或值为 None 的情况下不会），则返回 `PyErr`。
pub fn get_optional<T>(dict: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<T>>
where
    T: for<'a, 'py> FromPyObject<'a, 'py>,
    for<'a, 'py> PyErr: From<<T as FromPyObject<'a, 'py>>::Error>,
{
    match dict.get_item(key)? {
        Some(value) => {
            if value.is_none() {
                Ok(None)
            } else {
                value.extract().map(Some).map_err(PyErr::from)
            }
        }
        None => Ok(None),
    }
}

/// 获取必填值、使用闭包进行解析并处理解析错误的辅助函数。
///
/// # 返回 (Returns)
///
/// 返回解析后的值；如果键缺失、提取失败或解析失败，则返回 `PyErr`。
///
/// # 错误 (Errors)
///
/// 如果键缺失、值提取失败或解析失败，则返回 `PyErr`。
pub fn get_required_parsed<T, F>(dict: &Bound<'_, PyDict>, key: &str, parser: F) -> PyResult<T>
where
    F: FnOnce(String) -> Result<T, String>,
{
    let value_str = get_required_string(dict, key)?;
    parser(value_str).map_err(|e| to_pyvalue_err(format!("未能解析 '{key}': {e}")))
}

/// 获取可选值、使用闭包进行解析并处理解析错误的辅助函数。
///
/// # 返回 (Returns)
///
/// 如果键存在且解析成功，返回 `Some(parsed_value)`；
/// 如果键缺失或值为 Python None，返回 None；
/// 如果提取或解析失败，返回 `PyErr`。
///
/// # 错误 (Errors)
///
/// 如果值提取或解析失败（但在键缺失或值为 None 的情况下不会），则返回 `PyErr`。
pub fn get_optional_parsed<T, F>(
    dict: &Bound<'_, PyDict>,
    key: &str,
    parser: F,
) -> PyResult<Option<T>>
where
    F: FnOnce(String) -> Result<T, String>,
{
    match dict.get_item(key)? {
        Some(value) => {
            if value.is_none() {
                Ok(None)
            } else {
                let value_str: String = value.extract()?;
                parser(value_str)
                    .map(Some)
                    .map_err(|e| to_pyvalue_err(format!("未能解析 '{key}': {e}")))
            }
        }
        None => Ok(None),
    }
}

/// 从 Python 字典中获取必填 `PyList` 的辅助函数。
///
/// # 返回 (Returns)
///
/// 返回提取到的 `PyList`；如果键缺失或提取失败，则返回 `PyErr`。
///
/// # 错误 (Errors)
///
/// 如果键缺失或值提取失败，则返回 `PyErr`。
pub fn get_required_list<'py>(
    dict: &Bound<'py, PyDict>,
    key: &str,
) -> PyResult<Bound<'py, PyList>> {
    dict.get_item(key)?
        .ok_or_else(|| to_pykey_err(format!("缺失必填键: {key}")))?
        .downcast_into()
        .map_err(Into::into)
}
