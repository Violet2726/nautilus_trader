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

//! 使用 [`PyO3`](https://pyo3.rs) 构建的 Python 绑定与互操作性工具。

#![allow(
    deprecated,
    reason = "pyo3-stub-gen 目前依赖于某些被标记为已过时的 PyO3 初始化辅助程序"
)]
//!
//! 此子模块汇总了*仅在*启用 `python` 特性标志进行编译时才需要的 Rust 代码。
//! 它提供了薄适配层 (thin adapters)，以便在不牺牲类型安全或性能的情况下，
//! 能够从 `nautilus_trader` Python 包中调用 NautilusTrader 的功能。

pub mod casing;
pub mod datetime;
pub mod enums;
pub mod params;
pub mod parsing;
pub mod serialization;
/// Python 的字符串处理工具。
pub mod string;
pub mod uuid;
pub mod version;

use std::fmt::Display;

use pyo3::{
    Py,
    conversion::IntoPyObjectExt,
    exceptions::{
        PyException, PyKeyError, PyNotImplementedError, PyRuntimeError, PyTypeError, PyValueError,
    },
    prelude::*,
    types::PyString,
    wrap_pyfunction,
};
use pyo3_stub_gen::derive::gen_stub_pyfunction;

use crate::{
    UUID4,
    consts::{NAUTILUS_USER_AGENT, NAUTILUS_VERSION},
    datetime::{
        MILLISECONDS_IN_SECOND, NANOSECONDS_IN_MICROSECOND, NANOSECONDS_IN_MILLISECOND,
        NANOSECONDS_IN_SECOND,
    },
};

/// 通过获取 GIL 并妥善管理引用计数，安全地克隆一个 Python 对象。
///
/// 此函数的存在是为了打破在使用持有回调函数的结构体中的 `Arc<Py<PyAny>>` 时，
/// Rust 与 Python 之间可能产生的循环引用。最初的设计将 Python 回调包装在
/// `Arc` 中以实现多线程共享，但由此产生了循环引用：
///
/// 1. Rust `Arc` 持有 Python 对象 → 增加了 Python 引用计数。
/// 2. Python 对象可能引用 Rust 对象 → 形成了循环引用。
/// 3. 双方都无法被垃圾回收 → 导致内存泄漏。
///
/// 通过使用普通 `Py<PyAny>` 配合基于 GIL 的克隆而非 `Arc<Py<PyAny>>`，我们能够：
/// - 避免 Rust 与 Python 内存管理之间的循环引用。
/// - 确保在 GIL 保护下进行正确的 Python 引用计数。
/// - 允许 Rust 和 Python 的垃圾回收器正常工作。
///
/// # 安全性 (Safety)
///
/// 此函数在执行克隆操作前会妥善获取 Python GIL，确保了对 Python 对象的线程安全访问
/// 以及正确的引用计数。
#[must_use]
pub fn clone_py_object(obj: &Py<PyAny>) -> Py<PyAny> {
    Python::attach(|py| obj.clone_ref(py))
}

/// 使用单个参数调用 Python 回调，并记录发生的任何错误。
pub fn call_python(py: Python, callback: &Py<PyAny>, py_obj: Py<PyAny>) {
    if let Err(e) = callback.call1(py, (py_obj,)) {
        log::error!("调用 Python 时出错: {e}");
    }
}

/// 扩展 `IntoPyObjectExt` 辅助 trait，以便在转换后展开 (unwrap) `Py<PyAny>`。
pub trait IntoPyObjectNautilusExt<'py>: IntoPyObjectExt<'py> {
    /// 将 `self` 转换为 [`Py<PyAny>`]，如果转换失败则 *触发 panic*。
    ///
    /// 这是一个对 [`IntoPyObjectExt::into_py_any`] 的便捷封装，当我们确信转换不会失败时
    /// （例如转换原语或其他已经实现了必要 PyO3 trait 的类型时），可以避免繁琐的 `Result` 处理。
    #[inline]
    fn into_py_any_unwrap(self, py: Python<'py>) -> Py<PyAny> {
        self.into_py_any(py)
            .expect("未能将类型转换为 Py<PyAny>")
    }
}

impl<'py, T> IntoPyObjectNautilusExt<'py> for T where T: IntoPyObjectExt<'py> {}

/// 获取给定 Python 对象 `obj` 的类型名称。
///
/// # Errors
///
/// 如果访问类型名称失败，则返回错误。
pub fn get_pytype_name<'py>(obj: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyString>> {
    obj.get_type().name()
}

/// 将任何实现了 `Display` 的类型转换为 Python `ValueError`。
pub fn to_pyvalue_err(e: impl Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// 将任何实现了 `Display` 的类型转换为 Python `TypeError`。
pub fn to_pytype_err(e: impl Display) -> PyErr {
    PyTypeError::new_err(e.to_string())
}

/// 将任何实现了 `Display` 的类型转换为 Python `RuntimeError`。
pub fn to_pyruntime_err(e: impl Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// 将任何实现了 `Display` 的类型转换为 Python `KeyError`。
pub fn to_pykey_err(e: impl Display) -> PyErr {
    PyKeyError::new_err(e.to_string())
}

/// 将任何实现了 `Display` 的类型转换为 Python `Exception`。
pub fn to_pyexception(e: impl Display) -> PyErr {
    PyException::new_err(e.to_string())
}

/// 将任何实现了 `Display` 的类型转换为 Python `NotImplementedError`。
pub fn to_pynotimplemented_err(e: impl Display) -> PyErr {
    PyNotImplementedError::new_err(e.to_string())
}

/// 返回一个指明 `obj` 是否为 `PyCapsule` 的值。
///
/// 参数 (Parameters)
/// ----------
/// obj : Any
///     要检查的对象。
///
/// 返回 (Returns)
/// -------
/// bool
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "is_pycapsule")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Python FFI 要求使用拥有的 (owned) 类型"
)]
#[allow(unsafe_code)]
fn py_is_pycapsule(obj: Py<PyAny>) -> bool {
    // 安全性：obj.as_ptr() 返回一个有效的 Python 对象指针
    unsafe {
        // PyCapsule_CheckExact 检查该对象是否刚好是 PyCapsule
        pyo3::ffi::PyCapsule_CheckExact(obj.as_ptr()) != 0
    }
}

/// 加载为 `nautilus_pyo3.core`。
///
/// # Errors
///
/// 如果注册模块组件失败，则返回 `PyErr`。
#[pymodule]
#[rustfmt::skip]
pub fn core(_: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(stringify!(NAUTILUS_VERSION), NAUTILUS_VERSION)?;
    m.add(stringify!(NAUTILUS_USER_AGENT), NAUTILUS_USER_AGENT)?;
    m.add(stringify!(MILLISECONDS_IN_SECOND), MILLISECONDS_IN_SECOND)?;
    m.add(stringify!(NANOSECONDS_IN_SECOND), NANOSECONDS_IN_SECOND)?;
    m.add(stringify!(NANOSECONDS_IN_MILLISECOND), NANOSECONDS_IN_MILLISECOND)?;
    m.add(stringify!(NANOSECONDS_IN_MICROSECOND), NANOSECONDS_IN_MICROSECOND)?;
    m.add_class::<UUID4>()?;
    m.add_function(wrap_pyfunction!(py_is_pycapsule, m)?)?;
    m.add_function(wrap_pyfunction!(casing::py_convert_to_snake_case, m)?)?;
    m.add_function(wrap_pyfunction!(string::py_mask_api_key, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_secs_to_nanos, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_secs_to_millis, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_millis_to_nanos, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_micros_to_nanos, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_nanos_to_secs, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_nanos_to_millis, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_nanos_to_micros, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_unix_nanos_to_iso8601, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_last_weekday_nanos, m)?)?;
    m.add_function(wrap_pyfunction!(datetime::py_is_within_last_24_hours, m)?)?;
    Ok(())
}
