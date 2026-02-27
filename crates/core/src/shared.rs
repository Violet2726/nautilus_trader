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

//! 对频繁使用的 `Rc<RefCell<T>>` / `Weak<RefCell<T>>` 组合进行了高效且易用的封装。
//!
//! NautilusTrader 码库极大地依赖于许多引擎组件间的共享所有权和内部可变性 (`Rc<RefCell<T>>`)。
//! 在许多 API 中重复冗长的类型及其弱引用（Weak）副本会让代码变得杂乱，并且容易让人不小心存储强引用而非弱引用（导致循环引用）。
//!
//! `SharedCell<T>` 和 `WeakCell<T>` 是零成本的新类型（Newtype），它们使代码意图更加明确，
//! 并提供了如 `downgrade`、`upgrade`、`borrow`、`borrow_mut` 等便捷工具函数。因为这些封装器使用了 `#[repr(transparent)]` 特性，
//! 所以它们与封装的 `Rc` / `Weak` 具有完全相同的内存布局，且不引入运行时开销。

//! ## 如何在 `SharedCell` 与 `WeakCell` 之间做出选择
//!
//! * 当当前所有者确实 *拥有*（或共同拥有）该值时，应使用 **`SharedCell<T>`** ——
//!   就像你通常存储 `Rc<RefCell<T>>` 那样。
//! * 对于可能形成循环引用的反向引用，应使用 **`WeakCell<T>`**。
//!   反向指针 **不会** 保持该值的存活状态，且每次访问前都必须先
//!   `upgrade()` 为强引用 `SharedCell`。我们使用这种模式来打破循环所有权，
//!   例如 *交易平台 (Exchange) ↔ `ExecutionClient`*：交易平台持有指向客户端的 `SharedCell`，
//!   而客户端仅持有指向交易平台的 `WeakCell`。

use std::{
    cell::{BorrowError, BorrowMutError, Ref, RefCell, RefMut},
    rc::{Rc, Weak},
};

/// 带有内部可变性，对 `T` 的强共享所有权。
#[repr(transparent)]
#[derive(Debug)]
pub struct SharedCell<T>(Rc<RefCell<T>>);

impl<T> Clone for SharedCell<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> SharedCell<T> {
    /// 将一个值包装在 `Rc<RefCell<..>>` 之中。
    #[inline]
    pub fn new(value: T) -> Self {
        Self(Rc::new(RefCell::new(value)))
    }

    /// 创建一个指向同一分配空间的 [`WeakCell`]。
    #[inline]
    #[must_use]
    pub fn downgrade(&self) -> WeakCell<T> {
        WeakCell(Rc::downgrade(&self.0))
    }

    /// 对内部值进行不可变借用。
    #[inline]
    #[must_use]
    pub fn borrow(&self) -> Ref<'_, T> {
        self.0.borrow()
    }

    /// 对内部值进行可变借用。
    #[inline]
    #[must_use]
    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.0.borrow_mut()
    }

    /// 尝试对内部值进行不可变借用。
    ///
    /// 如果该值当前已被可变借用，则返回 `Err`。
    #[inline]
    pub fn try_borrow(&self) -> Result<Ref<'_, T>, BorrowError> {
        self.0.try_borrow()
    }

    /// 尝试对内部值进行可变借用。
    ///
    /// 如果该值当前已被借用（无论是可变借用还是不可变借用），则返回 `Err`。
    #[inline]
    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, T>, BorrowMutError> {
        self.0.try_borrow_mut()
    }

    /// 活跃强引用的数量。
    #[inline]
    #[must_use]
    pub fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    /// 活跃弱引用的数量。
    #[inline]
    #[must_use]
    pub fn weak_count(&self) -> usize {
        Rc::weak_count(&self.0)
    }
}

impl<T> From<Rc<RefCell<T>>> for SharedCell<T> {
    fn from(inner: Rc<RefCell<T>>) -> Self {
        Self(inner)
    }
}

impl<T> From<SharedCell<T>> for Rc<RefCell<T>> {
    fn from(shared: SharedCell<T>) -> Self {
        shared.0
    }
}

impl<T> std::ops::Deref for SharedCell<T> {
    type Target = Rc<RefCell<T>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// [`SharedCell`] 对等的弱引用。
#[repr(transparent)]
#[derive(Debug)]
pub struct WeakCell<T>(Weak<RefCell<T>>);

impl<T> Clone for WeakCell<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> WeakCell<T> {
    /// 尝试将弱引用升级为强引用 [`SharedCell`]。
    #[inline]
    pub fn upgrade(&self) -> Option<SharedCell<T>> {
        self.0.upgrade().map(SharedCell)
    }

    /// 如果所指的值已被销毁 (dropped)，则返回 `true`。
    #[inline]
    #[must_use]
    pub fn is_dropped(&self) -> bool {
        self.0.strong_count() == 0
    }
}

impl<T> From<Weak<RefCell<T>>> for WeakCell<T> {
    fn from(inner: Weak<RefCell<T>>) -> Self {
        Self(inner)
    }
}

impl<T> From<WeakCell<T>> for Weak<RefCell<T>> {
    fn from(cell: WeakCell<T>) -> Self {
        cell.0
    }
}
