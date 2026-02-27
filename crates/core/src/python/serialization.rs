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

//! 连接 Rust ↔︎ Python 类型的（反）序列化工具。

use pyo3::{prelude::*, types::PyDict};
use serde::{Serialize, de::DeserializeOwned};

use crate::python::to_pyvalue_err;

/// 将 Python 字典转换为实现了 `DeserializeOwned` 的 Rust 类型。
///
/// # Errors
///
/// 如果发生以下情况，则返回错误：
/// - Python 字典无法被序列化为 JSON。
/// - JSON 字符串无法被反序列化为类型 `T`。
/// - Python 的 `json` 模块导入或执行失败。
pub fn from_dict_pyo3<T>(py: Python<'_>, values: Py<PyDict>) -> Result<T, PyErr>
where
    T: DeserializeOwned,
{
    // 提取为 JSON 字节
    let json_str: String = PyModule::import(py, "json")?
        .call_method("dumps", (values,), None)?
        .extract()?;

    // 反序列化为对象
    let instance = serde_json::from_str(&json_str).map_err(to_pyvalue_err)?;
    Ok(instance)
}

/// 将实现了 `Serialize` 的 Rust 类型转换为 Python 字典。
///
/// # Errors
///
/// 如果发生以下情况，则返回错误：
/// - Rust 值无法被序列化为 JSON。
/// - JSON 字符串无法被解析为 Python 字典。
/// - Python 的 `json` 模块导入或执行失败。
pub fn to_dict_pyo3<T>(py: Python<'_>, value: &T) -> PyResult<Py<PyDict>>
where
    T: Serialize,
{
    let json_str = serde_json::to_string(value).map_err(to_pyvalue_err)?;

    // 将 JSON 解析为 Python 字典
    let py_dict: Py<PyDict> = PyModule::import(py, "json")?
        .call_method("loads", (json_str,), None)?
        .extract()?;
    Ok(py_dict)
}
