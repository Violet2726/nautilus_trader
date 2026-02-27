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

//! 用于内省运行中的 Python 解释器及已安装包的函数。

#![allow(
    clippy::manual_let_else,
    reason = "在错误处理中更倾向于显式的控制流"
)]
use pyo3::{prelude::*, types::PyTuple};

/// 以字符串形式获取 Python 解释器版本。
///
/// # Panics
///
/// 如果 `version_info` 无法被转换为元组或元组元素缺失，则触发 panic。
#[must_use]
pub fn get_python_version() -> String {
    Python::attach(|py| {
        let sys = match py.import("sys") {
            Ok(mod_sys) => mod_sys,
            Err(_) => return "不可用 (导入 sys 失败)".to_string(),
        };

        let version_info = match sys.getattr("version_info") {
            Ok(info) => info,
            Err(_) => return "不可用 (未找到 version_info)".to_string(),
        };

        let version_tuple: &Bound<'_, PyTuple> = version_info
            .cast::<PyTuple>()
            .expect("未能提取 version_info");

        let major = version_tuple
            .get_item(0)
            .expect("未能获取主版本号 (major)")
            .extract::<i32>()
            .unwrap_or(-1);
        let minor = version_tuple
            .get_item(1)
            .expect("未能获取次版本号 (minor)")
            .extract::<i32>()
            .unwrap_or(-1);
        let micro = version_tuple
            .get_item(2)
            .expect("未能获取修订版本号 (micro)")
            .extract::<i32>()
            .unwrap_or(-1);

        if major == -1 || minor == -1 || micro == -1 {
            "不可用 (未能提取版本组件)".to_string()
        } else {
            format!("{major}.{minor}.{micro}")
        }
    })
}

#[must_use]
/// 尝试检索 *Python* 包的 `__version__` 属性。
///
/// 当请求的包无法被导入，或者它未定义 `__version__` 属性时，
/// 该函数返回一个以 `"Unavailable"` 开头的人类可读的回退字符串，
/// 以便下游代码能够区分“真正的”版本字符串与错误情况。
///
/// 此辅助程序主要用于 NautilusTrader Python 绑定内部的诊断/日志记录目的。
pub fn get_python_package_version(package_name: &str) -> String {
    Python::attach(|py| match py.import(package_name) {
        Ok(package) => match package.getattr("__version__") {
            Ok(version_attr) => match version_attr.extract::<String>() {
                Ok(version) => version,
                Err(_) => "不可用 (未能提取版本)".to_string(),
            },
            Err(_) => "不可用 (未找到 __version__ 属性)".to_string(),
        },
        Err(_) => "不可用 (导入包失败)".to_string(),
    })
}
