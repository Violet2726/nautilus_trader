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

//! 用于事件驱动消息处理的 Actor 系统。
//!
//! 此模块提供 NautilusTrader 中用于处理数据处理、事件管理和异步消息处理的 Actor 框架。
//! Actor 是处理隔离消息的轻量级组件。

#![allow(unsafe_code)]

use std::{any::Any, fmt::Debug};

use ustr::Ustr;

pub mod data_actor;
#[cfg(feature = "indicators")]
pub(crate) mod indicators;
pub mod registry;

#[cfg(test)]
mod tests;

// Re-exports
pub use data_actor::{DataActor, DataActorConfig, DataActorCore};

pub use crate::component::Component;

pub trait Actor: Any + Debug {
    /// Actor 的唯一标识符。
    fn id(&self) -> Ustr;
    /// 处理 `msg`。
    fn handle(&mut self, msg: &dyn Any);
    /// 将 `self` 作为 `Any` 的引用返回，以便支持向下转型 (downcasting)。
    fn as_any(&self) -> &dyn Any;
    /// 将 `self` 作为 `Any` 的可变引用返回，以便支持向下转型 (downcasting)。
    ///
    /// 默认实现仅将 `&mut Self` 强制转换为 `&mut dyn Any`。
    ///
    /// # 注意
    ///
    /// 此方法是非对象安全的，因此仅适用于具有固定大小 (sized) 的 `Self`。
    fn as_any_mut(&mut self) -> &mut dyn Any
    where
        Self: Sized,
    {
        self
    }
}
