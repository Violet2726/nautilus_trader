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

//! 解析项目和工作区目录路径的工具函数。

use std::path::PathBuf;

/// 返回工作区根目录路径。
///
/// 这是包含顶级 `Cargo.toml`（带有 `[workspace]` 部分）的目录，通常也是 `pyproject.toml` 和 `docs/` 所在的目录。
///
/// # Panics
///
/// 如果环境变量 `CARGO_MANIFEST_DIR` 未设置或无法确定其父目录，则触发 panic。
#[must_use]
pub fn get_workspace_root_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // 从 crates/core 级 parent 到 crates/
        .and_then(|p| p.parent()) // 从 crates/ 级 parent 到 nautilus_trader/
        .expect("无法获取工作区根目录")
        .to_path_buf()
}

/// 返回项目根目录路径。
///
/// 对于此单仓工程 (monorepo)，项目根目录与工作区根目录相同。
///
/// # Panics
///
/// 如果无法确定工作区根目录路径，则触发 panic。
#[must_use]
pub fn get_project_root_path() -> PathBuf {
    get_workspace_root_path()
}

/// 返回测试代码根目录路径。
#[must_use]
pub fn get_tests_root_path() -> PathBuf {
    get_project_root_path().join("tests")
}

/// 返回测试数据目录路径。
#[must_use]
pub fn get_test_data_path() -> PathBuf {
    if let Ok(test_data_root_path) = std::env::var("TEST_DATA_ROOT_PATH") {
        get_project_root_path()
            .join(test_data_root_path)
            .join("test_data")
    } else {
        get_project_root_path().join("tests").join("test_data")
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_workspace_root_contains_pyproject() {
        let root = get_workspace_root_path();
        assert!(
            root.join("pyproject.toml").exists(),
            "工作区根目录应当包含 pyproject.toml，实际为：{root:?}"
        );
    }

    #[rstest]
    fn test_workspace_root_contains_crates_dir() {
        let root = get_workspace_root_path();
        assert!(
            root.join("crates").is_dir(),
            "工作区根目录应当包含 crates/ 目录，实际为：{root:?}"
        );
    }

    #[rstest]
    fn test_project_root_equals_workspace_root() {
        assert_eq!(get_project_root_path(), get_workspace_root_path());
    }

    #[rstest]
    fn test_tests_root_path() {
        let tests_root = get_tests_root_path();
        assert!(
            tests_root.ends_with("tests"),
            "测试代码根目录应以 'tests' 结尾，实际为：{tests_root:?}"
        );
    }
}
