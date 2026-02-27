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

//! 为 PyO3 提供的 UUID 辅助工具。

use std::{
    collections::hash_map::DefaultHasher,
    ffi::CStr,
    hash::{Hash, Hasher},
    str::FromStr,
};

use pyo3::{
    IntoPyObjectExt, Py,
    prelude::*,
    pyclass::CompareOp,
    types::{PyBytes, PyTuple},
};

use super::{IntoPyObjectNautilusExt, to_pyvalue_err};
use crate::uuid::{UUID4, UUID4_LEN};

#[pymethods]
impl UUID4 {
    /// 创建一个新 [`UUID4`] 实例。
    ///
    /// 如果提供了字符串值，它会尝试将其解析为 UUID。
    /// 如果未提供任何值，则生成一个新的随机 UUID。
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    /// 在反序列化（unpickling）过程中设置 `UUID4` 实例的状态。
    #[allow(
        clippy::needless_pass_by_value,
        reason = "Python FFI 要求使用拥有的 (owned) 类型"
    )]
    fn __setstate__(&mut self, py: Python<'_>, state: Py<PyAny>) -> PyResult<()> {
        let bytes: &Bound<'_, PyBytes> = state.cast_bound::<PyBytes>(py)?;
        let slice = bytes.as_bytes();

        if slice.len() != UUID4_LEN {
            return Err(to_pyvalue_err(
                "反序列化状态无效，字节长度不正确",
            ));
        }

        if slice[UUID4_LEN - 1] != 0 {
            return Err(to_pyvalue_err(
                "反序列化状态无效，缺失 null 终止符",
            ));
        }

        let cstr = CStr::from_bytes_with_nul(slice).map_err(|_| {
            to_pyvalue_err("反序列化状态无效，字节必须是以 null 结尾的 UTF-8 编码")
        })?;

        let value = cstr.to_str().map_err(|_| {
            to_pyvalue_err("反序列化状态无效，字节必须是有效的 UTF-8 编码")
        })?;

        let parsed = Self::from_str(value).map_err(|e| {
            to_pyvalue_err(format!(
                "反序列化状态无效，无法解析 UUID: {e}"
            ))
        })?;

        self.value.copy_from_slice(&parsed.value);
        Ok(())
    }

    /// 获取 `UUID4` 实例的状态以进行序列化（pickling）。
    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        PyBytes::new(py, &self.value).into_py_any(py)
    }

    /// 为序列化（pickling）削减 (Reduce) `UUID4` 实例。
    fn __reduce__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let safe_constructor = py.get_type::<Self>().getattr("_safe_constructor")?;
        let state = self.__getstate__(py)?;
        (safe_constructor, PyTuple::empty(py), state).into_py_any(py)
    }

    /// 在反序列化（unpickling）过程中使用的安全构造函数，以确保 `UUID4` 的正确初始化。
    #[staticmethod]
    #[allow(
        clippy::unnecessary_wraps,
        reason = "Python FFI 要求返回 Result 类型"
    )]
    fn _safe_constructor() -> PyResult<Self> {
        Ok(Self::new()) // 安全默认值
    }

    /// 比较两个 `UUID4` 实例是否相等。
    fn __richcmp__(&self, other: &Self, op: CompareOp, py: Python<'_>) -> Py<PyAny> {
        match op {
            CompareOp::Eq => self.eq(other).into_py_any_unwrap(py),
            CompareOp::Ne => self.ne(other).into_py_any_unwrap(py),
            _ => py.NotImplemented(),
        }
    }

    /// 返回 `UUID4` 实例的哈希值。
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "为 Python 互操作性而进行的有意转换"
    )]
    fn __hash__(&self) -> isize {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish() as isize
    }

    /// 返回 `UUID4` 实例的详细字符串表示形式。
    fn __repr__(&self) -> String {
        format!("{self:?}")
    }

    /// 返回 `UUID4` 的字符串形式。
    fn __str__(&self) -> String {
        self.to_string()
    }

    /// 获取 `UUID4` 的字符串值。
    #[getter]
    #[pyo3(name = "value")]
    fn py_value(&self) -> String {
        self.to_string()
    }

    /// 从字符串表示形式创建一个新 [`UUID4`]。
    #[staticmethod]
    #[pyo3(name = "from_str")]
    fn py_from_str(value: &str) -> PyResult<Self> {
        Self::from_str(value).map_err(to_pyvalue_err)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use pyo3::Python;
    use rstest::rstest;

    use super::*;

    fn ensure_python_initialized() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            Python::initialize();
        });
    }

    #[rstest]
    fn test_setstate_rejects_invalid_uuid_bytes() {
        ensure_python_initialized();
        Python::attach(|py| {
            let mut uuid = UUID4::new();
            let mut invalid = [b'a'; UUID4_LEN];
            invalid[UUID4_LEN - 1] = 0;
            let py_bytes = PyBytes::new(py, &invalid);
            let err = uuid
                .__setstate__(py, py_bytes.into_py_any_unwrap(py))
                .expect_err("应在无效状态下报错");
            assert!(err.to_string().contains("Invalid state for deserializing"));
        });
    }

    #[rstest]
    fn test_setstate_rejects_missing_null_terminator() {
        ensure_python_initialized();
        Python::attach(|py| {
            let mut uuid = UUID4::new();
            let mut bytes = uuid.value;
            bytes[UUID4_LEN - 1] = b'0';
            let py_bytes = PyBytes::new(py, &bytes);
            let err = uuid
                .__setstate__(py, py_bytes.into_py_any_unwrap(py))
                .expect_err("应在缺失 NUL 终止符时报错");
            assert!(
                err.to_string()
                    .contains("Invalid state for deserializing, missing null terminator")
            );
        });
    }

    #[rstest]
    fn test_setstate_accepts_valid_state() {
        ensure_python_initialized();
        Python::attach(|py| {
            let source = UUID4::new();
            let mut target = UUID4::new();
            let py_bytes = PyBytes::new(py, &source.value);
            target
                .__setstate__(py, py_bytes.into_py_any_unwrap(py))
                .expect("有效状态应转换成功");
            assert_eq!(target.to_string(), source.to_string());
        });
    }
}
