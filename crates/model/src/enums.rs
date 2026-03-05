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

//! 交易领域模型的枚举。

use std::{str::FromStr, sync::OnceLock};

use ahash::AHashSet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{AsRefStr, Display, EnumIter, EnumString, FromRepr};

use crate::enum_strum_serde;

/// 提供 `u8` 值到枚举类型的转换。
pub trait FromU8 {
    /// 将 `u8` 值转换为实现类型。
    ///
    /// 如果值不是有效表示，则返回 `None`。
    fn from_u8(value: u8) -> Option<Self>
    where
        Self: Sized;
}

/// 提供 `u16` 值到枚举类型的转换。
pub trait FromU16 {
    /// 将 `u16` 值转换为实现类型。
    ///
    /// 如果值不是有效表示，则返回 `None`。
    fn from_u16(value: u16) -> Option<Self>
    where
        Self: Sized;
}

/// 交易场所或经纪人提供的账户类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum AccountType {
    /// 仅包含无杠杆现金资产的账户。
    Cash = 1,
    /// 使用账户资产作为抵押品进行保证金交易的账户。
    Margin = 2,
    /// 特定于博彩市场的账户。
    Betting = 3,
    /// 表示区块链钱包的账户。
    Wallet = 4,
}

/// 派生数据的聚合源。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum AggregationSource {
    /// 数据在外部聚合（在 Nautilus 系统边界之外）。
    External = 1,
    /// 数据在内部聚合（在 Nautilus 系统边界之内）。
    Internal = 2,
}

/// 市场中交易的主动订单方。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum AggressorSide {
    /// 该交易没有特定的主动方。
    #[default]
    NoAggressor = 0,
    /// 买单是该交易的主动方。
    Buyer = 1,
    /// 卖单是该交易的主动方。
    Seller = 2,
}

impl FromU8 for AggressorSide {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::NoAggressor),
            1 => Some(Self::Buyer),
            2 => Some(Self::Seller),
            _ => None,
        }
    }
}

/// 广泛的金融市场资产类别。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
#[allow(non_camel_case_types)]
pub enum AssetClass {
    /// 外汇（FOREX）资产。
    FX = 1,
    /// 股票/权益资产。
    Equity = 2,
    /// 大宗商品资产。
    Commodity = 3,
    /// 基于债务的资产。
    Debt = 4,
    /// 基于指数的资产（篮子）。
    Index = 5,
    /// 加密货币或加密代币资产。
    Cryptocurrency = 6,
    /// 另类资产。
    Alternative = 7,
}

impl FromU8 for AssetClass {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::FX),
            2 => Some(Self::Equity),
            3 => Some(Self::Commodity),
            4 => Some(Self::Debt),
            5 => Some(Self::Index),
            6 => Some(Self::Cryptocurrency),
            7 => Some(Self::Alternative),
            _ => None,
        }
    }
}

/// 生成和关闭 K 线的聚合方法。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum BarAggregation {
    /// 基于一定数量的 tick。
    Tick = 1,
    /// 基于 tick 的买卖失衡。
    TickImbalance = 2,
    /// 基于连续的买卖 tick 序列。
    TickRuns = 3,
    /// 基于交易量。
    Volume = 4,
    /// 基于交易量的买卖失衡。
    VolumeImbalance = 5,
    /// 基于连续的买卖交易量序列。
    VolumeRuns = 6,
    /// 基于合约的"名义"价值。
    Value = 7,
    /// 基于名义价值的交易买卖失衡。
    ValueImbalance = 8,
    /// 基于连续的买卖名义价值序列。
    ValueRuns = 9,
    /// 基于毫秒级的时间间隔。
    Millisecond = 10,
    /// 基于秒级的时间间隔。
    Second = 11,
    /// 基于分钟级的时间间隔。
    Minute = 12,
    /// 基于小时级的时间间隔。
    Hour = 13,
    /// 基于天级的时间间隔。
    Day = 14,
    /// 基于周级的时间间隔。
    Week = 15,
    /// 基于月级的时间间隔。
    Month = 16,
    /// 基于年级的时间间隔。
    Year = 17,
    /// 基于固定的价格变动（砖块大小）。
    Renko = 18,
}

/// K 线聚合的间隔类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum BarIntervalType {
    /// 左开区间 `(start, end]`：起始是排他的，结束是包含的（默认）。
    #[default]
    LeftOpen = 1,
    /// 右开区间 `[start, end)`：起始是包含的，结束是排他的。
    RightOpen = 2,
}

/// 表示博彩市场中注的一方。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum BetSide {
    /// "支持"注表示对特定结果的支持。
    Back = 1,
    /// "反对"注表示对特定结果的反对。
    Lay = 2,
}

impl BetSide {
    /// 返回相反的注方。
    #[must_use]
    pub fn opposite(&self) -> Self {
        match self {
            Self::Back => Self::Lay,
            Self::Lay => Self::Back,
        }
    }
}

impl From<OrderSide> for BetSide {
    /// 返回给定 [`OrderSide`] 的等效 [`BetSide`]。
    ///
    /// # Panics
    ///
    /// 如果 `side` 是 [`OrderSide::NoOrderSide`] 则会 panic。
    fn from(side: OrderSide) -> Self {
        match side {
            OrderSide::Buy => Self::Back,
            OrderSide::Sell => Self::Lay,
            OrderSide::NoOrderSide => panic!("Invalid `OrderSide` for `BetSide`, was {side}"),
        }
    }
}

/// 订单簿事件的订单操作类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum BookAction {
    /// 订单被添加到订单簿。
    Add = 1,
    /// 订单簿中的现有订单被更新/修改。
    Update = 2,
    /// 订单簿中的现有订单被删除/取消。
    Delete = 3,
    /// 订单簿的状态被清除。
    Clear = 4,
}

impl FromU8 for BookAction {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Add),
            2 => Some(Self::Update),
            3 => Some(Self::Delete),
            4 => Some(Self::Clear),
            _ => None,
        }
    }
}

/// 订单簿类型，表示级别粒度和增量更新启发式方法的类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
#[allow(non_camel_case_types)]
pub enum BookType {
    /// 订单簿顶部的最佳买价/卖价，每方一个级别。
    L1_MBP = 1,
    /// 按价格的市场，每个级别一个订单（聚合）。
    L2_MBP = 2,
    /// 按订单的市场，每个级别多个订单（完整粒度）。
    L3_MBO = 3,
}

impl FromU8 for BookType {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::L1_MBP),
            2 => Some(Self::L2_MBP),
            3 => Some(Self::L3_MBO),
            _ => None,
        }
    }
}

/// 订单条件类型，指定关联订单的行为。
///
/// [FIX 5.0 SP2 : ContingencyType <1385> 字段](https://www.onixs.biz/fix-dictionary/5.0.sp2/tagnum_1385.html)。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum ContingencyType {
    /// 不是条件订单。
    #[default]
    NoContingency = 0,
    /// 一方取消另一方。
    Oco = 1,
    /// 一方触发另一方。
    Oto = 2,
    /// 一方更新另一方（按比例数量）。
    Ouo = 3,
}

/// 广泛的货币类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum CurrencyType {
    /// 加密货币或加密代币的类型。
    Crypto = 1,
    /// 政府发行且不以商品为支持的货币类型。
    Fiat = 2,
    /// 基于基础商品价值的货币类型。
    CommodityBacked = 3,
}

/// 合约类别。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum InstrumentClass {
    /// 现货市场合约类别。立即交割和支付的合约的当前市场价格。
    Spot = 1,
    /// 互换合约类别。一种衍生合约，双方通过该合约交换两种不同金融工具的现金流或负债。
    Swap = 2,
    /// 期货合约类别。一种法律协议，在未来的特定时间以预定价格购买或出售资产。
    Future = 3,
    /// 期货价差合约类别。一种使用期货合约的策略，利用不同合约月份、基础资产或市场之间的价格差异。
    FuturesSpread = 4,
    /// 远期衍生合约类别。双方之间的定制合约，在未来的特定日期以指定价格购买或出售资产。
    Forward = 5,
    /// 差价合约（CFD）类别。投资者与 CFD 经纪人之间的合约，交换合约开仓和平仓之间金融产品价值的差异。
    Cfd = 6,
    /// 债券合约类别。一种债务投资，投资者向实体（通常是公司或政府）出借资金，实体在定义的时间段内以可变或固定利率借入资金。
    Bond = 7,
    /// 期权合约类别。一种衍生品，赋予持有人在特定未来日期之前或之时以预定价格购买或出售基础资产的权利，而非义务。
    Option = 8,
    /// 期权价差合约类别。一种策略，涉及购买和/或出售同一基础资产上具有不同行权价或到期日的多个期权合约，以对冲风险或投机价格变动。
    OptionSpread = 9,
    /// 认股权证合约类别。一种衍生品，赋予持有人在到期前以特定价格购买或出售证券的权利，而非义务。
    Warrant = 10,
    /// 体育博彩合约类别。一种金融化衍生品，允许使用结构化合约或预测市场对体育事件的结果进行投注。
    SportsBetting = 11,
    /// 二元期权合约类别。一种衍生品，其收益要么是固定金额，要么是什么都没有，具体取决于基础资产在到期时的价格是否高于或低于预定水平。
    BinaryOption = 12,
}

impl InstrumentClass {
    /// 返回此合约类别是否有到期日。
    #[must_use]
    pub const fn has_expiration(&self) -> bool {
        matches!(
            self,
            Self::Future | Self::FuturesSpread | Self::Option | Self::OptionSpread
        )
    }
}

/// 合约关闭的事件类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum InstrumentCloseType {
    /// 当市场会话结束时。
    EndOfSession = 1,
    /// 当合约到期时。
    ContractExpired = 2,
}

/// 将给定的 `value` 转换为 [`InstrumentCloseType`]。
impl FromU8 for InstrumentCloseType {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::EndOfSession),
            2 => Some(Self::ContractExpired),
            _ => None,
        }
    }
}

/// 交易的流动性方。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
#[allow(clippy::enum_variant_names)]
pub enum LiquiditySide {
    /// 未指定流动性方。
    NoLiquiditySide = 0,
    /// 订单被动地为市场提供流动性以完成交易（做市）。
    Maker = 1,
    /// 订单主动地从市场获取流动性以完成交易。
    Taker = 2,
}

/// 交易场所上单个市场的状态。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum MarketStatus {
    /// 合约正在交易。
    Open = 1,
    /// 合约处于开盘前时段。
    Closed = 2,
    /// 合约的交易已暂停。
    Paused = 3,
    /// 合约的交易已停止。
    // Halted = 4,  # TODO: 由于 Cython（C 枚举命名空间）的原因，暂时无法使用
    /// 合约的交易已中止。
    Suspended = 5,
    /// 合约的交易不可用。
    NotAvailable = 6,
}

/// 影响交易场所上单个市场状态的操作。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum MarketStatusAction {
    /// 无变化。
    None = 0,
    /// 合约处于开盘前时段。
    PreOpen = 1,
    /// 合约处于预交叉时段。
    PreCross = 2,
    /// 合约正在报价但未交易。
    Quoting = 3,
    /// 合约处于交叉/拍卖时段。
    Cross = 4,
    /// 合约正在通过交易轮换开盘。
    Rotation = 5,
    /// 合约有新的价格指示可用。
    NewPriceIndication = 6,
    /// 合约正在交易。
    Trading = 7,
    /// 合约的交易已停止。
    Halt = 8,
    /// 合约的交易已暂停。
    Pause = 9,
    /// 合约的交易已中止。
    Suspend = 10,
    /// 合约处于收盘前时段。
    PreClose = 11,
    /// 合约的交易已收盘。
    Close = 12,
    /// 合约处于收盘后时段。
    PostClose = 13,
    /// 卖空限制的变化。
    ShortSellRestrictionChange = 14,
    /// 合约不可用于交易，交易已收盘或停止。
    NotAvailableForTrading = 15,
    /// 合约已复牌。
    Resume = 16,
}

/// 将给定的 `value` 转换为 [`OrderSide`]。
impl FromU16 for MarketStatusAction {
    fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::PreOpen),
            2 => Some(Self::PreCross),
            3 => Some(Self::Quoting),
            4 => Some(Self::Cross),
            5 => Some(Self::Rotation),
            6 => Some(Self::NewPriceIndication),
            7 => Some(Self::Trading),
            8 => Some(Self::Halt),
            9 => Some(Self::Pause),
            10 => Some(Self::Suspend),
            11 => Some(Self::PreClose),
            12 => Some(Self::Close),
            13 => Some(Self::PostClose),
            14 => Some(Self::ShortSellRestrictionChange),
            15 => Some(Self::NotAvailableForTrading),
            16 => Some(Self::Resume),
            _ => None,
        }
    }
}

/// 交易场所或交易策略的订单管理系统（OMS）类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum OmsType {
    /// 未指定特定的订单管理类型（将委托给交易所 OMS）。
    #[default]
    Unspecified = 0,
    /// 每个合约一个持仓的净额类型。
    Netting = 1,
    /// 每个合约可以有多个持仓的对冲类型。
    /// 这可以是多头/空头方向，按持仓/票据 ID，或由 Nautilus 虚拟跟踪。
    Hedging = 2,
}

/// 期权合约的种类。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum OptionKind {
    /// 看涨期权赋予持有人在指定时期内以指定行权价购买基础资产的权利，而非义务。
    Call = 1,
    /// 看跌期权赋予持有人在指定时期内以指定行权价出售基础资产的权利，而非义务。
    Put = 2,
}

/// 定义 OTO（一方触发另一方）子订单何时释放。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum OtoTriggerMode {
    /// 按比例释放子订单以响应每个部分成交（默认）。
    #[default]
    Partial = 0,
    /// 仅在父订单完全成交后释放子订单。
    Full = 1,
}

/// 特定订单或与订单相关的操作的订单方。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[allow(clippy::enum_variant_names)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum OrderSide {
    /// 未指定订单方。
    #[default]
    NoOrderSide = 0,
    /// 订单是买单。
    Buy = 1,
    /// 订单是卖单。
    Sell = 2,
}

impl OrderSide {
    /// 返回此方的指定 [`OrderSideSpecified`]（买入或卖出）。
    ///
    /// # Panics
    ///
    /// 如果 `self` 是 [`OrderSide::NoOrderSide`] 则会 panic。
    #[must_use]
    pub fn as_specified(&self) -> OrderSideSpecified {
        match &self {
            Self::Buy => OrderSideSpecified::Buy,
            Self::Sell => OrderSideSpecified::Sell,
            _ => panic!("Order invariant failed: side must be `Buy` or `Sell`"),
        }
    }
}

/// 将给定的 `value` 转换为 [`OrderSide`]。
impl FromU8 for OrderSide {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::NoOrderSide),
            1 => Some(Self::Buy),
            2 => Some(Self::Sell),
            _ => None,
        }
    }
}

/// 指定的订单方（买入或卖出）。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[allow(clippy::enum_variant_names)]
pub enum OrderSideSpecified {
    /// 订单是买单。
    Buy = 1,
    /// 订单是卖单。
    Sell = 2,
}

impl OrderSideSpecified {
    /// 返回相反的订单方。
    #[must_use]
    pub fn opposite(&self) -> Self {
        match &self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }

    /// 将此指定方转换为 [`OrderSide`]。
    #[must_use]
    pub fn as_order_side(&self) -> OrderSide {
        match &self {
            Self::Buy => OrderSide::Buy,
            Self::Sell => OrderSide::Sell,
        }
    }
}

/// 特定订单的状态。
///
/// 以下状态下的订单被视为_开放_：
///  - `ACCEPTED`
///  - `TRIGGERED`
///  - `PENDING_UPDATE`
///  - `PENDING_CANCEL`
///  - `PARTIALLY_FILLED`
///
/// 以下状态下的订单被视为_在途_：
///  - `SUBMITTED`
///  - `PENDING_UPDATE`
///  - `PENDING_CANCEL`
///
/// 以下状态下的订单被视为_已关闭_：
///  - `DENIED`
///  - `REJECTED`
///  - `CANCELED`
///  - `EXPIRED`
///  - `FILLED`
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum OrderStatus {
    /// 订单在 Nautilus 系统中已初始化（实例化）。
    Initialized = 1,
    /// 订单被 Nautilus 系统拒绝，因为无效、无法处理或超出风险限制。
    Denied = 2,
    /// 订单在 Nautilus 系统的 `OrderEmulator` 组件中被模拟。
    Emulated = 3,
    /// 订单从 Nautilus 系统的 `OrderEmulator` 组件中释放。
    Released = 4,
    /// 订单由 Nautilus 系统提交到外部服务或交易场所（等待确认）。
    Submitted = 5,
    /// 订单被交易场所确认为已接收且有效（现在可能正在工作）。
    Accepted = 6,
    /// 订单被交易场所拒绝。
    Rejected = 7,
    /// 订单被取消（关闭/完成）。
    Canceled = 8,
    /// 订单达到 GTD 到期（关闭/完成）。
    Expired = 9,
    /// 订单的 STOP 价格在交易场所被触发。
    Triggered = 10,
    /// 订单当前在交易场所上等待修改请求。
    PendingUpdate = 11,
    /// 订单当前在交易场所上等待取消请求。
    PendingCancel = 12,
    /// 订单在交易场所上已部分成交。
    PartiallyFilled = 13,
    /// 订单在交易场所上已完全成交（关闭/完成）。
    Filled = 14,
}

impl OrderStatus {
    /// 返回一个缓存的 `AHashSet`，包含适合取消查询的订单状态。
    ///
    /// 这些是订单在交易所工作但尚未处于取消或更新过程中的状态。
    /// 在取消过滤器中包含 `PENDING_CANCEL` 可能会导致重复的取消尝试或错误的开放订单计数。
    ///
    /// 返回：
    /// - `ACCEPTED`：订单在交易所工作。
    /// - `TRIGGERED`：止损订单已被触发。
    /// - `PENDING_UPDATE`：订单正在更新。
    /// - `PARTIALLY_FILLED`：订单已部分成交但仍工作。
    ///
    /// 排除：
    /// - `PENDING_CANCEL`：正在被取消。
    #[must_use]
    pub fn cancellable_statuses_set() -> &'static AHashSet<Self> {
        static CANCELLABLE_SET: OnceLock<AHashSet<OrderStatus>> = OnceLock::new();
        CANCELLABLE_SET.get_or_init(|| {
            AHashSet::from_iter([
                Self::Accepted,
                Self::Triggered,
                Self::PendingUpdate,
                Self::PartiallyFilled,
            ])
        })
    }

    /// 返回订单状态是否表示开放/工作订单。
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(
            self,
            Self::Submitted
                | Self::Accepted
                | Self::Triggered
                | Self::PendingUpdate
                | Self::PendingCancel
                | Self::PartiallyFilled
        )
    }
}

/// 订单类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum OrderType {
    /// 以当前市场最佳可用价格买入或卖出的市价订单。
    Market = 1,
    /// 以特定价格或更好价格买入或卖出的限价订单。
    Limit = 2,
    /// 当价格达到指定止损/触发价格时买入或卖出的止损市价订单。当止损价格达到时，订单实际上成为市价订单。
    StopMarket = 3,
    /// 结合止损订单和限价订单特征的止损限价买入或卖出订单。一旦止损/触发价格达到，止损限价订单实际上成为限价订单。
    StopLimit = 4,
    /// 市价转限价订单是市价订单，在到达市场后以当前最佳市场价格作为限价订单执行。
    MarketToLimit = 5,
    /// 当达到指定触发价格时，市价触及订单实际上成为市价订单。
    MarketIfTouched = 6,
    /// 当达到指定触发价格时，限价触及订单实际上成为限价订单。
    LimitIfTouched = 7,
    /// 跟踪止损市价订单将止损/触发价格设置为距离市场的固定"跟踪偏移"量。
    TrailingStopMarket = 8,
    /// 跟踪止损限价订单结合了跟踪止损订单和限价订单的特征。
    TrailingStopLimit = 9,
}

/// 持仓调整类型。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum PositionAdjustmentType {
    /// 影响持仓数量的佣金调整。
    Commission = 1,
    /// 影响持仓已实现盈亏的资金支付。
    Funding = 2,
}

impl FromU8 for PositionAdjustmentType {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Commission),
            2 => Some(Self::Funding),
            _ => None,
        }
    }
}

/// 特定持仓的市场方，或与持仓相关的操作。
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[allow(clippy::enum_variant_names)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum PositionSide {
    /// 未指定持仓方（仅在涉及持仓的操作的过滤器上下文中有效）。
    #[default]
    NoPositionSide = 0,
    /// 中性/平仓持仓，当前市场中未持有任何持仓。
    Flat = 1,
    /// 市场中的多头持仓，通常通过一个或多个买单获得。
    Long = 2,
    /// 市场中的空头持仓，通常通过一个或多个卖单获得。
    Short = 3,
}

impl PositionSide {
    /// 返回此方的指定 [`PositionSideSpecified`]（`Long`、`Short` 或 `Flat`）。
    ///
    /// # Panics
    ///
    /// 如果 `self` 是 [`PositionSide::NoPositionSide`] 则会 panic。
    #[must_use]
    pub fn as_specified(&self) -> PositionSideSpecified {
        match &self {
            Self::Long => PositionSideSpecified::Long,
            Self::Short => PositionSideSpecified::Short,
            Self::Flat => PositionSideSpecified::Flat,
            _ => panic!("Position invariant failed: side must be `Long`, `Short`, or `Flat`"),
        }
    }
}

/// The market side for a specific position, or action related to positions.
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[allow(clippy::enum_variant_names)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum PositionSideSpecified {
    /// A neural/flat position, where no position is currently held in the market.
    Flat = 1,
    /// A long position in the market, typically acquired through one or many BUY orders.
    Long = 2,
    /// A short position in the market, typically acquired through one or many SELL orders.
    Short = 3,
}

impl PositionSideSpecified {
    /// Converts this specified side into a [`PositionSide`].
    #[must_use]
    pub fn as_position_side(&self) -> PositionSide {
        match &self {
            Self::Long => PositionSide::Long,
            Self::Short => PositionSide::Short,
            Self::Flat => PositionSide::Flat,
        }
    }
}

/// The type of price for an instrument in a market.
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum PriceType {
    /// The best quoted price at which buyers are willing to buy a quantity of an instrument.
    /// Often considered the best bid in the order book.
    Bid = 1,
    /// The best quoted price at which sellers are willing to sell a quantity of an instrument.
    /// Often considered the best ask in the order book.
    Ask = 2,
    /// The arithmetic midpoint between the best bid and ask quotes.
    Mid = 3,
    /// The price at which the last trade of an instrument was executed.
    Last = 4,
    /// A reference price reflecting an instrument's fair value, often used for portfolio
    /// calculations and risk management.
    Mark = 5,
}

/// A record flag bit field, indicating event end and data information.
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
#[allow(non_camel_case_types)]
pub enum RecordFlag {
    /// Last message in the book event or packet from the venue for a given `instrument_id`.
    F_LAST = 1 << 7, // 128
    /// Top-of-book message, not an individual order.
    F_TOB = 1 << 6, // 64
    /// Message sourced from a replay, such as a snapshot server.
    F_SNAPSHOT = 1 << 5, // 32
    /// Aggregated price level message, not an individual order.
    F_MBP = 1 << 4, // 16
    /// Reserved for future use.
    RESERVED_2 = 1 << 3, // 8
    /// Reserved for future use.
    RESERVED_1 = 1 << 2, // 4
}

impl RecordFlag {
    /// Checks if the flag matches a given value.
    #[must_use]
    pub fn matches(self, value: u8) -> bool {
        (self as u8) & value != 0
    }
}

/// The 'Time in Force' instruction for an order.
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum TimeInForce {
    /// Good Till Cancel (GTC) - Remains active until canceled.
    Gtc = 1,
    /// Immediate or Cancel (IOC) - Executes immediately to the extent possible, with any unfilled portion canceled.
    Ioc = 2,
    /// Fill or Kill (FOK) - Executes in its entirety immediately or is canceled if full execution is not possible.
    Fok = 3,
    /// Good Till Date (GTD) - Remains active until the specified expiration date or time is reached.
    Gtd = 4,
    /// Day - Remains active until the close of the current trading session.
    Day = 5,
    /// At the Opening (ATO) - Executes at the market opening or expires if not filled.
    AtTheOpen = 6,
    /// At the Closing (ATC) - Executes at the market close or expires if not filled.
    AtTheClose = 7,
}

/// The trading state for a node.
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum TradingState {
    /// Normal trading operations.
    Active = 1,
    /// Trading is completely halted, no new order commands will be emitted.
    Halted = 2,
    /// Only order commands which would cancel order, or reduce position sizes are permitted.
    Reducing = 3,
}

/// The trailing offset type for an order type which specifies a trailing stop/trigger or limit price.
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum TrailingOffsetType {
    /// No trailing offset type is specified (invalid for trailing type orders).
    #[default]
    NoTrailingOffset = 0,
    /// The trailing offset is based on a market price.
    Price = 1,
    /// The trailing offset is based on a percentage represented in basis points, of a market price.
    BasisPoints = 2,
    /// The trailing offset is based on the number of ticks from a market price.
    Ticks = 3,
    /// The trailing offset is based on a price tier set by a specific trading venue.
    PriceTier = 4,
}

/// The trigger type for the stop/trigger price of an order.
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
    AsRefStr,
    FromRepr,
    EnumIter,
    EnumString,
)]
#[strum(ascii_case_insensitive)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.model.enums",
        from_py_object,
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum TriggerType {
    /// No trigger type is specified (invalid for orders with a trigger).
    #[default]
    NoTrigger = 0,
    /// The default trigger type set by the trading venue.
    Default = 1,
    /// Based on the last traded price for the instrument.
    LastPrice = 2,
    /// Based on the mark price for the instrument.
    MarkPrice = 3,
    /// Based on the index price for the instrument.
    IndexPrice = 4,
    /// Based on the top-of-book quoted prices for the instrument.
    BidAsk = 5,
    /// Based on a 'double match' of the last traded price for the instrument
    DoubleLast = 6,
    /// Based on a 'double match' of the bid/ask price for the instrument
    DoubleBidAsk = 7,
    /// Based on both the [`TriggerType::LastPrice`] and [`TriggerType::BidAsk`].
    LastOrBidAsk = 8,
    /// Based on the mid-point of the [`TriggerType::BidAsk`].
    MidPoint = 9,
}

enum_strum_serde!(AccountType);
enum_strum_serde!(AggregationSource);
enum_strum_serde!(AggressorSide);
enum_strum_serde!(AssetClass);
enum_strum_serde!(BarAggregation);
enum_strum_serde!(BarIntervalType);
enum_strum_serde!(BookAction);
enum_strum_serde!(BookType);
enum_strum_serde!(ContingencyType);
enum_strum_serde!(CurrencyType);
enum_strum_serde!(InstrumentClass);
enum_strum_serde!(InstrumentCloseType);
enum_strum_serde!(LiquiditySide);
enum_strum_serde!(MarketStatus);
enum_strum_serde!(MarketStatusAction);
enum_strum_serde!(OmsType);
enum_strum_serde!(OptionKind);
enum_strum_serde!(OrderSide);
enum_strum_serde!(OrderSideSpecified);
enum_strum_serde!(OrderStatus);
enum_strum_serde!(OrderType);
enum_strum_serde!(PositionAdjustmentType);
enum_strum_serde!(PositionSide);
enum_strum_serde!(PositionSideSpecified);
enum_strum_serde!(PriceType);
enum_strum_serde!(RecordFlag);
enum_strum_serde!(TimeInForce);
enum_strum_serde!(TradingState);
enum_strum_serde!(TrailingOffsetType);
enum_strum_serde!(TriggerType);
