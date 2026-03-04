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

//! 用于管理有状态系统实体的组件系统。
//!
//! 此模块提供了用于管理系统实体生命周期和状态的组件框架。
//! 组件具有定义的状态（预初始化、就绪、运行中、停止等），
//! 并为状态管理和转换提供一致的接口。

#![allow(unsafe_code)]

use std::{
    cell::{RefCell, UnsafeCell},
    fmt::Debug,
    rc::Rc,
};

use ahash::{AHashMap, AHashSet};
use nautilus_model::identifiers::{ComponentId, TraderId};
use ustr::Ustr;

use crate::{
    actor::{Actor, registry::get_actor_registry},
    cache::Cache,
    clock::Clock,
    enums::{ComponentState, ComponentTrigger},
};

/// 组件具有状态和生命周期管理能力。
pub trait Component {
    /// 返回此组件的唯一标识符。
    fn component_id(&self) -> ComponentId;

    /// 返回组件的当前状态。
    fn state(&self) -> ComponentState;

    /// 使用状态触发器转换组件状态。
    ///
    /// # 错误
    ///
    /// 如果 `trigger` 是从当前状态出发的无效转换，则返回错误。
    fn transition_state(&mut self, trigger: ComponentTrigger) -> anyhow::Result<()>;

    /// 返回组件是否已就绪。
    fn is_ready(&self) -> bool {
        self.state() == ComponentState::Ready
    }

    /// 返回组件是否**不**在运行。
    fn not_running(&self) -> bool {
        !self.is_running()
    }

    /// 返回组件是否正在运行。
    fn is_running(&self) -> bool {
        self.state() == ComponentState::Running
    }

    /// 返回组件是否已停止。
    fn is_stopped(&self) -> bool {
        self.state() == ComponentState::Stopped
    }

    /// 返回组件是否已降级。
    fn is_degraded(&self) -> bool {
        self.state() == ComponentState::Degraded
    }

    /// 返回组件是否已发生故障。
    fn is_faulted(&self) -> bool {
        self.state() == ComponentState::Faulted
    }

    /// 返回组件是否已销毁。
    fn is_disposed(&self) -> bool {
        self.state() == ComponentState::Disposed
    }

    /// 向系统注册组件。
    ///
    /// # 错误
    ///
    /// 如果组件注册失败，则返回错误。
    fn register(
        &mut self,
        trader_id: TraderId,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<()>;

    /// 初始化组件。
    ///
    /// # 错误
    ///
    /// 如果初始化状态转换失败，则返回错误。
    fn initialize(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Initialize)
    }

    /// 启动组件。
    ///
    /// # 错误
    ///
    /// 如果组件启动失败，则返回错误。
    fn start(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Start)?; // -> 进入 Starting 状态

        if let Err(e) = self.on_start() {
            log_error(&e);
            return Err(e); // 停止状态转换
        }

        self.transition_state(ComponentTrigger::StartCompleted)?;

        Ok(())
    }

    /// 停止组件。
    ///
    /// # 错误
    ///
    /// 如果组件停止失败，则返回错误。
    fn stop(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Stop)?; // -> 进入 Stopping 状态

        if let Err(e) = self.on_stop() {
            log_error(&e);
            return Err(e); // 停止状态转换
        }

        self.transition_state(ComponentTrigger::StopCompleted)?;

        Ok(())
    }

    /// 恢复组件。
    ///
    /// # 错误
    ///
    /// 如果组件恢复失败，则返回错误。
    fn resume(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Resume)?; // -> 进入 Resuming 状态

        if let Err(e) = self.on_resume() {
            log_error(&e);
            return Err(e); // 停止状态转换
        }

        self.transition_state(ComponentTrigger::ResumeCompleted)?;

        Ok(())
    }

    /// 降级组件。
    ///
    /// # 错误
    ///
    /// 如果组件降级失败，则返回错误。
    fn degrade(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Degrade)?; // -> 进入 Degrading 状态

        if let Err(e) = self.on_degrade() {
            log_error(&e);
            return Err(e); // 停止状态转换
        }

        self.transition_state(ComponentTrigger::DegradeCompleted)?;

        Ok(())
    }

    /// 使组件进入故障状态。
    ///
    /// # 错误
    ///
    /// 如果组件故障处理失败，则返回错误。
    fn fault(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Fault)?; // -> 进入 Faulting 状态

        if let Err(e) = self.on_fault() {
            log_error(&e);
            return Err(e); // 停止状态转换状态转换
        }

        self.transition_state(ComponentTrigger::FaultCompleted)?;

        Ok(())
    }

    /// 将组件重置为其初始状态。
    ///
    /// # 错误
    ///
    /// 如果组件重置失败，则返回错误。
    fn reset(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Reset)?; // -> 进入 Resetting 状态

        if let Err(e) = self.on_reset() {
            log_error(&e);
            return Err(e); // 停止状态转换
        }

        self.transition_state(ComponentTrigger::ResetCompleted)?;

        Ok(())
    }

    /// 销毁组件，释放所有资源。
    ///
    /// # 错误
    ///
    /// 如果组件销毁失败，则返回错误。
    fn dispose(&mut self) -> anyhow::Result<()> {
        self.transition_state(ComponentTrigger::Dispose)?; // -> 进入 Disposing 状态

        if let Err(e) = self.on_dispose() {
            log_error(&e);
            return Err(e); // 停止状态转换
        }

        self.transition_state(ComponentTrigger::DisposeCompleted)?;

        Ok(())
    }

    /// 在启动时执行的操作。
    ///
    /// # 错误
    ///
    /// 如果启动 Actor 失败，则返回错误。
    fn on_start(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "`on_start` 处理器被调用但未被重载，\
            预期在此处理停止组件时所需的任何操作，\
            例如取消数据订阅",
        );
        Ok(())
    }

    /// 在停止时执行的操作。
    ///
    /// # 错误
    ///
    /// 如果停止 Actor 失败，则返回错误。
    fn on_stop(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "`on_stop` 处理器被调用但未被重载，\
            预期在此处理停止组件时所需的任何操作，\
            例如取消数据订阅",
        );
        Ok(())
    }

    /// 在恢复时执行的操作。
    ///
    /// # 错误
    ///
    /// 如果恢复 Actor 失败，则返回错误。
    fn on_resume(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "`on_resume` 处理器被调用但未被重载，\
            预期在此处理停止后的组件恢复时所需的任何操作"
        );
        Ok(())
    }

    /// 在重置时执行的操作。
    ///
    /// # 错误
    ///
    /// 如果重置 Actor 失败，则返回错误。
    fn on_reset(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "`on_reset` 处理器被调用但未被重载，\
            预期在此处理重置组件时所需的任何操作，\
            例如重置指标和其他状态"
        );
        Ok(())
    }

    /// 在销毁时执行的操作。
    ///
    /// # 错误
    ///
    /// 如果销毁 Actor 失败，则返回错误。
    fn on_dispose(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// 在降级时执行的操作。
    ///
    /// # 错误
    ///
    /// 如果降级 Actor 失败，则返回错误。
    fn on_degrade(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// 在故障时执行的操作。
    ///
    /// # 错误
    ///
    /// 如果使 Actor 进入故障状态失败，则返回错误。
    fn on_fault(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

fn log_error(e: &anyhow::Error) {
    log::error!("{e}");
}

#[rustfmt::skip]
impl ComponentState {
    /// 使用组件 `trigger` 转换状态机。
    ///
    /// # 错误
    ///
    /// 如果 `trigger` 对于当前状态无效，则返回错误。
    pub fn transition(&mut self, trigger: &ComponentTrigger) -> anyhow::Result<Self> {
        let new_state = match (&self, trigger) {
            (Self::PreInitialized, ComponentTrigger::Initialize) => Self::Ready,
            (Self::Ready, ComponentTrigger::Reset) => Self::Resetting,
            (Self::Ready, ComponentTrigger::Start) => Self::Starting,
            (Self::Ready, ComponentTrigger::Dispose) => Self::Disposing,
            (Self::Resetting, ComponentTrigger::ResetCompleted) => Self::Ready,
            (Self::Starting, ComponentTrigger::StartCompleted) => Self::Running,
            (Self::Starting, ComponentTrigger::Stop) => Self::Stopping,
            (Self::Starting, ComponentTrigger::Fault) => Self::Faulting,
            (Self::Running, ComponentTrigger::Stop) => Self::Stopping,
            (Self::Running, ComponentTrigger::Degrade) => Self::Degrading,
            (Self::Running, ComponentTrigger::Fault) => Self::Faulting,
            (Self::Resuming, ComponentTrigger::Stop) => Self::Stopping,
            (Self::Resuming, ComponentTrigger::ResumeCompleted) => Self::Running,
            (Self::Resuming, ComponentTrigger::Fault) => Self::Faulting,
            (Self::Stopping, ComponentTrigger::StopCompleted) => Self::Stopped,
            (Self::Stopping, ComponentTrigger::Fault) => Self::Faulting,
            (Self::Stopped, ComponentTrigger::Reset) => Self::Resetting,
            (Self::Stopped, ComponentTrigger::Resume) => Self::Resuming,
            (Self::Stopped, ComponentTrigger::Dispose) => Self::Disposing,
            (Self::Stopped, ComponentTrigger::Fault) => Self::Faulting,
            (Self::Degrading, ComponentTrigger::DegradeCompleted) => Self::Degraded,
            (Self::Degraded, ComponentTrigger::Resume) => Self::Resuming,
            (Self::Degraded, ComponentTrigger::Stop) => Self::Stopping,
            (Self::Degraded, ComponentTrigger::Fault) => Self::Faulting,
            (Self::Disposing, ComponentTrigger::DisposeCompleted) => Self::Disposed,
            (Self::Faulting, ComponentTrigger::FaultCompleted) => Self::Faulted,
            _ => anyhow::bail!("Invalid state trigger {self} -> {trigger}"),
        };
        Ok(new_state)
    }
}

thread_local! {
    static COMPONENT_REGISTRY: ComponentRegistry = ComponentRegistry::new();
}

/// 用于存储具有运行时借用跟踪 (borrow tracking) 信息的组件注册表。
///
/// 该注册表跟踪目前哪些组件被可变借用，以防止多个同时的可变借用（这会导致未定义行为）。
pub struct ComponentRegistry {
    components: RefCell<AHashMap<Ustr, Rc<UnsafeCell<dyn Component>>>>,
    borrows: RefCell<AHashSet<Ustr>>,
}

impl Debug for ComponentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let components_ref = self.components.borrow();
        let keys: Vec<&Ustr> = components_ref.keys().collect();
        f.debug_struct(stringify!(ComponentRegistry))
            .field("components", &keys)
            .field("active_borrows", &self.borrows.borrow().len())
            .finish()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            components: RefCell::new(AHashMap::new()),
            borrows: RefCell::new(AHashSet::new()),
        }
    }

    pub fn insert(&self, id: Ustr, component: Rc<UnsafeCell<dyn Component>>) {
        self.components.borrow_mut().insert(id, component);
    }

    pub fn get(&self, id: &Ustr) -> Option<Rc<UnsafeCell<dyn Component>>> {
        self.components.borrow().get(id).cloned()
    }

    /// 检查组件当前是否被借用。
    pub fn is_borrowed(&self, id: &Ustr) -> bool {
        self.borrows.borrow().contains(id)
    }

    /// 将组件标记为已借用。如果已被借用，则返回 false。
    fn try_borrow(&self, id: Ustr) -> bool {
        let mut borrows = self.borrows.borrow_mut();
        if borrows.contains(&id) {
            false
        } else {
            borrows.insert(id);
            true
        }
    }

    /// 释放组件的借用。
    fn release_borrow(&self, id: &Ustr) {
        self.borrows.borrow_mut().remove(id);
    }
}

/// 在 Drop 时释放组件借用的守卫 (Guard)。
///
/// 这确保即使在生命周期方法调用期间代码发生 panic，借用也能被释放。
struct BorrowGuard {
    id: Ustr,
}

impl BorrowGuard {
    fn new(id: Ustr) -> Self {
        Self { id }
    }
}

impl Drop for BorrowGuard {
    fn drop(&mut self) {
        get_component_registry().release_borrow(&self.id);
    }
}

/// 返回全局组件注册表的引用。
pub fn get_component_registry() -> &'static ComponentRegistry {
    COMPONENT_REGISTRY.with(|registry| unsafe {
        // 安全性 (SAFETY): 我们返回一个在线程生命周期内持续存在的静态引用。
        // 由于这是 thread_local 存储，每个线程都有自己的实例。
        std::mem::transmute::<&ComponentRegistry, &'static ComponentRegistry>(registry)
    })
}

/// 注册一个组件。
pub fn register_component<T>(component: T) -> Rc<UnsafeCell<T>>
where
    T: Component + 'static,
{
    let component_id = component.component_id().inner();
    let component_ref = Rc::new(UnsafeCell::new(component));

    // 在组件注册表中注册
    let component_trait_ref: Rc<UnsafeCell<dyn Component>> = component_ref.clone();
    get_component_registry().insert(component_id, component_trait_ref);

    component_ref
}

/// 注册一个同时实现了 Actor 的组件。
pub fn register_component_actor<T>(component: T) -> Rc<UnsafeCell<T>>
where
    T: Component + Actor + 'static,
{
    let component_id = component.component_id().inner();
    let actor_id = component.id();
    let component_ref = Rc::new(UnsafeCell::new(component));

    // 在组件注册表中注册
    let component_trait_ref: Rc<UnsafeCell<dyn Component>> = component_ref.clone();
    get_component_registry().insert(component_id, component_trait_ref);

    // 在 Actor 注册表中注册
    let actor_trait_ref: Rc<UnsafeCell<dyn Actor>> = component_ref.clone();
    get_actor_registry().insert(actor_id, actor_trait_ref);

    component_ref
}

/// 在全局注册表中安全地调用组件的 start()。
///
/// # 错误
///
/// - 如果未找到组件，则返回错误。
/// - 如果组件已被借用，则返回错误。
/// - 如果 start() 失败，则返回错误。
pub fn start_component(id: &Ustr) -> anyhow::Result<()> {
    let registry = get_component_registry();
    let component_ref = registry
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("在全局注册表中未找到组件 '{id}'"))?;

    if !registry.try_borrow(*id) {
        anyhow::bail!(
            "组件 '{id}' 已被可变借用。 \
             这将创建别名可变引用（未定义行为）。"
        );
    }

    let _guard = BorrowGuard::new(*id);

    // 安全性 (SAFETY): 借用跟踪确保独占访问
    unsafe {
        let component = &mut *component_ref.get();
        component.start()
    }
}

/// 在全局注册表中安全地调用组件的 stop()。
///
/// # 错误
///
/// - 如果未找到组件，则返回错误。
/// - 如果组件已被借用，则返回错误。
/// - 如果 stop() 失败，则返回错误。
pub fn stop_component(id: &Ustr) -> anyhow::Result<()> {
    let registry = get_component_registry();
    let component_ref = registry
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("在全局注册表中未找到组件 '{id}'"))?;

    if !registry.try_borrow(*id) {
        anyhow::bail!(
            "组件 '{id}' 已被可变借用。 \
             这将创建别名可变引用（未定义行为）。"
        );
    }

    let _guard = BorrowGuard::new(*id);

    // 安全性 (SAFETY): 借用跟踪确保独占访问
    unsafe {
        let component = &mut *component_ref.get();
        component.stop()
    }
}

/// 在全局注册表中安全地调用组件的 reset()。
///
/// # 错误
///
/// - 如果未找到组件，则返回错误。
/// - 如果组件已被借用，则返回错误。
/// - 如果 reset() 失败，则返回错误。
pub fn reset_component(id: &Ustr) -> anyhow::Result<()> {
    let registry = get_component_registry();
    let component_ref = registry
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("在全局注册表中未找到组件 '{id}'"))?;

    if !registry.try_borrow(*id) {
        anyhow::bail!(
            "组件 '{id}' 已被可变借用。 \
             这将创建别名可变引用（未定义行为）。"
        );
    }

    let _guard = BorrowGuard::new(*id);

    // 安全性 (SAFETY): 借用跟踪确保独占访问
    unsafe {
        let component = &mut *component_ref.get();
        component.reset()
    }
}

/// 在全局注册表中安全地调用组件的 dispose()。
///
/// # 错误
///
/// - 如果未找到组件，则返回错误。
/// - 如果组件已被借用，则返回错误。
/// - 如果 dispose() 失败，则返回错误。
pub fn dispose_component(id: &Ustr) -> anyhow::Result<()> {
    let registry = get_component_registry();
    let component_ref = registry
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("在全局注册表中未找到组件 '{id}'"))?;

    if !registry.try_borrow(*id) {
        anyhow::bail!(
            "组件 '{id}' 已被可变借用。 \
             这将创建别名可变引用（未定义行为）。"
        );
    }

    let _guard = BorrowGuard::new(*id);

    // 安全性 (SAFETY): 借用跟踪确保独占访问
    unsafe {
        let component = &mut *component_ref.get();
        component.dispose()
    }
}

/// 根据 ID 从全局注册表中返回一个组件。
pub fn get_component(id: &Ustr) -> Option<Rc<UnsafeCell<dyn Component>>> {
    get_component_registry().get(id)
}

#[cfg(test)]
/// 清理组件注册表（用于测试隔离）。
pub fn clear_component_registry() {
    let registry = get_component_registry();
    registry.components.borrow_mut().clear();
    registry.borrows.borrow_mut().clear();
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use rstest::rstest;

    use super::*;

    struct TestComponent {
        id: ComponentId,
        state: ComponentState,
        should_panic: &'static AtomicBool,
    }

    impl TestComponent {
        fn new(name: &str, should_panic: &'static AtomicBool) -> Self {
            Self {
                id: ComponentId::new(name),
                state: ComponentState::Ready,
                should_panic,
            }
        }
    }

    impl Component for TestComponent {
        fn component_id(&self) -> ComponentId {
            self.id
        }

        fn state(&self) -> ComponentState {
            self.state
        }

        fn transition_state(&mut self, trigger: ComponentTrigger) -> anyhow::Result<()> {
            self.state = self.state.transition(&trigger)?;
            Ok(())
        }

        fn register(
            &mut self,
            _trader_id: TraderId,
            _clock: Rc<RefCell<dyn Clock>>,
            _cache: Rc<RefCell<Cache>>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        #[allow(clippy::panic_in_result_fn)] // 为了测试有意而为的 panic
        fn on_start(&mut self) -> anyhow::Result<()> {
            assert!(
                !self.should_panic.load(Ordering::SeqCst),
                "为了测试有意而为的 panic"
            );
            Ok(())
        }
    }

    static NO_PANIC: AtomicBool = AtomicBool::new(false);
    static DO_PANIC: AtomicBool = AtomicBool::new(true);

    #[rstest]
    fn test_component_borrow_tracking_prevents_double_borrow() {
        clear_component_registry();

        let id = Ustr::from("test-component-1");
        let component = TestComponent::new("test-component-1", &NO_PANIC);
        let component_id = component.id.inner();

        let component_ref = Rc::new(UnsafeCell::new(component));
        get_component_registry().insert(component_id, component_ref);

        // 第一次通过 start_component 进行的借用应该成功
        let result1 = start_component(&id);
        assert!(result1.is_ok());

        // 组件现在应该可以再次借用（守卫 guard 已释放）
        let result2 = stop_component(&id);
        assert!(result2.is_ok());
    }

    #[rstest]
    fn test_component_borrow_released_after_lifecycle_call() {
        clear_component_registry();

        let id = Ustr::from("test-component-2");
        let component = TestComponent::new("test-component-2", &NO_PANIC);
        let component_id = component.id.inner();

        let component_ref = Rc::new(UnsafeCell::new(component));
        get_component_registry().insert(component_id, component_ref);

        // 调用 start - 之后借用应释放
        let _ = start_component(&id);

        // 验证未被标记为已借用
        assert!(!get_component_registry().is_borrowed(&id));
    }

    #[rstest]
    fn test_component_borrow_released_on_panic() {
        clear_component_registry();

        let id = Ustr::from("test-component-panic");
        let component = TestComponent::new("test-component-panic", &DO_PANIC);
        let component_id = component.id.inner();

        let component_ref = Rc::new(UnsafeCell::new(component));
        get_component_registry().insert(component_id, component_ref);

        // 调用 start，这将引起 panic - 捕获该 panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = start_component(&id);
        }));
        assert!(result.is_err(), "预期 on_start 会发生 panic");

        // 由于 BorrowGuard 的 Drop，借用仍应被释放
        assert!(
            !get_component_registry().is_borrowed(&id),
            "发生 panic 后借用未被释放"
        );
    }
}
