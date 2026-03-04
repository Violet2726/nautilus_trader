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

//! [NautilusTrader](http://nautilustrader.io) 的通用组件。
//!
//! `nautilus-common` crate 提供了共享组件和工具，构成了 NautilusTrader 应用程序的系统基础。
//! 这包括 Actor 系统、消息总线、缓存层以及其他核心服务。
//!
//! # 平台
//!
//! [NautilusTrader](http://nautilustrader.io) 是一个开源、高性能、生产级的算法交易平台，
//! 为量化交易者提供了在历史数据上通过事件驱动引擎回测自动化交易策略组合的能力，
//! 并且可以将同样的策略部署到实盘，无需更改代码。
//!
//! NautilusTrader 的设计、架构和实现理念在最高层面上优先考虑软件的正确性和安全性，
//! 旨在支持关键任务的交易系统回测和实盘部署工作负载。
//!
//! # 特性标志 (Feature Flags)
//!
//! 本 crate 提供了特性标志来控制编译期间源码的包含情况，取决于预期的使用场景。
//! 例如，是为 [nautilus_trader](https://pypi.org/project/nautilus_trader) Python 包提供 Python 绑定，
//! 还是作为仅限 Rust 的构建的一部分。
//!
//! - `ffi`: 启用来自 [cbindgen](https://github.com/mozilla/cbindgen) 的 C 外部函数接口 (FFI)。
//! - `python`: 启用来自 [PyO3](https://pyo3.rs) 的 Python 绑定。
//! - `defi`: 启用 DeFi (去中心化金融) 支持。
//! - `indicators`: 包含 `nautilus-indicators` crate 和指标工具。
//! - `capnp`: 启用 [Cap'n Proto](https://capnproto.org/) 序列化支持。
//! - `live`: 启用用于实盘交易的 Tokio 异步运行时。
//! - `tracing-bridge`: 启用用于日志集成的 `tracing` 订阅者桥接。
//! - `extension-module`: 将 crate 构建为 Python 扩展模块。

#![warn(rustc::all)]
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(nonstandard_style)]
#![deny(missing_debug_implementations)]
#![deny(clippy::missing_errors_doc)]
#![deny(clippy::missing_panics_doc)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod actor;
pub mod cache;
pub mod clients;
pub mod clock;
pub mod component;
pub mod custom;
pub mod enums;
pub mod factories;
pub mod generators;
pub mod greeks;
pub mod logging;
pub mod messages;
pub mod msgbus;
pub mod runner;
pub mod session;
pub mod signal;
pub mod testing;
pub mod throttler;
pub mod timer;
pub mod xrate;

#[cfg(feature = "live")]
pub mod live;

#[cfg(feature = "defi")]
pub mod defi;

#[cfg(feature = "ffi")]
pub mod ffi;

#[cfg(feature = "python")]
pub mod python;

#[cfg(feature = "capnp")]
pub mod serialization;
