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

//! 来自 [cbindgen](https://github.com/mozilla/cbindgen) 的 C 外部函数接口 (FFI)。
//!
//! 所有导出的函数都通过 `abort_on_panic` 路由，以便 Rust 实现内部的任何 panic
//! 都会立即中止进程，而不是跨越外部边界进行展开 (unwinding)。
//! 展开到 C/Python 是未定义行为，因此这样做保持了现有的“快速失败”语义，
//! 同时避免了调试过程中微妙的栈损坏。

#![allow(unsafe_code)]
#![allow(unsafe_attr_outside_unsafe)]

pub mod cvec;
pub mod datetime;
pub mod parsing;
pub mod string;
pub mod uuid;

use std::{
    panic::{self, AssertUnwindSafe},
    process,
};

/// 执行 `f`，如果发生 panic 则中止进程。
///
/// FFI 导出函数始终调用此辅助函数，因此 panic 绝不会跨越 `extern "C"` 边界展开。
/// 展开到 C/Python 是未定义行为，并且会静默损坏外部栈；
/// 相比之下，采用中止操作可以在几乎没有调试负面影响（panic 信息在中止前仍会被记录）的情况下，
/// 有效保证了“快速失败”。
#[inline]
pub fn abort_on_panic<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => process::abort(),
    }
}
