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

//! 显式的、可手动调用的清理挂钩，用于在触发 `Drop` 之前打破引用循环。
//!
//! 许多长生命周期的组件会注册回调或处理程序，这些处理程序会保留指向其自身的强引用，从而产生引用计数循环，
//! 导致 Rust 的自动析构函数 (`Drop`) 无法运行。`CleanDrop` trait 提供了一个*对象安全*（object-safe）的方法 `clean_drop`，
//! 可以显式调用该方法（例如在正常关机期间）以释放此类资源。实现类还应当在其 `Drop` 实现中调用 `clean_drop`
//! 作为最后一道安全保障。
//!
//! 设计契约：
//! 1. **幂等性** – 多次调用必须是安全的。
//! 2. 在此处执行所有外部可观察到的清理工作（注销处理程序、中止任务、清除回调、降级 `Rc`/`Arc` 引用等）。

/// 提供可在 `Drop` 之前调用的显式清理方法的 trait。
pub trait CleanDrop {
    /// 执行自定义清理，释放外部资源并打破强引用循环。
    fn clean_drop(&mut self);
}
