// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// -------------------------------------------------------------------------------------------------

use std::{cell::UnsafeCell, rc::Rc};

use nautilus_common::actor::{Actor, DataActor, data_actor::DataActorCore};
use pyo3::prelude::*;

use crate::algorithm::{
    ExecutionAlgorithm, ExecutionAlgorithmConfig, ExecutionAlgorithmCore, TwapAlgorithm,
    TwapAlgorithmConfig,
};

/// Inner state of PyTwapAlgorithm, shared between Python wrapper and Rust registries.
pub struct PyTwapAlgorithmInner {
    core: TwapAlgorithm,
    py_self: Option<Py<PyAny>>,
}

/// Python-facing wrapper for TwapAlgorithm.
#[allow(non_camel_case_types)]
#[pyo3::pyclass(module = "nautilus_trader.trading", name = "TwapAlgorithm", unsendable)]
pub struct PyTwapAlgorithm {
    inner: Rc<UnsafeCell<PyTwapAlgorithmInner>>,
}

impl PyTwapAlgorithm {
    #[inline]
    #[allow(unsafe_code)]
    pub(crate) fn inner(&self) -> &PyTwapAlgorithmInner {
        unsafe { &*self.inner.get() }
    }

    #[inline]
    #[allow(unsafe_code, clippy::mut_from_ref)]
    pub(crate) fn inner_mut(&self) -> &mut PyTwapAlgorithmInner {
        unsafe { &mut *self.inner.get() }
    }
}

impl PyTwapAlgorithm {
    /// Creates a new PyTwapAlgorithm instance.
    pub fn new(config: TwapAlgorithmConfig) -> Self {
        let core = TwapAlgorithm::new(config);

        let inner = PyTwapAlgorithmInner {
            core,
            py_self: None,
        };

        Self {
            inner: Rc::new(UnsafeCell::new(inner)),
        }
    }

    /// Sets the Python instance reference for method dispatch.
    pub fn set_python_instance(&mut self, py_obj: Py<PyAny>) {
        self.inner_mut().py_self = Some(py_obj);
    }
}

#[pyo3::pymethods]
impl PyTwapAlgorithm {
    #[new]
    #[pyo3(signature = (config))]
    fn py_new(config: TwapAlgorithmConfig) -> Self {
        Self::new(config)
    }

    /// Captures the Python self reference for event dispatch.
    #[pyo3(signature = (config))]
    #[allow(unused_variables)]
    fn __init__(slf: &Bound<'_, Self>, config: TwapAlgorithmConfig) {
        let py_self: Py<PyAny> = slf.clone().unbind().into_any();
        slf.borrow_mut().set_python_instance(py_self);
    }
}

#[pyo3::pymethods]
impl ExecutionAlgorithmConfig {
    #[new]
    #[pyo3(signature = (
        exec_algorithm_id=None,
        log_events=true,
        log_commands=true
    ))]
    #[allow(clippy::too_many_arguments)]
    fn py_new(
        exec_algorithm_id: Option<nautilus_model::identifiers::ExecAlgorithmId>,
        log_events: bool,
        log_commands: bool,
    ) -> Self {
        Self {
            exec_algorithm_id,
            log_events,
            log_commands,
        }
    }

    #[getter]
    fn exec_algorithm_id(&self) -> Option<nautilus_model::identifiers::ExecAlgorithmId> {
        self.exec_algorithm_id
    }

    #[getter]
    fn log_events(&self) -> bool {
        self.log_events
    }

    #[getter]
    fn log_commands(&self) -> bool {
        self.log_commands
    }
}
