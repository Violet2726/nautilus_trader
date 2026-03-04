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

//! 具有生命周期安全访问守卫 (access guards) 的线程本地 Actor 注册表。
//!
//! # 设计
//!
//! Actor 注册表将 Actor 存储在线程本地存储中，并通过 [`ActorRef<T>`] 守卫提供访问。
//! 此设计解决了几个约束：
//!
//! - **防止使用后释放 (Use-after-free prevention)**：`ActorRef` 持有一个 `Rc` 克隆，
//!   即使在守卫存在时从注册表中删除了 Actor，也能保持其存活。
//! - **可重入回调 (Re-entrant callbacks)**：消息处理程序经常回调到注册表以访问其他 Actor。
//!   与 `RefCell` 风格的借用跟踪不同，多个 `ActorRef` 守卫可以同时存在而不会导致 panic。
//! - **没有 `'static` 生命周期谎言**：之前的设计返回 `&'static mut T`，
//!   这并不能反映实际的有效性。基于守卫的方法将借用与守卫的生命周期绑定在一起。
//!
//! # 局限性
//!
//! - **无法防止别名 (Aliasing not prevented)**：同一个 Actor 可以同时存在两个守卫，
//!   从而允许别名的可变访问。这在技术上是未定义行为，但可重入回调模式需要它。
//!   需要更高层级的纪律。
//! - **仅限线程本地**：守卫不得跨线程发送。

use std::{
    any::TypeId,
    cell::{RefCell, UnsafeCell},
    fmt::Debug,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use ahash::AHashMap;
use ustr::Ustr;

use super::Actor;

/// 提供对 Actor 的可变访问的守卫。
///
/// 此守卫持有一个 `Rc` 引用以保持 Actor 存活，从而防止在守卫存在时从注册表中删除 Actor 导致的使用后释放。
/// 该守卫实现了 `Deref` 和 `DerefMut` 以便符合人体工程学的访问。
///
/// # 安全性 (Safety)
///
/// 虽然此守卫可以防止由于从注册表中删除导致的使用后释放，但它并不能防止别名。
/// 同一个 Actor 可以同时存在多个 `ActorRef` 实例，这在技术上是未定义行为，
/// 但本代码库中的可重入回调模式需要这种行为。
pub struct ActorRef<T: Actor> {
    actor_rc: Rc<UnsafeCell<dyn Actor>>,
    _marker: PhantomData<T>,
}

impl<T: Actor> Debug for ActorRef<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(ActorRef))
            .field("actor_id", &self.deref().id())
            .finish()
    }
}

impl<T: Actor> Deref for ActorRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: Type was verified at construction time
        unsafe { &*(self.actor_rc.get() as *const T) }
    }
}

impl<T: Actor> DerefMut for ActorRef<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: Type was verified at construction time
        unsafe { &mut *self.actor_rc.get().cast::<T>() }
    }
}

thread_local! {
    static ACTOR_REGISTRY: ActorRegistry = ActorRegistry::new();
}

/// 用于存储 Actor 的注册表。
pub struct ActorRegistry {
    actors: RefCell<AHashMap<Ustr, Rc<UnsafeCell<dyn Actor>>>>,
}

impl Debug for ActorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let actors_ref = self.actors.borrow();
        let keys: Vec<&Ustr> = actors_ref.keys().collect();
        f.debug_struct(stringify!(ActorRegistry))
            .field("actors", &keys)
            .finish()
    }
}

impl Default for ActorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ActorRegistry {
    pub fn new() -> Self {
        Self {
            actors: RefCell::new(AHashMap::new()),
        }
    }

    pub fn insert(&self, id: Ustr, actor: Rc<UnsafeCell<dyn Actor>>) {
        let mut actors = self.actors.borrow_mut();
        if actors.contains_key(&id) {
            log::warn!("正在替换 ID 为 {id} 的现有 Actor");
        }
        actors.insert(id, actor);
    }

    pub fn get(&self, id: &Ustr) -> Option<Rc<UnsafeCell<dyn Actor>>> {
        self.actors.borrow().get(id).cloned()
    }

    /// 返回注册的 Actor 数量。
    pub fn len(&self) -> usize {
        self.actors.borrow().len()
    }

    /// 检查注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.actors.borrow().is_empty()
    }

    /// 从注册表中移除一个 Actor。
    pub fn remove(&self, id: &Ustr) -> Option<Rc<UnsafeCell<dyn Actor>>> {
        self.actors.borrow_mut().remove(id)
    }

    /// 检查是否存在具有该 `id` 的 Actor。
    pub fn contains(&self, id: &Ustr) -> bool {
        self.actors.borrow().contains_key(id)
    }
}

pub fn get_actor_registry() -> &'static ActorRegistry {
    ACTOR_REGISTRY.with(|registry| unsafe {
        // SAFETY: We return a static reference that lives for the lifetime of the thread.
        // Since this is thread_local storage, each thread has its own instance.
        // The transmute extends the lifetime to 'static which is safe because
        // thread_local ensures the registry lives for the thread's entire lifetime.
        std::mem::transmute::<&ActorRegistry, &'static ActorRegistry>(registry)
    })
}

/// 注册一个 Actor。
pub fn register_actor<T>(actor: T) -> Rc<UnsafeCell<T>>
where
    T: Actor + 'static,
{
    let actor_id = actor.id();
    let actor_ref = Rc::new(UnsafeCell::new(actor));

    // 注册为 Actor（仅限消息处理）
    let actor_trait_ref: Rc<UnsafeCell<dyn Actor>> = actor_ref.clone();
    get_actor_registry().insert(actor_id, actor_trait_ref);

    actor_ref
}

pub fn get_actor(id: &Ustr) -> Option<Rc<UnsafeCell<dyn Actor>>> {
    get_actor_registry().get(id)
}

/// 返回一个守卫，该守卫提供对类型为 `T` 的已注册 Actor 的可变访问。
///
/// 返回的 [`ActorRef`] 持有一个 `Rc` 以保持 Actor 存活，防止在从注册表中删除 Actor 时出现使用后释放。
///
/// # Panics
///
/// - 如果在注册表中未找到具有指定 `id` 的 Actor，则会 panic。
/// - 如果存储的 Actor 不是类型 `T`，则会 panic。
///
/// # 安全性 (Safety)
///
/// 虽然此函数未标记为 `unsafe`，但别名约束仍然适用：
///
/// - **别名 (Aliasing)**：调用者应确保不存在对同一 Actor 的其他并发可变引用。
///   本代码库中的基于回调的消息处理模式需要可重入访问，这在技术上违反了这一不变量。
/// - **线程安全**：注册表是线程本地的；请勿跨线程发送守卫。
#[must_use]
pub fn get_actor_unchecked<T: Actor>(id: &Ustr) -> ActorRef<T> {
    let registry = get_actor_registry();
    let actor_rc = registry
        .get(id)
        .unwrap_or_else(|| panic!("未找到 ID 为 {id} 的 Actor"));

    // SAFETY: Get a reference to check the type before casting
    let actor_ref = unsafe { &*actor_rc.get() };
    let actual_type = actor_ref.as_any().type_id();
    let expected_type = TypeId::of::<T>();

    assert!(
        actual_type == expected_type,
        "Actor 类型不匹配 '{id}': 期望 {expected_type:?}, 实际为 {actual_type:?}"
    );

    ActorRef {
        actor_rc,
        _marker: PhantomData,
    }
}

/// 尝试获取提供对已注册 Actor 的可变访问的守卫。
///
/// 如果未找到 Actor 或类型不匹配，则返回 `None`。
///
/// # 安全性 (Safety)
///
/// 有关安全性要求，请参见 [`get_actor_unchecked`]。同样的别名和线程安全约束也适用。
#[must_use]
pub fn try_get_actor_unchecked<T: Actor>(id: &Ustr) -> Option<ActorRef<T>> {
    let registry = get_actor_registry();
    let actor_rc = registry.get(id)?;

    // SAFETY: Get a reference to check the type before casting
    let actor_ref = unsafe { &*actor_rc.get() };
    let actual_type = actor_ref.as_any().type_id();
    let expected_type = TypeId::of::<T>();

    if actual_type != expected_type {
        return None;
    }

    Some(ActorRef {
        actor_rc,
        _marker: PhantomData,
    })
}

/// 检查注册表中是否存在具有该 `id` 的 Actor。
pub fn actor_exists(id: &Ustr) -> bool {
    get_actor_registry().contains(id)
}

/// 返回注册的 Actor 数量。
pub fn actor_count() -> usize {
    get_actor_registry().len()
}

#[cfg(test)]
/// 清除 Actor 注册表（用于测试隔离）。
pub fn clear_actor_registry() {
    let registry = get_actor_registry();
    registry.actors.borrow_mut().clear();
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    use rstest::rstest;

    use super::*;

    #[derive(Debug)]
    struct TestActor {
        id: Ustr,
        value: i32,
    }

    impl Actor for TestActor {
        fn id(&self) -> Ustr {
            self.id
        }
        fn handle(&mut self, _msg: &dyn Any) {}
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[rstest]
    fn test_register_and_get_actor() {
        clear_actor_registry();

        let id = Ustr::from("test-actor");
        let actor = TestActor { id, value: 42 };
        register_actor(actor);

        let actor_ref = get_actor_unchecked::<TestActor>(&id);
        assert_eq!(actor_ref.value, 42);
    }

    #[rstest]
    fn test_mutation_through_reference() {
        clear_actor_registry();

        let id = Ustr::from("test-actor-mut");
        let actor = TestActor { id, value: 0 };
        register_actor(actor);

        let mut actor_ref = get_actor_unchecked::<TestActor>(&id);
        actor_ref.value = 999;

        let actor_ref2 = get_actor_unchecked::<TestActor>(&id);
        assert_eq!(actor_ref2.value, 999);
    }

    #[rstest]
    fn test_try_get_returns_none_for_missing() {
        clear_actor_registry();

        let id = Ustr::from("nonexistent");
        let result = try_get_actor_unchecked::<TestActor>(&id);
        assert!(result.is_none());
    }

    #[rstest]
    fn test_try_get_returns_none_for_wrong_type() {
        #[derive(Debug)]
        struct OtherActor {
            id: Ustr,
        }

        impl Actor for OtherActor {
            fn id(&self) -> Ustr {
                self.id
            }
            fn handle(&mut self, _msg: &dyn Any) {}
            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        clear_actor_registry();

        let id = Ustr::from("other-actor");
        let actor = OtherActor { id };
        register_actor(actor);

        let result = try_get_actor_unchecked::<TestActor>(&id);
        assert!(result.is_none());
    }
}
