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

//! 为 [`Params`] 类型转换提供的 Python 绑定。

use indexmap::IndexMap;
use pyo3::{
    conversion::IntoPyObjectExt,
    prelude::*,
    types::{PyDict, PyList, PyModule},
};
use serde_json::Value;

use crate::{params::Params, python::to_pyvalue_err};

/// 将 Python 字典转换为 `Params` (IndexMap<String, Value>)。
///
/// # Errors
///
/// 如果发生以下情况，则返回 `PyErr`：
/// - 该字典无法被序列化为 JSON
/// - 该 JSON 不是一个有效的对象
pub fn pydict_to_params(py: Python<'_>, dict: Py<PyDict>) -> PyResult<Option<Params>> {
    let dict_bound = dict.bind(py);
    if dict_bound.is_empty() {
        return Ok(None);
    }

    let json_str: String = PyModule::import(py, "json")?
        .call_method("dumps", (dict,), None)?
        .extract()?;
    let json_value: Value = serde_json::from_str(&json_str).map_err(to_pyvalue_err)?;

    if let Value::Object(map) = json_value {
        Ok(Some(Params::from_index_map(
            map.into_iter().collect::<IndexMap<String, Value>>(),
        )))
    } else {
        Err(to_pyvalue_err("应为字典对象"))
    }
}

/// 辅助函数，用于将 `serde_json::Value` 转换为 Python 对象。
///
/// 这是在将 `Params` 转换为 Python 字典时常用的一类转换模式。
///
/// # Errors
///
/// 如果值类型不受支持或转换失败，则返回 `PyErr`。
pub fn value_to_pyobject(py: Python<'_>, val: &Value) -> PyResult<Py<PyAny>> {
    match val {
        Value::Null => Ok(py.None()),
        Value::Bool(b) => b.into_py_any(py),
        Value::String(s) => s.into_py_any(py),
        Value::Number(n) => {
            if n.is_i64() {
                n.as_i64().unwrap().into_py_any(py)
            } else if n.is_u64() {
                n.as_u64().unwrap().into_py_any(py)
            } else if n.is_f64() {
                n.as_f64().unwrap().into_py_any(py)
            } else {
                Err(to_pyvalue_err("不支持的 JSON 数字类型"))
            }
        }
        Value::Array(arr) => {
            let py_list =
                PyList::new(py, &[] as &[Py<PyAny>]).expect("无效的 `ExactSizeIterator`已");
            for item in arr {
                let py_item = value_to_pyobject(py, item)?;
                py_list.append(py_item)?;
            }
            py_list.into_py_any(py)
        }
        Value::Object(_) => {
            // 对于嵌套对象，递归地转换为字典
            let json_str = serde_json::to_string(val).map_err(to_pyvalue_err)?;
            let py_dict: Py<PyDict> = PyModule::import(py, "json")?
                .call_method("loads", (json_str,), None)?
                .extract()?;
            py_dict.into_py_any(py)
        }
    }
}

/// 将 `Params` (IndexMap<String, Value>) 转换为 Python 字典。
///
/// # Errors
///
/// 如果任何值的转换失败，则返回 `PyErr`。
pub fn params_to_pydict(py: Python<'_>, params: &Params) -> PyResult<Py<PyDict>> {
    let dict = PyDict::new(py);
    for (key, value) in params {
        let py_value = value_to_pyobject(py, value)?;
        dict.set_item(key, py_value)?;
    }
    Ok(dict.into())
}
