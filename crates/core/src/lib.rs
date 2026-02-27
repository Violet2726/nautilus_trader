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

//! [NautilusTrader](http://nautilustrader.io) 的核心基础类型和工具。
//!
//! `nautilus-core` crate 被设计为轻量且高效，并尽可能提供零成本抽象。
//! 它提供了在整个 NautilusTrader 生态系统中使用的基本构建模块，包括：
//!
//! - 时间处理和原子时钟功能。
//! - UUID 生成和管理。
//! - 数学函数和插值工具。
//! - 正确性验证函数。
//! - 序列化 trait 和辅助程序。
//! - 跨平台环境工具。
//! - 常见集合的抽象。
//!
//! # 平台
//!
//! [NautilusTrader](http://nautilustrader.io) 是一个开源、高性能、生产级的算法交易平台，
//! 为量化交易者提供了利用事件驱动引擎在历史数据上进行自动化交易策略组合回测的能力，
//! 并且可以在不更改代码的情况下将这些策略部署到实盘环境。
//!
//! NautilusTrader 的设计、架构和实现理念将软件的正确性和安全性置于最高级别，
//! 旨在支持关键任务级别的交易系统回测和实盘部署工作负载。
//!
//! # 特性标志 (Feature Flags)
//!
//! 此 crate 提供了特性标志，用于根据预期的使用场景控制编译期间的源代码包含，
//! 例如是为 [nautilus_trader](https://pypi.org/project/nautilus_trader) Python 包提供 Python 绑定，
//! 还是作为纯 Rust 构建的一部分。
//!
//! - `ffi`: 启用来自 [cbindgen](https://github.com/mozilla/cbindgen) 的 C 外部函数接口 (FFI)。
//! - `python`: 启用来自 [PyO3](https://pyo3.rs) 的 Python 绑定。
//! - `extension-module`: 将 crate 构建为 Python 扩展模块。

#![warn(rustc::all)]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(nonstandard_style)]
#![deny(missing_debug_implementations)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod collections;
pub mod consts;
pub mod correctness;
pub mod datetime;
pub mod drop;
pub mod env;
pub mod formatting;
pub mod math;
pub mod message;
pub mod nanos;
pub mod params;
pub mod stack_str;

pub mod parsing;
pub mod paths;
pub mod serialization;
pub mod shared;
pub mod string;
pub mod time;
pub mod uuid;

#[cfg(feature = "ffi")]
pub mod ffi;

#[cfg(feature = "python")]
pub mod python;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("不支持的平台：Nautilus 仅支持 Linux、macOS 和 Windows");

// 重新导出 (Re-exports)
#[cfg(feature = "python")]
pub use crate::params::from_pydict;
pub use crate::{
    drop::CleanDrop,
    nanos::UnixNanos,
    params::Params,
    shared::SharedCell,
    shared::WeakCell,
    stack_str::STACKSTR_CAPACITY,
    stack_str::StackStr,
    time::AtomicTime,
    uuid::UUID4,
};

/// 当由于 Mutex 毒化 (poisoning) 而无法获取互斥锁保护时显示的消息。
///
/// 互斥锁保护应当使用 `expect` 而不是处理毒化错误。
/// 被毒化的互斥锁表示某个线程在持有锁时发生了 panic，这意味着受保护的数据可能处于不一致的状态。
/// 传播（抛出）该 panic 是惯用且安全的方法，因为使用可能损坏的数据继续运行会违反安全不变量。
pub const MUTEX_POISONED: &str = "互斥锁已毒化 (Mutex poisoned)";
