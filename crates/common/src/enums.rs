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

//! 通用组件的枚举类型。

use log::Level;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString, FromRepr};

/// 系统中组件的状态。
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Display,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    FromRepr,
    EnumIter,
    EnumString,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum ComponentState {
    /// 当组件被实例化，但尚未准备好履行其规范时。
    #[default]
    PreInitialized = 0,
    /// 当组件能够启动时。
    Ready = 1,
    /// 当组件在 `start` 时执行其操作。
    Starting = 2,
    /// 当组件正正常运行并能履行其规范时。
    Running = 3,
    /// 当组件在 `stop` 时执行其操作。
    Stopping = 4,
    /// 当组件已成功停止。
    Stopped = 5,
    /// 当组件在初始启动后再次启动。
    Resuming = 6,
    /// 当组件在 `reset` 时执行其操作。
    Resetting = 7,
    /// 当组件在 `dispose` 时执行其操作。
    Disposing = 8,
    /// 当组件已成功关闭并释放了所有资源。
    Disposed = 9,
    /// 当组件在 `degrade` 时执行其操作。
    Degrading = 10,
    /// 当组件已成功降级，可能无法完全履行其规范。
    Degraded = 11,
    /// 当组件在 `fault` 时执行其操作。
    Faulting = 12,
    /// 当组件因检测到故障而成功关闭。
    Faulted = 13,
}

impl ComponentState {
    pub fn variant_name(&self) -> String {
        let s = self.to_string();
        format!("{}{}", s[0..1].to_uppercase(), s[1..].to_lowercase())
    }
}

/// 系统中组件的触发条件。
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    FromRepr,
    EnumIter,
    EnumString,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum ComponentTrigger {
    /// 组件初始化的触发器。
    Initialize = 1,
    /// 组件启动的触发器。
    Start = 2,
    /// 组件成功启动时的触发器。
    StartCompleted = 3,
    /// 组件停止的触发器。
    Stop = 4,
    /// 组件成功停止时的触发器。
    StopCompleted = 5,
    /// 组件恢复（停止后）的触发器。
    ComponentResume = 6,
    /// 组件成功恢复时的触发器。
    ResumeCompleted = 7,
    /// 组件重置的触发器。
    Reset = 8,
    /// 组件成功重置时的触发器。
    ResetCompleted = 9,
    /// 组件销毁并释放资源的触发器。
    Dispose = 10,
    /// 组件成功销毁时的触发器。
    DisposeCompleted = 11,
    /// 组件降级的触发器。
    Degrade = 12,
    /// 组件成功降级时的触发器。
    DegradeCompleted = 13,
    /// 组件故障的触发器。
    Fault = 14,
    /// 组件成功进入故障状态时的触发器。
    FaultCompleted = 15,
}

/// 代表 Nautilus 系统的环境上下文。
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    FromRepr,
    EnumIter,
    EnumString,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum Environment {
    Backtest,
    Sandbox,
    Live,
}

/// 日志消息的日志级别。
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    FromRepr,
    EnumIter,
    EnumString,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum LogLevel {
    /// **OFF** 日志级别。低于所有其他日志级别（关闭）。
    #[strum(serialize = "OFF")]
    #[serde(rename = "OFF")]
    Off = 0,
    /// **TRACE** 日志级别。仅在 Rust 的调试/开发构建中可用。
    #[strum(serialize = "TRACE")]
    #[serde(rename = "TRACE")]
    Trace = 1,
    /// **DEBUG** 日志级别。
    #[strum(serialize = "DEBUG")]
    #[serde(rename = "DEBUG")]
    Debug = 2,
    /// **INFO** 日志级别。
    #[strum(serialize = "INFO")]
    #[serde(rename = "INFO")]
    Info = 3,
    /// **WARNING** 日志级别。
    #[strum(serialize = "WARN", serialize = "WARNING")]
    #[serde(rename = "WARNING")]
    Warning = 4,
    /// **ERROR** 日志级别。
    #[strum(serialize = "ERROR")]
    #[serde(rename = "ERROR")]
    Error = 5,
}

/// 日志消息的日志颜色。
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    FromRepr,
    EnumIter,
    EnumString,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum LogColor {
    /// 默认/常规的日志颜色。
    #[strum(serialize = "NORMAL")]
    Normal = 0,
    /// 绿色日志颜色，通常用于 [`LogLevel::Info`] 日志级别，并与成功事件相关联。
    #[strum(serialize = "GREEN")]
    Green = 1,
    /// 蓝色日志颜色，通常用于 [`LogLevel::Info`] 日志级别，并与用户操作相关联。
    #[strum(serialize = "BLUE")]
    Blue = 2,
    /// 品红色日志颜色，通常用于 [`LogLevel::Info`] 日志级别。
    #[strum(serialize = "MAGENTA")]
    Magenta = 3,
    /// 青色日志颜色，通常用于 [`LogLevel::Info`] 日志级别。
    #[strum(serialize = "CYAN")]
    Cyan = 4,
    /// 黄色日志颜色，通常用于 [`LogLevel::Warning`] 日志级别。
    #[strum(serialize = "YELLOW")]
    Yellow = 5,
    /// 红色日志颜色，通常用于 [`LogLevel::Error`] 级别。
    #[strum(serialize = "RED")]
    Red = 6,
}

impl LogColor {
    #[must_use]
    pub const fn as_ansi(&self) -> &str {
        match *self {
            Self::Normal => "",
            Self::Green => "\x1b[92m",
            Self::Blue => "\x1b[94m",
            Self::Magenta => "\x1b[35m",
            Self::Cyan => "\x1b[36m",
            Self::Yellow => "\x1b[1;33m",
            Self::Red => "\x1b[1;31m",
        }
    }
}

impl From<u8> for LogColor {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Green,
            2 => Self::Blue,
            3 => Self::Magenta,
            4 => Self::Cyan,
            5 => Self::Yellow,
            6 => Self::Red,
            _ => Self::Normal,
        }
    }
}

impl From<Level> for LogColor {
    fn from(value: Level) -> Self {
        match value {
            Level::Error => Self::Red,
            Level::Warn => Self::Yellow,
            Level::Info => Self::Normal,
            Level::Debug => Self::Normal,
            Level::Trace => Self::Normal,
        }
    }
}

/// ANSI 日志行格式说明符。
/// 用于使用 ANSI 转义码格式化日志消息。
#[repr(C)]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, FromRepr, EnumString, Display)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum LogFormat {
    /// 标题日志格式。此 ANSI 转义码用于品红色文本，通常用于日志输出中的标题。
    #[strum(serialize = "\x1b[95m")]
    Header,

    /// Endc 日志格式。此 ANSI 转义码用于将所有格式属性重置为默认值。应在应用其他格式后使用。
    #[strum(serialize = "\x1b[0m")]
    Endc,

    /// 粗体日志格式。此 ANSI 转义码用于使日志输出中的文本变粗。
    #[strum(serialize = "\x1b[1m")]
    Bold,

    /// 下划线日志格式。此 ANSI 转义码用于在日志输出中为文本加下划线。
    #[strum(serialize = "\x1b[4m")]
    Underline,
}

/// 序列化编码。
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Display,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    FromRepr,
    EnumIter,
    EnumString,
    Serialize,
    Deserialize,
)]
#[strum(ascii_case_insensitive)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum SerializationEncoding {
    /// MessagePack 编码。
    #[serde(rename = "msgpack")]
    MsgPack = 0,
    /// JavaScript 对象符号 (JSON) 编码。
    #[serde(rename = "json")]
    Json = 1,
}
