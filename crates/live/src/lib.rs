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

//! [NautilusTrader](http://nautilustrader.io) 的实盘系统节点。
//!
//! `nautilus-live` crate 为运行实盘交易系统提供了高层抽象和基础设施，
//! 包括数据流、执行管理和系统生命周期处理。
//! 它构建在系统内核之上，为实盘部署提供简化的接口：
//!
//! - `LiveNode` 实盘系统节点的高层抽象。
//! - `LiveNodeConfig` 实盘节点部署的配置。
//! - `AsyncRunner` 负责管理系统实时数据流。
//!
//! # 平台
//!
//! [NautilusTrader](http://nautilustrader.io) 是一个开源、高性能、生产级的算法交易平台，
//! 为量化交易者提供了利用事件驱动引擎在历史数据上进行自动化交易策略组合回测的能力，
//! 并且可以在不更改代码的情况下将这些策略部署到实盘环境。
//!
//! NautilusTrader 的设计、架构 and 实现理念将软件的正确性和安全性置于最高级别，
//! 旨在支持关键任务级别的交易系统回测 and 实盘部署工作负载。
//!
//! # 特性标志 (Feature Flags)
//!
//! 此 crate 提供了特性标志，用于根据预期的使用场景控制编译期间的源代码包含，
//! 例如是为 [nautilus_trader](https://pypi.org/project/nautilus_trader) Python 包提供 Python 绑定，
//! 还是作为纯 Rust 构建的一部分。
//!
//! - `ffi`: 启用来自 [cbindgen](https://github.com/mozilla/cbindgen) 的 C 外部函数接口 (FFI)。
//! - `streaming`: 启用 `persistence` 依赖项以进行流式配置。
//! - `python`: 启用来自 [PyO3](https://pyo3.rs) 的 Python 绑定（自动启用 `streaming`）。
//! - `defi`: 启用 DeFi（去中心化金融）支持。
//! - `extension-module`: 将 crate 构建为 Python 扩展模块。

#![warn(rustc::all)]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(nonstandard_style)]
#![deny(missing_debug_implementations)]
#![deny(clippy::missing_errors_doc)]
#![deny(clippy::missing_panics_doc)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod builder;
pub mod config;
pub mod emitter;
pub mod manager;
pub mod node;
pub mod runner;

// 适配器的重新导出 (Re-exports)
pub use emitter::ExecutionEventEmitter;
pub use nautilus_common::factories::OrderEventFactory;
pub use nautilus_execution::client::core::ExecutionClientCore;

#[cfg(feature = "python")]
pub mod python;
