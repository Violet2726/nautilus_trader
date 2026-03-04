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

//! 用于追踪 ID 和键值对的有界 FIFO（先进先出）缓存，支持 O(1) 查询。

use std::{fmt::Debug, hash::Hash};

use ahash::{AHashMap, AHashSet};
use arraydeque::ArrayDeque;

/// 一个维持一组 ID 并支持 O(1) 查询的有界缓存。
///
/// 使用 `ArrayDeque` 维持 FIFO 顺序，使用 `AHashSet` 进行快速成员检查。
/// 当超过容量时，最旧的条目将自动被逐出。
///
/// # 示例
///
/// ```
/// use nautilus_common::cache::fifo::FifoCache;
///
/// let mut cache: FifoCache<u32, 3> = FifoCache::new();
/// cache.add(1);
/// cache.add(2);
/// cache.add(3);
/// assert!(cache.contains(&1));
///
/// // 超过容量时，逐出最旧的条目
/// cache.add(4);
/// assert!(!cache.contains(&1));
/// assert!(cache.contains(&4));
/// ```
///
/// 容量为零会导致编译错误：
///
/// ```compile_fail
/// use nautilus_common::cache::fifo::FifoCache;
///
/// // 编译失败：容量必须大于 0
/// let cache: FifoCache<u32, 0> = FifoCache::new();
/// ```
///
/// 默认实现也强制要求非零容量：
///
/// ```compile_fail
/// use nautilus_common::cache::fifo::FifoCache;
///
/// // 这也会导致编译失败
/// let cache: FifoCache<u32, 0> = FifoCache::default();
/// ```
#[derive(Debug)]
pub struct FifoCache<T, const N: usize>
where
    T: Clone + Debug + Eq + Hash,
{
    order: ArrayDeque<T, N>,
    index: AHashSet<T>,
}

impl<T, const N: usize> FifoCache<T, N>
where
    T: Clone + Debug + Eq + Hash,
{
    /// 创建一个新的空 [`FifoCache`]，容量为 `N`。
    ///
    /// # Panics
    ///
    /// 如果 `N == 0`，则在编译时（或运行时断言）触发 Panic。
    #[must_use]
    pub fn new() -> Self {
        const { assert!(N > 0, "FifoCache 的容量必须大于零") };

        Self {
            order: ArrayDeque::new(),
            index: AHashSet::with_capacity(N),
        }
    }

    /// 返回缓存的容量。
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// 返回缓存中 ID 的数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// 返回缓存是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// 返回缓存是否包含给定的 ID（O(1) 查询）。
    #[must_use]
    pub fn contains(&self, id: &T) -> bool {
        self.index.contains(id)
    }

    /// 向缓存添加一个 ID。
    ///
    /// 如果 ID 已存在，则不执行任何操作。
    /// 如果缓存已满，则逐出最旧的条目。
    pub fn add(&mut self, id: T) {
        if self.index.contains(&id) {
            return;
        }

        if self.order.is_full()
            && let Some(evicted) = self.order.pop_back()
        {
            self.index.remove(&evicted);
        }

        if self.order.push_front(id.clone()).is_ok() {
            self.index.insert(id);
        }
    }

    /// 从缓存中移除一个 ID。
    pub fn remove(&mut self, id: &T) {
        if self.index.remove(id) {
            self.order.retain(|x| x != id);
        }
    }

    /// 清除缓存中的所有条目。
    pub fn clear(&mut self) {
        self.order.clear();
        self.index.clear();
    }
}

impl<T, const N: usize> Default for FifoCache<T, N>
where
    T: Clone + Debug + Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

/// 一个支持 O(1) 查询并维持键值对的有界缓存。
///
/// 使用 `ArrayDeque` 维持 FIFO 顺序，使用 `AHashMap` 进行快速的键值访问。
/// 当超过容量时，最旧的条目将自动被逐出。
///
/// # 示例
///
/// ```
/// use nautilus_common::cache::fifo::FifoCacheMap;
///
/// let mut cache: FifoCacheMap<u32, String, 3> = FifoCacheMap::new();
/// cache.insert(1, "one".to_string());
/// cache.insert(2, "two".to_string());
/// cache.insert(3, "three".to_string());
/// assert_eq!(cache.get(&1), Some(&"one".to_string()));
///
/// // 超过容量时逐出最旧的条目
/// cache.insert(4, "four".to_string());
/// assert_eq!(cache.get(&1), None);
/// assert_eq!(cache.get(&4), Some(&"four".to_string()));
/// ```
///
/// 容量为零会导致编译错误：
///
/// ```compile_fail
/// use nautilus_common::cache::fifo::FifoCacheMap;
///
/// // 编译失败：容量必须大于 0
/// let cache: FifoCacheMap<u32, String, 0> = FifoCacheMap::new();
/// ```
#[derive(Debug)]
pub struct FifoCacheMap<K, V, const N: usize>
where
    K: Clone + Debug + Eq + Hash,
{
    order: ArrayDeque<K, N>,
    index: AHashMap<K, V>,
}

impl<K, V, const N: usize> FifoCacheMap<K, V, N>
where
    K: Clone + Debug + Eq + Hash,
{
    /// 创建一个新的空 [`FifoCacheMap`]，容量为 `N`。
    ///
    /// # Panics
    ///
    /// 如果 `N == 0`，则在编译时触发 Panic。
    #[must_use]
    pub fn new() -> Self {
        const { assert!(N > 0, "FifoCacheMap 的容量必须大于零") };

        Self {
            order: ArrayDeque::new(),
            index: AHashMap::with_capacity(N),
        }
    }

    /// 返回缓存的容量。
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// 返回缓存中的条目数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// 返回缓存是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// 返回缓存是否包含给定的键（O(1) 查询）。
    #[must_use]
    pub fn contains_key(&self, key: &K) -> bool {
        self.index.contains_key(key)
    }

    /// 返回给定键对应的数值引用（O(1) 查询）。
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        self.index.get(key)
    }

    /// 返回给定键对应的可变数值引用（O(1) 查询）。
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.index.get_mut(key)
    }

    /// 向缓存插入一个键值对。
    ///
    /// 如果键已存在，则更新值（不发生逐出）。
    /// 如果缓存已满且键是新的，则逐出最旧的条目。
    pub fn insert(&mut self, key: K, value: V) {
        if self.index.contains_key(&key) {
            self.index.insert(key, value);
            return;
        }

        if self.order.is_full()
            && let Some(evicted) = self.order.pop_back()
        {
            self.index.remove(&evicted);
        }

        if self.order.push_front(key.clone()).is_ok() {
            self.index.insert(key, value);
        }
    }

    /// 从缓存中移除一个键，如果键存在则返回其对应的值。
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(value) = self.index.remove(key) {
            self.order.retain(|x| x != key);
            Some(value)
        } else {
            None
        }
    }

    /// 清除缓存中的所有条目。
    pub fn clear(&mut self) {
        self.order.clear();
        self.index.clear();
    }
}

impl<K, V, const N: usize> Default for FifoCacheMap<K, V, N>
where
    K: Clone + Debug + Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_add_and_contains() {
        let mut cache: FifoCache<u32, 4> = FifoCache::new();
        cache.add(1);
        cache.add(2);
        cache.add(3);

        assert!(cache.contains(&1));
        assert!(cache.contains(&2));
        assert!(cache.contains(&3));
        assert!(!cache.contains(&4));
        assert_eq!(cache.len(), 3);
    }

    #[rstest]
    fn test_eviction_at_capacity() {
        let mut cache: FifoCache<u32, 3> = FifoCache::new();
        cache.add(1);
        cache.add(2);
        cache.add(3);
        assert_eq!(cache.len(), 3);

        // 添加第 4 个元素应该逐出最旧的 (1)
        cache.add(4);
        assert_eq!(cache.len(), 3);
        assert!(!cache.contains(&1));
        assert!(cache.contains(&2));
        assert!(cache.contains(&3));
        assert!(cache.contains(&4));
    }

    #[rstest]
    fn test_duplicate_add_is_noop() {
        let mut cache: FifoCache<u32, 3> = FifoCache::new();
        cache.add(1);
        cache.add(2);
        cache.add(1); // 重复添加

        assert_eq!(cache.len(), 2);
        assert!(cache.contains(&1));
        assert!(cache.contains(&2));
    }

    #[rstest]
    fn test_remove() {
        let mut cache: FifoCache<u32, 4> = FifoCache::new();
        cache.add(1);
        cache.add(2);
        cache.add(3);

        cache.remove(&2);
        assert_eq!(cache.len(), 2);
        assert!(cache.contains(&1));
        assert!(!cache.contains(&2));
        assert!(cache.contains(&3));
    }

    #[rstest]
    fn test_remove_nonexistent_is_noop() {
        let mut cache: FifoCache<u32, 4> = FifoCache::new();
        cache.add(1);
        cache.remove(&99);
        assert_eq!(cache.len(), 1);
    }

    #[rstest]
    fn test_capacity() {
        let cache: FifoCache<u32, 10> = FifoCache::new();
        assert_eq!(cache.capacity(), 10);
    }

    #[rstest]
    fn test_is_empty() {
        let mut cache: FifoCache<u32, 4> = FifoCache::new();
        assert!(cache.is_empty());
        cache.add(1);
        assert!(!cache.is_empty());
    }

    #[rstest]
    fn test_capacity_one_evicts_immediately() {
        let mut cache: FifoCache<u32, 1> = FifoCache::new();
        cache.add(1);
        assert!(cache.contains(&1));
        assert_eq!(cache.len(), 1);

        cache.add(2);
        assert!(!cache.contains(&1));
        assert!(cache.contains(&2));
        assert_eq!(cache.len(), 1);
    }

    #[rstest]
    fn test_sequential_eviction_order() {
        let mut cache: FifoCache<u32, 3> = FifoCache::new();

        // 填充：[3, 2, 1]（从前到后）
        cache.add(1);
        cache.add(2);
        cache.add(3);

        // 添加 4：逐出 1 -> [4, 3, 2]
        cache.add(4);
        assert!(!cache.contains(&1));
        assert!(cache.contains(&2));

        // 添加 5：逐出 2 -> [5, 4, 3]
        cache.add(5);
        assert!(!cache.contains(&2));
        assert!(cache.contains(&3));

        // 添加 6：逐出 3 -> [6, 5, 4]
        cache.add(6);
        assert!(!cache.contains(&3));
        assert!(cache.contains(&4));
        assert!(cache.contains(&5));
        assert!(cache.contains(&6));
    }

    #[rstest]
    fn test_remove_then_readd() {
        let mut cache: FifoCache<u32, 3> = FifoCache::new();
        cache.add(1);
        cache.add(2);
        cache.remove(&1);
        assert!(!cache.contains(&1));
        assert_eq!(cache.len(), 1);

        cache.add(1);
        assert!(cache.contains(&1));
        assert_eq!(cache.len(), 2);
    }

    #[rstest]
    fn test_remove_frees_slot_for_new_element() {
        let mut cache: FifoCache<u32, 3> = FifoCache::new();

        cache.add(1);
        cache.add(2);
        cache.add(3);
        cache.remove(&2);
        assert_eq!(cache.len(), 2);

        // 添加新元素 - 不应该逐出任何人
        cache.add(4);
        assert_eq!(cache.len(), 3);
        assert!(cache.contains(&1));
        assert!(cache.contains(&3));
        assert!(cache.contains(&4));
    }

    #[rstest]
    fn test_duplicate_add_does_not_refresh_position() {
        let mut cache: FifoCache<u32, 3> = FifoCache::new();

        // 添加 1, 2, 3 (1 是最旧的)
        cache.add(1);
        cache.add(2);
        cache.add(3);

        // 重新添加 1 (应该是无操作，1 保持最旧)
        cache.add(1);

        // 添加 4：应该逐出 1 (仍然是最旧的)，而不是 2
        cache.add(4);
        assert!(!cache.contains(&1));
        assert!(cache.contains(&2));
        assert!(cache.contains(&3));
        assert!(cache.contains(&4));
    }

    #[rstest]
    fn test_interleaved_add_remove() {
        let mut cache: FifoCache<u32, 4> = FifoCache::new();

        cache.add(1);
        cache.add(2);
        cache.remove(&1);
        cache.add(3);
        cache.add(4);
        cache.remove(&3);
        cache.add(5);

        assert!(!cache.contains(&1));
        assert!(cache.contains(&2));
        assert!(!cache.contains(&3));
        assert!(cache.contains(&4));
        assert!(cache.contains(&5));
        assert_eq!(cache.len(), 3);
    }

    #[rstest]
    fn test_remove_all_elements() {
        let mut cache: FifoCache<u32, 3> = FifoCache::new();
        cache.add(1);
        cache.add(2);
        cache.add(3);

        cache.remove(&1);
        cache.remove(&2);
        cache.remove(&3);

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[rstest]
    fn test_string_type() {
        let mut cache: FifoCache<String, 2> = FifoCache::new();
        cache.add("hello".to_string());
        cache.add("world".to_string());

        assert!(cache.contains(&"hello".to_string()));
        assert!(cache.contains(&"world".to_string()));

        cache.add("foo".to_string());
        assert!(!cache.contains(&"hello".to_string()));
    }

    #[rstest]
    fn test_map_insert_and_get() {
        let mut cache: FifoCacheMap<u32, String, 4> = FifoCacheMap::new();
        cache.insert(1, "one".to_string());
        cache.insert(2, "two".to_string());
        cache.insert(3, "three".to_string());

        assert_eq!(cache.get(&1), Some(&"one".to_string()));
        assert_eq!(cache.get(&2), Some(&"two".to_string()));
        assert_eq!(cache.get(&3), Some(&"three".to_string()));
        assert_eq!(cache.get(&4), None);
        assert_eq!(cache.len(), 3);
    }

    #[rstest]
    fn test_map_eviction_at_capacity() {
        let mut cache: FifoCacheMap<u32, &str, 3> = FifoCacheMap::new();
        cache.insert(1, "one");
        cache.insert(2, "two");
        cache.insert(3, "three");
        assert_eq!(cache.len(), 3);

        // 添加第 4 个应该逐出最旧的 (1)
        cache.insert(4, "four");
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), Some(&"two"));
        assert_eq!(cache.get(&3), Some(&"three"));
        assert_eq!(cache.get(&4), Some(&"four"));
    }

    #[rstest]
    fn test_map_update_existing_key() {
        let mut cache: FifoCacheMap<u32, &str, 3> = FifoCacheMap::new();
        cache.insert(1, "one");
        cache.insert(2, "two");
        cache.insert(3, "three");

        // Update existing key - should not evict
        cache.insert(1, "ONE");
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get(&1), Some(&"ONE"));
        assert_eq!(cache.get(&2), Some(&"two"));
        assert_eq!(cache.get(&3), Some(&"three"));
    }

    #[rstest]
    fn test_map_remove() {
        let mut cache: FifoCacheMap<u32, &str, 4> = FifoCacheMap::new();
        cache.insert(1, "one");
        cache.insert(2, "two");
        cache.insert(3, "three");

        let removed = cache.remove(&2);
        assert_eq!(removed, Some("two"));
        assert_eq!(cache.len(), 2);
        assert!(cache.contains_key(&1));
        assert!(!cache.contains_key(&2));
        assert!(cache.contains_key(&3));
    }

    #[rstest]
    fn test_map_remove_nonexistent() {
        let mut cache: FifoCacheMap<u32, &str, 4> = FifoCacheMap::new();
        cache.insert(1, "one");
        let removed = cache.remove(&99);
        assert_eq!(removed, None);
        assert_eq!(cache.len(), 1);
    }

    #[rstest]
    fn test_map_get_mut() {
        let mut cache: FifoCacheMap<u32, String, 4> = FifoCacheMap::new();
        cache.insert(1, "one".to_string());

        if let Some(value) = cache.get_mut(&1) {
            value.push_str("_modified");
        }

        assert_eq!(cache.get(&1), Some(&"one_modified".to_string()));
    }

    #[rstest]
    fn test_map_capacity() {
        let cache: FifoCacheMap<u32, &str, 10> = FifoCacheMap::new();
        assert_eq!(cache.capacity(), 10);
    }

    #[rstest]
    fn test_map_is_empty() {
        let mut cache: FifoCacheMap<u32, &str, 4> = FifoCacheMap::new();
        assert!(cache.is_empty());
        cache.insert(1, "one");
        assert!(!cache.is_empty());
    }

    #[rstest]
    fn test_map_capacity_one() {
        let mut cache: FifoCacheMap<u32, &str, 1> = FifoCacheMap::new();
        cache.insert(1, "one");
        assert_eq!(cache.get(&1), Some(&"one"));

        cache.insert(2, "two");
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), Some(&"two"));
        assert_eq!(cache.len(), 1);
    }

    #[rstest]
    fn test_map_sequential_eviction() {
        let mut cache: FifoCacheMap<u32, u32, 3> = FifoCacheMap::new();

        cache.insert(1, 10);
        cache.insert(2, 20);
        cache.insert(3, 30);

        // 添加 4：逐出 1
        cache.insert(4, 40);
        assert!(!cache.contains_key(&1));
        assert!(cache.contains_key(&2));

        // 添加 5：逐出 2
        cache.insert(5, 50);
        assert!(!cache.contains_key(&2));
        assert!(cache.contains_key(&3));
    }

    #[rstest]
    fn test_map_update_does_not_change_eviction_order() {
        let mut cache: FifoCacheMap<u32, &str, 3> = FifoCacheMap::new();

        cache.insert(1, "one");
        cache.insert(2, "two");
        cache.insert(3, "three");

        // Update key 1 - should NOT move it to front
        cache.insert(1, "ONE");

        // Add new key - should still evict 1 (oldest by insertion order)
        cache.insert(4, "four");
        assert!(!cache.contains_key(&1));
        assert!(cache.contains_key(&2));
        assert!(cache.contains_key(&3));
        assert!(cache.contains_key(&4));
    }

    #[rstest]
    fn test_map_remove_frees_slot() {
        let mut cache: FifoCacheMap<u32, &str, 3> = FifoCacheMap::new();

        cache.insert(1, "one");
        cache.insert(2, "two");
        cache.insert(3, "three");

        cache.remove(&2);
        assert_eq!(cache.len(), 2);

        // 添加新元素 - 不应该逐出任何人
        cache.insert(4, "four");
        assert_eq!(cache.len(), 3);
        assert!(cache.contains_key(&1));
        assert!(cache.contains_key(&3));
        assert!(cache.contains_key(&4));
    }

    use proptest::prelude::*;

    /// 可以在 FifoCache 上执行的操作
    #[derive(Clone, Debug)]
    enum Op {
        Add(u8),
        Remove(u8),
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![(0..50u8).prop_map(Op::Add), (0..50u8).prop_map(Op::Remove),]
    }

    fn ops_strategy() -> impl Strategy<Value = Vec<Op>> {
        proptest::collection::vec(op_strategy(), 0..100)
    }

    /// 执行操作并返回最终的缓存状态
    fn apply_ops<const N: usize>(ops: &[Op]) -> FifoCache<u8, N> {
        let mut cache = FifoCache::<u8, N>::new();
        for op in ops {
            match op {
                Op::Add(id) => cache.add(*id),
                Op::Remove(id) => cache.remove(id),
            }
        }
        cache
    }

    proptest! {
        /// 不变性：len() 绝不超过容量
        #[rstest]
        fn prop_len_never_exceeds_capacity(ops in ops_strategy()) {
            let cache = apply_ops::<8>(&ops);
            prop_assert!(cache.len() <= cache.capacity());
        }

        /// 不变性：is_empty() 当且仅当 len() == 0
        #[rstest]
        fn prop_is_empty_consistent_with_len(ops in ops_strategy()) {
            let cache = apply_ops::<8>(&ops);
            if cache.is_empty() {
                prop_assert_eq!(cache.len(), 0);
            } else {
                prop_assert!(!cache.is_empty());
            }
        }

        /// 不变性：添加重复项不会改变 len
        #[rstest]
        fn prop_add_duplicate_is_idempotent(
            ops in ops_strategy(),
            id in 0..50u8
        ) {
            let mut cache = apply_ops::<8>(&ops);
            cache.add(id);
            let len_after_first = cache.len();
            let contained_after_first = cache.contains(&id);

            cache.add(id);
            prop_assert_eq!(cache.len(), len_after_first);
            prop_assert_eq!(cache.contains(&id), contained_after_first);
        }

        /// 不变性：执行 remove(x) 后，contains(x) 为假
        #[rstest]
        fn prop_remove_ensures_not_contained(
            ops in ops_strategy(),
            id in 0..50u8
        ) {
            let mut cache = apply_ops::<8>(&ops);
            cache.remove(&id);
            prop_assert!(!cache.contains(&id));
        }

        /// 不变性：执行 add(x) 后，contains(x) 为真 (除非立即被逐出)
        #[rstest]
        fn prop_add_ensures_contained_if_capacity(id in 0..50u8) {
            let mut cache: FifoCache<u8, 8> = FifoCache::new();
            cache.add(id);
            prop_assert!(cache.contains(&id));
        }

        /// 不变性：FIFO 逐出顺序 - 最旧的元素首先被逐出
        #[rstest]
        fn prop_fifo_eviction_order(extra in 0..20u8) {
            let mut cache: FifoCache<u8, 4> = FifoCache::new();

            // 用 0, 1, 2, 3 填满缓存
            for i in 0..4u8 {
                cache.add(i);
            }
            prop_assert_eq!(cache.len(), 4);

            // 添加更多元素，应该按 FIFO 顺序逐出
            for i in 0..extra {
                let new_id = 100 + i;
                cache.add(new_id);

                // 应该已被逐出的元素
                let evicted = i;
                if evicted < 4 {
                    prop_assert!(!cache.contains(&evicted),
                        "元素 {} 应该已被逐出", evicted);
                }
            }
        }

        /// 不变性：在空缓存上执行 remove 是安全的无操作
        #[rstest]
        fn prop_remove_on_empty_is_noop(id in 0..50u8) {
            let mut cache: FifoCache<u8, 8> = FifoCache::new();
            cache.remove(&id);
            prop_assert!(cache.is_empty());
            prop_assert_eq!(cache.len(), 0);
        }

        /// 不变性：移除现有元素时 len() 减少 1
        #[rstest]
        fn prop_remove_decreases_len(
            ops in ops_strategy(),
            id in 0..50u8
        ) {
            let mut cache = apply_ops::<8>(&ops);
            cache.add(id); // 确保其存在
            let len_before = cache.len();

            cache.remove(&id);

            if cache.contains(&id) {
                prop_assert!(false, "移除后元素仍包含在缓存中");
            }
            prop_assert!(cache.len() < len_before || len_before == 0);
        }

        /// 不变性：在达到容量时，添加新元素保持 len 不变
        #[rstest]
        fn prop_add_at_capacity_maintains_len(new_id in 50..100u8) {
            let mut cache: FifoCache<u8, 4> = FifoCache::new();

            // 用不同的值填满至容量
            for i in 0..4u8 {
                cache.add(i);
            }
            prop_assert_eq!(cache.len(), 4);

            // 添加新元素 (保证不在缓存中)
            cache.add(new_id);
            prop_assert_eq!(cache.len(), 4);
        }

        /// 不变性：所有添加的元素直到被逐出或移除前都包含在缓存中
        #[rstest]
        fn prop_recent_adds_are_contained(recent in proptest::collection::vec(0..50u8, 1..5)) {
            let mut cache: FifoCache<u8, 8> = FifoCache::new();

            for &id in &recent {
                cache.add(id);
            }

            // 去重以获取期望的唯一计数
            let mut unique: Vec<u8> = recent;
            unique.sort_unstable();
            unique.dedup();
            let expected_len = unique.len().min(8);

            prop_assert_eq!(cache.len(), expected_len);

            // 所有唯一的最近添加项都应该包含在缓存中 (容量为 8，我们最多添加 5 个)
            for id in unique {
                prop_assert!(cache.contains(&id), "最近添加的 {} 未包含在缓存中", id);
            }
        }

        /// 不变性：Map 的 len() 绝不超过容量
        #[rstest]
        fn prop_map_len_never_exceeds_capacity(
            keys in proptest::collection::vec(0..50u8, 0..100)
        ) {
            let mut cache: FifoCacheMap<u8, u8, 8> = FifoCacheMap::new();
            for key in keys {
                cache.insert(key, key);
            }
            prop_assert!(cache.len() <= cache.capacity());
        }

        /// 不变性：Map 的 is_empty() 当且仅当 len() == 0
        #[rstest]
        fn prop_map_is_empty_consistent_with_len(
            keys in proptest::collection::vec(0..50u8, 0..20)
        ) {
            let mut cache: FifoCacheMap<u8, u8, 8> = FifoCacheMap::new();
            for key in keys {
                cache.insert(key, key);
            }
            if cache.is_empty() {
                prop_assert_eq!(cache.len(), 0);
            } else {
                prop_assert!(!cache.is_empty());
            }
        }

        /// 不变性：更新现有键不会改变 len
        #[rstest]
        fn prop_map_update_is_idempotent_for_len(
            keys in proptest::collection::vec(0..50u8, 1..10),
            key in 0..50u8
        ) {
            let mut cache: FifoCacheMap<u8, u8, 8> = FifoCacheMap::new();
            for k in keys {
                cache.insert(k, k);
            }
            cache.insert(key, 100);
            let len_after_first = cache.len();

            cache.insert(key, 200);
            prop_assert_eq!(cache.len(), len_after_first);
        }

        /// 不变性：执行 remove(k) 后，get(k) 为 None
        #[rstest]
        fn prop_map_remove_ensures_not_contained(
            keys in proptest::collection::vec(0..50u8, 0..20),
            key in 0..50u8
        ) {
            let mut cache: FifoCacheMap<u8, u8, 8> = FifoCacheMap::new();
            for k in keys {
                cache.insert(k, k);
            }
            cache.remove(&key);
            prop_assert!(cache.get(&key).is_none());
        }

        /// 不变性：执行 insert(k, v) 后，get(k) 返回 Some(&v)
        #[rstest]
        fn prop_map_insert_ensures_get(key in 0..50u8, value in 0..100u8) {
            let mut cache: FifoCacheMap<u8, u8, 8> = FifoCacheMap::new();
            cache.insert(key, value);
            prop_assert_eq!(cache.get(&key), Some(&value));
        }

        /// 不变性：在达到容量时，插入新键保持 len 不变
        #[rstest]
        fn prop_map_insert_at_capacity_maintains_len(new_key in 50..100u8) {
            let mut cache: FifoCacheMap<u8, u8, 4> = FifoCacheMap::new();

            for i in 0..4u8 {
                cache.insert(i, i * 10);
            }
            prop_assert_eq!(cache.len(), 4);

            cache.insert(new_key, 99);
            prop_assert_eq!(cache.len(), 4);
        }

        /// 不变性：Map 的 FIFO 逐出
        #[rstest]
        fn prop_map_fifo_eviction(extra in 0..20u8) {
            let mut cache: FifoCacheMap<u8, u8, 4> = FifoCacheMap::new();

            for i in 0..4u8 {
                cache.insert(i, i * 10);
            }

            for i in 0..extra {
                let new_key = 100 + i;
                cache.insert(new_key, new_key);

                let evicted = i;
                if evicted < 4 {
                    prop_assert!(cache.get(&evicted).is_none(),
                        "键 {} 应该已被逐出", evicted);
                }
            }
        }
    }
}
