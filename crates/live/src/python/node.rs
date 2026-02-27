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

//! 实盘节点的 Python 绑定。

use std::{cell::RefCell, rc::Rc};

use nautilus_common::{
    actor::data_actor::{DataActorConfig, ImportableActorConfig},
    enums::Environment,
    live::get_runtime,
    logging::logger::LoggerConfig,
    python::actor::PyDataActor,
};
use nautilus_core::{
    UUID4,
    python::{to_pyruntime_err, to_pyvalue_err},
};
use nautilus_model::identifiers::{ActorId, TraderId};
use nautilus_system::get_global_pyo3_registry;
use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};
use serde_json;

use crate::{builder::LiveNodeBuilder, node::LiveNode};

#[pymethods]
impl LiveNode {
    #[staticmethod]
    #[pyo3(name = "builder")]
    fn py_builder(
        name: String,
        trader_id: TraderId,
        environment: Environment,
    ) -> PyResult<LiveNodeBuilderPy> {
        match Self::builder(trader_id, environment) {
            Ok(builder) => Ok(LiveNodeBuilderPy {
                inner: Rc::new(RefCell::new(Some(builder.with_name(name)))),
            }),
            Err(e) => Err(to_pyruntime_err(e)),
        }
    }

    #[getter]
    #[pyo3(name = "environment")]
    fn py_environment(&self) -> Environment {
        self.environment()
    }

    #[getter]
    #[pyo3(name = "trader_id")]
    fn py_trader_id(&self) -> TraderId {
        self.trader_id()
    }

    #[getter]
    #[pyo3(name = "instance_id")]
    const fn py_instance_id(&self) -> UUID4 {
        self.instance_id()
    }

    #[getter]
    #[pyo3(name = "is_running")]
    fn py_is_running(&self) -> bool {
        self.is_running()
    }

    #[pyo3(name = "start")]
    fn py_start(&mut self) -> PyResult<()> {
        if self.is_running() {
            return Err(to_pyruntime_err("LiveNode 已经在运行中"));
        }

        // 非阻塞启动 - 仅在后台启动节点
        get_runtime().block_on(async { self.start().await.map_err(to_pyruntime_err) })
    }

    #[pyo3(name = "run")]
    fn py_run(&mut self, py: Python) -> PyResult<()> {
        if self.is_running() {
            return Err(to_pyruntime_err("LiveNode 已经在运行中"));
        }

        // 获取用于与信号检查器协调的句柄
        let handle = self.handle();

        // 导入信号模块
        let signal_module = py.import("signal")?;
        let original_handler =
            signal_module.call_method1("signal", (2, signal_module.getattr("SIG_DFL")?))?; // 保存原始 SIGINT 处理程序 (signal 2)

        // 设置使用我们句柄的自定义信号处理程序
        let handle_for_signal = handle;
        let signal_callback = pyo3::types::PyCFunction::new_closure(
            py,
            None,
            None,
            move |_args: &pyo3::Bound<'_, PyTuple>,
                  _kwargs: Option<&pyo3::Bound<'_, PyDict>>|
                  -> PyResult<()> {
                log::info!("Python 信号处理程序被调用");
                handle_for_signal.stop();
                Ok(())
            },
        )?;

        // 安装我们的信号处理程序
        signal_module.call_method1("signal", (2, signal_callback))?;

        // 运行节点并在之后恢复信号处理程序
        let result =
            { get_runtime().block_on(async { self.run().await.map_err(to_pyruntime_err) }) };

        // 恢复原始信号处理程序
        signal_module.call_method1("signal", (2, original_handler))?;

        result
    }

    #[pyo3(name = "stop")]
    fn py_stop(&self) -> PyResult<()> {
        if !self.is_running() {
            return Err(to_pyruntime_err("LiveNode 未在运行中"));
        }

        // 使用句柄发出停止信号 - 它是线程安全的，不需要 async
        self.handle().stop();
        Ok(())
    }

    #[allow(unsafe_code, reason = "Python 参与者组件注册所需")]
    #[pyo3(name = "add_actor_from_config")]
    fn py_add_actor_from_config(
        &mut self,
        _py: Python,
        config: ImportableActorConfig,
    ) -> PyResult<()> {
        log::debug!("`add_actor_from_config` 参数为: {config:?}");

        // 从 actor_path 中提取模块和类名
        let parts: Vec<&str> = config.actor_path.split(':').collect();
        if parts.len() != 2 {
            return Err(to_pyvalue_err(
                "actor_path 必须符合 'module.path:ClassName' 格式",
            ));
        }
        let (module_name, class_name) = (parts[0], parts[1]);

        log::info!("正在从模块 {} 类 {} 导入参与者", module_name, class_name);

        // 导入 Python 类以验证其是否存在，并获取它以进行方法调度
        let _python_class = Python::attach(|py| -> PyResult<Py<PyAny>> {
            let actor_module = py.import(module_name)?;
            let actor_class = actor_module.getattr(class_name)?;
            Ok(actor_class.unbind())
        })
        .map_err(|e| to_pyruntime_err(format!("导入 Python 类失败: {e}")))?;

        // 为 Rust PyDataActor 创建默认的 DataActorConfig。
        // 在创建 Python 参与者后提取并连接继承的配置属性
        let basic_data_actor_config = DataActorConfig::default();

        log::debug!("为 Rust 创建了基础 DataActorConfig: {basic_data_actor_config:?}");

        // 创建 Python 参与者并注册内部 PyDataActor
        let python_actor = Python::attach(|py| -> anyhow::Result<Py<PyAny>> {
            // 导入 Python 类
            let actor_module = py
                .import(module_name)
                .map_err(|e| anyhow::anyhow!("导入模块 {} 失败: {e}", module_name))?;
            let actor_class = actor_module
                .getattr(class_name)
                .map_err(|e| anyhow::anyhow!("获取类 {} 失败: {e}", class_name))?;

            // 如果提供了 config_path 和 config，则创建配置实例
            let config_instance = if !config.config_path.is_empty() && !config.config.is_empty() {
                // 解析 config_path 以获取模块和类
                let config_parts: Vec<&str> = config.config_path.split(':').collect();
                if config_parts.len() != 2 {
                    anyhow::bail!(
                        "config_path 必须符合 'module.path:ClassName' 格式，实际为 {}",
                        config.config_path
                    );
                }
                let (config_module_name, config_class_name) = (config_parts[0], config_parts[1]);

                log::debug!("正在从模块 {} 类 {} 导入配置类", config_module_name, config_class_name);

                // 导入配置类
                let config_module = py
                    .import(config_module_name)
                    .map_err(|e| anyhow::anyhow!("导入配置模块 {} 失败: {e}", config_module_name))?;
                let config_class = config_module
                    .getattr(config_class_name)
                    .map_err(|e| anyhow::anyhow!("获取配置类 {} 失败: {e}", config_class_name))?;

                // 将 serde_json::Value 配置字典转换回 Python 字典
                let py_dict = PyDict::new(py);
                for (key, value) in &config.config {
                    // 通过 JSON 将 serde_json::Value 转换回 Python 对象
                    let json_str = serde_json::to_string(value)
                        .map_err(|e| anyhow::anyhow!("序列化配置值失败: {e}"))?;
                    let py_value = PyModule::import(py, "json")?
                        .call_method("loads", (json_str,), None)?;
                    py_dict.set_item(key, py_value)?;
                }

                log::debug!("创建了配置字典: {py_dict:?}");

                // 尝试多种方法来创建配置实例
                let config_instance = {
                    // 首先尝试使用 **kwargs 调用配置类
                    match config_class.call((), Some(&py_dict)) {
                        Ok(instance) => {
                            log::debug!("成功使用 kwargs 创建了配置实例");

                            // 如果 __post_init__ 存在，则手动调用它
                            if let Err(e) = instance.call_method0("__post_init__") {
                                log::error!("在配置实例上调用 __post_init__ 失败: {e}");
                                anyhow::bail!("__post_init__ 失败: {e}");
                            }
                            log::debug!("在配置实例上成功调用了 __post_init__");

                            instance
                        },
                        Err(kwargs_err) => {
                            log::debug!("使用 kwargs 创建配置失败: {kwargs_err}");

                            // 第二种方法：尝试使用默认构造函数创建并设置属性
                            match config_class.call0() {
                                Ok(instance) => {
                                    log::debug!("创建了默认配置实例，正在设置属性");
                                    for (key, value) in &config.config {
                                        // 将 serde_json::Value 转换为 Python 对象
                                        let json_str = serde_json::to_string(value)
                                            .map_err(|e| anyhow::anyhow!("序列化配置值失败: {e}"))?;
                                        let py_value = PyModule::import(py, "json")?
                                            .call_method("loads", (json_str,), None)?;
                                        if let Err(setattr_err) = instance.setattr(key, py_value) {
                                            log::warn!("设置属性 {} 失败: {setattr_err}", key);
                                        }
                                    }

                                    // 如果 __post_init__ 存在，则手动调用它
                                    if let Err(e) = instance.call_method0("__post_init__") {
                                        log::error!("在配置实例上调用 __post_init__ 失败: {e}");
                                        anyhow::bail!("__post_init__ 失败: {e}");
                                    }
                                    log::debug!("在配置实例上调用了 __post_init__");

                                    instance
                                },
                                Err(default_err) => {
                                    log::debug!("创建默认配置失败: {default_err}");

                                    // 如果两种方法都失败，则返回原始错误
                                    anyhow::bail!(
                                        "创建配置实例失败。尝试 kwargs 方法: {kwargs_err}，默认构造函数: {default_err}"
                                    );
                                }
                            }
                        }
                    }
                };

                log::debug!("创建了配置实例: {config_instance:?}");

                Some(config_instance)
            } else {
                log::debug!("没有 config_path 或配置为空，使用 None");
                None
            };

            // 使用配置创建 Python 参与者实例
            let python_actor = if let Some(config_obj) = config_instance.clone() {
                actor_class.call1((config_obj,))?
            } else {
                actor_class.call0()?
            };

            log::debug!("创建了 Python 参与者实例: {python_actor:?}");

            // 获取内部 PyDataActor 的可变引用以进行注册
            let mut py_data_actor_ref = python_actor
                .extract::<PyRefMut<PyDataActor>>()
                .map_err(Into::<PyErr>::into)
                .map_err(|e| anyhow::anyhow!("提取 PyDataActor 失败: {e}"))?;

            log::debug!(
                "内部 PyDataActor 内存地址: {}，已注册: {}",
                &py_data_actor_ref.mem_address(),
                py_data_actor_ref.is_registered()
            );

            // 从 Python 参与者实例中提取继承的 DataActorConfig 字段，
            // 并将其连接到 PyDataActor 的核心配置中
            if let Some(config_obj) = config_instance.as_ref() {
                log::debug!("正在从 Python 参与者配置中提取继承的配置字段");

                // 如果存在 actor_id，则提取它
                if let Ok(actor_id) = config_obj.getattr("actor_id")
                    && !actor_id.is_none() {
                        // 首先尝试提取为 ActorId，然后尝试作为字符串
                        let actor_id_val = if let Ok(actor_id_val) = actor_id.extract::<ActorId>() {
                            actor_id_val
                        } else if let Ok(actor_id_str) = actor_id.extract::<String>() {
                            ActorId::from(actor_id_str.as_str())
                        } else {
                            log::warn!("未能将 actor_id 提取为 ActorId 或 String");
                            anyhow::bail!("无效的 `actor_id` 类型");
                        };

                        log::debug!("提取了 actor_id: {actor_id_val}");
                        py_data_actor_ref.set_actor_id(actor_id_val);
                    }

                // 如果存在 log_events，则提取它
                if let Ok(log_events) = config_obj.getattr("log_events")
                    && let Ok(log_events_val) = log_events.extract::<bool>() {
                        log::debug!("提取了 log_events: {log_events_val}");
                        py_data_actor_ref.set_log_events(log_events_val);
                    }

                // 如果存在 log_commands，则提取它
                if let Ok(log_commands) = config_obj.getattr("log_commands")
                    && let Ok(log_commands_val) = log_commands.extract::<bool>() {
                        log::debug!("提取了 log_commands: {log_commands_val}");
                        py_data_actor_ref.set_log_commands(log_commands_val);
                    }

                log::debug!("成功从 Python 参与者实例更新了 PyDataActor 配置");
            }

            // 为原始实例上的方法调度设置 Python 实例引用
            py_data_actor_ref.set_python_instance(python_actor.clone().unbind());

            log::debug!("设置了用于方法调度的 Python 实例引用");

            // 注册内部 PyDataActor
            let trader_id = self.trader_id();
            let clock = self.kernel().clock();
            let cache = self.kernel().cache();

            py_data_actor_ref
                .register(trader_id, clock, cache)
                .map_err(|e| anyhow::anyhow!("注册 PyDataActor 失败: {e}"))?;

            log::debug!(
                "内部 PyDataActor 已注册: {}，状态: {:?}",
                py_data_actor_ref.is_registered(),
                py_data_actor_ref.state()
            );

            Ok(python_actor.unbind())
        })
        .map_err(to_pyruntime_err)?;

        let actor_id = Python::attach(|py| -> anyhow::Result<ActorId> {
            let py_actor = python_actor.bind(py);
            let py_data_actor_ref = py_actor
                .cast::<PyDataActor>()
                .map_err(|e| anyhow::anyhow!("向下转型为 PyDataActor 失败: {e}"))?;
            let py_data_actor = py_data_actor_ref.borrow();
            py_data_actor.register_in_global_registries();

            Ok(py_data_actor.actor_id())
        })
        .map_err(to_pyruntime_err)?;

        self.kernel_mut()
            .trader
            .add_actor_id_for_lifecycle(actor_id)
            .map_err(to_pyruntime_err)?;

        // 注意：不需要 mem::forget - 参与者的 py_self 字段持有一个 Py<PyAny>，
        // 它可以保持 Python 实例处于活动状态，并且注册表通过 Rc::clone() 共享内部对象

        log::info!("已注册 Python 参与者 {}", actor_id);
        Ok(())
    }

    fn __repr__(&self) -> String {
        format!(
            "LiveNode(trader_id={}, environment={:?}, running={})",
            self.trader_id(),
            self.environment(),
            self.is_running()
        )
    }
}

/// `LiveNodeBuilder` 的 Python 包装器，使用内部可变性
/// 以解决 PyO3 的共享所有权模型。
#[derive(Debug)]
#[pyclass(name = "LiveNodeBuilder", module = "nautilus_trader.live", unsendable)]
pub struct LiveNodeBuilderPy {
    inner: Rc<RefCell<Option<LiveNodeBuilder>>>,
}

#[pymethods]
impl LiveNodeBuilderPy {
    #[pyo3(name = "with_instance_id")]
    fn py_with_instance_id(&self, instance_id: UUID4) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_instance_id(instance_id));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_load_state")]
    fn py_with_load_state(&self, load_state: bool) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_load_state(load_state));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_save_state")]
    fn py_with_save_state(&self, save_state: bool) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_save_state(save_state));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_timeout_connection")]
    fn py_with_timeout_connection(&self, timeout_secs: u64) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_timeout_connection(timeout_secs));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_timeout_reconciliation")]
    fn py_with_timeout_reconciliation(&self, timeout_secs: u64) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_timeout_reconciliation(timeout_secs));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_timeout_portfolio")]
    fn py_with_timeout_portfolio(&self, timeout_secs: u64) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_timeout_portfolio(timeout_secs));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_timeout_disconnection_secs")]
    fn py_with_timeout_disconnection_secs(&self, timeout_secs: u64) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_timeout_disconnection_secs(timeout_secs));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_delay_post_stop_secs")]
    fn py_with_delay_post_stop_secs(&self, delay_secs: u64) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_delay_post_stop_secs(delay_secs));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_delay_shutdown_secs")]
    fn py_with_delay_shutdown_secs(&self, delay_secs: u64) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_delay_shutdown_secs(delay_secs));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_reconciliation")]
    fn py_with_reconciliation(&self, reconciliation: bool) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_reconciliation(reconciliation));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_reconciliation_lookback_mins")]
    fn py_with_reconciliation_lookback_mins(&self, mins: u32) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_reconciliation_lookback_mins(mins));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "with_logging")]
    fn py_with_logging(&self, logging: LoggerConfig) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            *inner_ref = Some(builder.with_logging(logging));
            Ok(Self {
                inner: self.inner.clone(),
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "add_data_client")]
    fn py_add_data_client(
        &self,
        name: Option<String>,
        factory: Py<PyAny>,
        config: Py<PyAny>,
    ) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            Python::attach(|py| -> PyResult<Self> {
                // 使用全局注册表将 Py<PyAny> 提取到 trait 对象
                let registry = get_global_pyo3_registry();

                let boxed_factory = registry.extract_factory(py, factory.clone_ref(py))?;
                let boxed_config = registry.extract_config(py, config.clone_ref(py))?;

                // 使用原始工厂的工厂名称作为客户端名称
                let factory_name = factory
                    .getattr(py, "name")?
                    .call0(py)?
                    .extract::<String>(py)?;
                let client_name = name.unwrap_or(factory_name);

                // 使用 boxed trait 对象将数据客户端添加到 builder 中
                match builder.add_data_client(Some(client_name), boxed_factory, boxed_config) {
                    Ok(updated_builder) => {
                        *inner_ref = Some(updated_builder);
                        Ok(Self {
                            inner: self.inner.clone(),
                        })
                    }
                    Err(e) => Err(to_pyruntime_err(format!("添加数据客户端失败: {e}"))),
                }
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "add_exec_client")]
    fn py_add_exec_client(
        &self,
        name: Option<String>,
        factory: Py<PyAny>,
        config: Py<PyAny>,
    ) -> PyResult<Self> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            Python::attach(|py| -> PyResult<Self> {
                let registry = get_global_pyo3_registry();

                let boxed_factory = registry.extract_exec_factory(py, factory.clone_ref(py))?;
                let boxed_config = registry.extract_config(py, config.clone_ref(py))?;

                let factory_name = factory
                    .getattr(py, "name")?
                    .call0(py)?
                    .extract::<String>(py)?;
                let client_name = name.unwrap_or(factory_name);

                match builder.add_exec_client(Some(client_name), boxed_factory, boxed_config) {
                    Ok(updated_builder) => {
                        *inner_ref = Some(updated_builder);
                        Ok(Self {
                            inner: self.inner.clone(),
                        })
                    }
                    Err(e) => Err(to_pyruntime_err(format!("添加执行客户端失败: {e}"))),
                }
            })
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    #[pyo3(name = "build")]
    fn py_build(&self) -> PyResult<LiveNode> {
        let mut inner_ref = self.inner.borrow_mut();
        if let Some(builder) = inner_ref.take() {
            match builder.build() {
                Ok(node) => Ok(node),
                Err(e) => Err(to_pyruntime_err(e)),
            }
        } else {
            Err(to_pyruntime_err("Builder 已被消耗"))
        }
    }

    fn __repr__(&self) -> String {
        format!("{self:?}")
    }
}
