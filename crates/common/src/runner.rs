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

//! 全局运行时机制和线程局部存储 (thread-local storage)。
//!
//! 此模块提供对共享运行时资源的全局访问，包括时钟、消息队列和时间事件通道。
//! 它管理系统级组件的线程局部存储，使这些组件能够跨线程访问。

use std::{
    cell::{OnceCell, RefCell},
    fmt::Debug,
    sync::Arc,
};

use crate::{
    messages::{data::DataCommand, execution::TradingCommand},
    msgbus::{self, MessagingSwitchboard},
    timer::TimeEventHandler,
};

/// 数据命令发送者的 trait，可为同步和异步运行器实现。
pub trait DataCommandSender {
    /// 执行一个数据命令。
    ///
    /// - **同步运行器 (Sync runners)** 将命令发送到队列以进行同步执行。
    /// - **异步运行器 (Async runners)** 将命令发送到通道以进行异步执行。
    fn execute(&self, command: DataCommand);
}

/// 用于回测环境的同步 [`DataCommandSender`]。
///
/// 将命令缓冲在线程局部队列中以便延迟执行，
/// 从而避免从事件处理器回调发送时发生 `RefCell` 重入。
#[derive(Debug)]
pub struct SyncDataCommandSender;

impl DataCommandSender for SyncDataCommandSender {
    fn execute(&self, command: DataCommand) {
        DATA_CMD_QUEUE.with(|q| q.borrow_mut().push(command));
    }
}

/// 排空所有缓冲的数据命令，并将每个命令调度到数据引擎。
pub fn drain_data_cmd_queue() {
    DATA_CMD_QUEUE.with(|q| {
        let commands: Vec<DataCommand> = q.borrow_mut().drain(..).collect();
        let endpoint = MessagingSwitchboard::data_engine_execute();
        for cmd in commands {
            msgbus::send_data_command(endpoint, cmd);
        }
    });
}

/// 如果数据命令队列为空，则返回 `true`。
pub fn data_cmd_queue_is_empty() -> bool {
    DATA_CMD_QUEUE.with(|q| q.borrow().is_empty())
}

/// 获取全局数据命令发送者。
///
/// # Panics
///
/// 如果发送者未初始化，则会抛出 panic。
#[must_use]
pub fn get_data_cmd_sender() -> Arc<dyn DataCommandSender> {
    DATA_CMD_SENDER.with(|sender| {
        sender
            .get()
            .expect("数据命令发送者应由运行器初始化")
            .clone()
    })
}

/// 设置全局数据命令发送者。
///
/// 应该在运行器初始化时调用。
/// 每个线程只能调用一次。
///
/// # Panics
///
/// 如果已经设置了发送者，则会抛出 panic。
pub fn set_data_cmd_sender(sender: Arc<dyn DataCommandSender>) {
    DATA_CMD_SENDER.with(|s| {
        assert!(
            s.set(sender).is_ok(),
            "数据命令发送者只能设置一次"
        );
    });
}

/// 如果尚未设置，则初始化全局数据命令发送者（幂等性）。
pub fn init_data_cmd_sender(sender: Arc<dyn DataCommandSender>) {
    DATA_CMD_SENDER.with(|s| {
        let _ = s.set(sender); // Ignore if already set
    });
}

/// 时间事件发送者的 trait，可为同步和异步运行器实现。
pub trait TimeEventSender: Debug + Send + Sync {
    /// 发送一个时间事件处理器。
    fn send(&self, handler: TimeEventHandler);
}

/// 获取全局时间事件发送者。
///
/// # Panics
///
/// 如果发送者未初始化，则会抛出 panic。
#[must_use]
pub fn get_time_event_sender() -> Arc<dyn TimeEventSender> {
    TIME_EVENT_SENDER.with(|sender| {
        sender
            .get()
            .expect("时间事件发送者应由运行器初始化")
            .clone()
    })
}

/// 尝试获取全局时间事件发送者而不产生 panic。
///
/// 如果发送者未初始化（例如在测试环境中），则返回 `None`。
#[must_use]
pub fn try_get_time_event_sender() -> Option<Arc<dyn TimeEventSender>> {
    TIME_EVENT_SENDER.with(|sender| sender.get().cloned())
}

/// 设置全局时间事件发送者。
///
/// 每个线程只能调用一次。
///
/// # Panics
///
/// 如果已经设置了发送者，则会抛出 panic。
pub fn set_time_event_sender(sender: Arc<dyn TimeEventSender>) {
    TIME_EVENT_SENDER.with(|s| {
        assert!(
            s.set(sender).is_ok(),
            "时间事件发送者只能设置一次"
        );
    });
}

/// 交易命令发送者的 trait，可为同步和异步运行器实现。
pub trait TradingCommandSender {
    /// 执行一个交易命令。
    ///
    /// - **同步运行器 (Sync runners)** 将命令发送到队列以进行同步执行。
    /// - **异步运行器 (Async runners)** 将命令发送到通道以进行异步执行。
    fn execute(&self, command: TradingCommand);
}

/// 用于回测环境的同步 [`TradingCommandSender`]。
///
/// 将命令缓冲在线程局部队列中以便延迟执行，
/// 从而避免从事件处理器回调发送时发生 `RefCell` 重入。
#[derive(Debug)]
pub struct SyncTradingCommandSender;

impl TradingCommandSender for SyncTradingCommandSender {
    fn execute(&self, command: TradingCommand) {
        TRADING_CMD_QUEUE.with(|q| q.borrow_mut().push(command));
    }
}

/// 排空所有缓冲的交易命令，并将每个命令调度到执行引擎。
pub fn drain_trading_cmd_queue() {
    TRADING_CMD_QUEUE.with(|q| {
        let commands: Vec<TradingCommand> = q.borrow_mut().drain(..).collect();
        let endpoint = MessagingSwitchboard::exec_engine_execute();
        for cmd in commands {
            msgbus::send_trading_command(endpoint, cmd);
        }
    });
}

/// 如果交易命令队列为空，则返回 `true`。
pub fn trading_cmd_queue_is_empty() -> bool {
    TRADING_CMD_QUEUE.with(|q| q.borrow().is_empty())
}

/// 获取全局交易命令发送者。
///
/// # Panics
///
/// 如果发送者未初始化，则会抛出 panic。
#[must_use]
pub fn get_trading_cmd_sender() -> Arc<dyn TradingCommandSender> {
    EXEC_CMD_SENDER.with(|sender| {
        sender
            .get()
            .expect("交易命令发送者应由运行器初始化")
            .clone()
    })
}

/// 尝试获取全局交易命令发送者而不产生 panic。
///
/// 如果发送者未初始化（例如在测试环境中），则返回 `None`。
#[must_use]
pub fn try_get_trading_cmd_sender() -> Option<Arc<dyn TradingCommandSender>> {
    EXEC_CMD_SENDER.with(|sender| sender.get().cloned())
}

/// 设置全局交易命令发送者。
///
/// 应该在运行器初始化时调用。
/// 每个线程只能调用一次。
///
/// # Panics
///
/// 如果已经设置了发送者，则会抛出 panic。
pub fn set_exec_cmd_sender(sender: Arc<dyn TradingCommandSender>) {
    EXEC_CMD_SENDER.with(|s| {
        assert!(
            s.set(sender).is_ok(),
            "交易命令发送者只能设置一次"
        );
    });
}

/// 如果尚未设置，则初始化全局交易命令发送者（幂等性）。
pub fn init_exec_cmd_sender(sender: Arc<dyn TradingCommandSender>) {
    EXEC_CMD_SENDER.with(|s| {
        let _ = s.set(sender); // Ignore if already set
    });
}

thread_local! {
    static TIME_EVENT_SENDER: OnceCell<Arc<dyn TimeEventSender>> = const { OnceCell::new() };
    static DATA_CMD_SENDER: OnceCell<Arc<dyn DataCommandSender>> = const { OnceCell::new() };
    static EXEC_CMD_SENDER: OnceCell<Arc<dyn TradingCommandSender>> = const { OnceCell::new() };
    static DATA_CMD_QUEUE: RefCell<Vec<DataCommand>> = const { RefCell::new(Vec::new()) };
    static TRADING_CMD_QUEUE: RefCell<Vec<TradingCommand>> = const { RefCell::new(Vec::new()) };
}
