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

//! 用于在 FFI 边界之间传输堆分配的 Rust `Vec<T>` 值的工具函数。
//!
//! 此模块提供的主要抽象是 `CVec`，这是一个 C 兼容的结构体，用于存储
//! 原始指针 (`ptr`) 以及向量的逻辑长度 (`len`) 和容量 (`cap`)。通过将
//! 分配元数据移动到普通的 `repr(C)` 类型中，我们允许 Rust 创建的内存在外部代码中
//! 被拥有、检查，并最终释放（反之亦然），而不会引入未定义行为。
//!
//! 仅向 C 暴露了非常小的 API 表面：
//!
//! * `cvec_new` – 创建一个空的 `CVec` 哨兵，可返回给外部代码。
//!
//! 该模块有意地**不**通过泛型辅助函数提供解除分配的功能。相反，每个 FFI 模块
//! 必须公开其自己的*特定类型*的 `vec_*_drop` 函数，该函数使用 [`Vec::from_raw_parts`]
//! 重构原始 `Vec<T>` 并允许其被销毁。这避免了过去“一站式” `cvec_drop` 存在的尺寸不匹配风险。
//!
//! 所有其他操作都在 Rust 侧交出所有权之前发生。这使得内存安全规则变得简单明了：
//! 外部调用方必须将 `ptr` 指向的内存区域视为**不透明的 (opaque)**，且仅通过此处提供的函数与之交互。

use std::{ffi::c_void, fmt::Display, ptr::NonNull};

use crate::ffi::abort_on_panic;

/// `CVec` 是一个 C 兼容的结构体，存储指向内存块的一个不透明指针，
/// 及其长度和分配该向量时的容量。
///
/// # 安全性 (Safety)
///
/// 更改此处的数值可能会导致在销毁内存时产生未定义行为。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CVec {
    /// 指向存放元素内存块的不透明指针。若要访问元素，需将其转换为底层类型。
    pub ptr: *mut c_void,
    /// 块中元素的数量。
    pub len: usize,
    /// 分配该向量时的容量。
    /// 在解除内存分配时使用。
    pub cap: usize,
}

// 安全性：CVec 被标记为 Send 以满足 PyO3 的 PyCapsule 要求，
// 后者需要跨 Python/Rust 边界传输所有权。然而，CVec 包含原始指针，
// 且仅在单线程语境下或具有外部同步保证时方可安全使用。
//
// 实现 Send 是出于以下需求：
// 1. PyO3 的 PyCapsule::new_with_destructor 带有 Send 约束条件。
// 2. 将 CVec 的所有权传输给 Python（其在单一的、受 GIL 保护的线程上运行）。
//
// 重要提示：在发送 CVec 实例跨线程前，请确保：
// - 底层数据类型 T 本身即实现了 Send + Sync。
// - 适当的外部同步机制（如互斥锁 mutex）保护了并发访问。
// - CVec 在将被重构的同一线程上被消耗。
//
// 在实践中，本码库中的 CVec 使用仅限于 Python FFI 边界，此处 Python GIL 提供了必要的同步。
unsafe impl Send for CVec {}

impl CVec {
    /// 返回一个空的 [`CVec`]。
    ///
    /// 这主要用于构造一个哨兵值，用以表示跨越 FFI 边界时的数据缺失。
    ///
    /// 使用悬空指针（类似于 `Vec::new()`）而非 null，以满足之后销毁 CVec 时
    /// `Vec::from_raw_parts` 的前置条件。
    #[must_use]
    pub fn empty() -> Self {
        Self {
            ptr: NonNull::<u8>::dangling().as_ptr().cast::<c_void>(),
            len: 0,
            cap: 0,
        }
    }
}

/// 消耗并泄漏 (leak) 该 Vec，将其内容的字段作为 [`CVec`] 以可变指针形式返回。
/// 该内存已经被泄漏，且现在除非手动销毁，否则将在程序的生命周期内一直存在。
/// 注意：通过如下文测试中所示的使用 `from_raw_parts` 方法重构该 vec 来销毁内存。
impl<T> From<Vec<T>> for CVec {
    fn from(mut data: Vec<T>) -> Self {
        if data.is_empty() {
            Self::empty()
        } else {
            let len = data.len();
            let cap = data.capacity();
            let ptr = data.as_mut_ptr();
            std::mem::forget(data);
            Self {
                ptr: ptr.cast::<std::ffi::c_void>(),
                len,
                cap,
            }
        }
    }
}

impl Display for CVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CVec {{ ptr: {:?}, len: {}, cap: {} }}",
            self.ptr, self.len, self.cap,
        )
    }
}

////////////////////////////////////////////////////////////////////////////////
// C API
////////////////////////////////////////////////////////////////////////////////

/// 构造一个新的*空* [`CVec`] 值，用作外部代码中的初始化程序或哨兵。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn cvec_new() -> CVec {
    abort_on_panic(CVec::empty)
}

#[cfg(test)]
mod tests {
    use rstest::*;

    use super::CVec;

    /// 访问转换成 [`CVec`] 的向量中的值。
    #[rstest]
    #[allow(unused_assignments)]
    fn access_values_test() {
        let test_data = vec![1_u64, 2, 3];
        let mut vec_len = 0;
        let mut vec_cap = 0;
        let cvec: CVec = {
            let data = test_data.clone();
            vec_len = data.len();
            vec_cap = data.capacity();
            data.into()
        };

        let CVec { ptr, len, cap } = cvec;
        assert_eq!(len, vec_len);
        assert_eq!(cap, vec_cap);

        let data = ptr.cast::<u64>();
        unsafe {
            assert_eq!(*data, test_data[0]);
            assert_eq!(*data.add(1), test_data[1]);
            assert_eq!(*data.add(2), test_data[2]);
        }

        unsafe {
            // 重构该结构体并销毁内存以释放空间
            let _ = Vec::from_raw_parts(ptr.cast::<u64>(), len, cap);
        }
    }

    /// 空向量在 [`CVec`] 中会被转换为悬空（非 null）指针。
    #[rstest]
    fn empty_vec_should_give_dangling_ptr() {
        let data: Vec<u64> = vec![];
        let cvec: CVec = data.into();
        assert!(!cvec.ptr.is_null());
        assert_eq!(cvec.len, 0);
        assert_eq!(cvec.cap, 0);
    }
}
