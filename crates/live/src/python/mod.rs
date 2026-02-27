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

//! 来自 [PyO3](https://pyo3.rs) 的 Python 绑定。

pub mod node;

use pyo3::prelude::*;

/// 作为 `nautilus_pyo3.live` 加载。
///
/// # 错误
///
/// 如果注册任何模块组件失败，则返回 `PyErr`。
#[pymodule]
pub fn live(_: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<crate::node::LiveNode>()?;
    m.add_class::<node::LiveNodeBuilderPy>()?;
    Ok(())
}
