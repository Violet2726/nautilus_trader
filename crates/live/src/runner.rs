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

use std::{fmt::Debug, sync::Arc};

use nautilus_common::{
    live::runner::{set_data_event_sender, set_exec_event_sender},
    messages::{
        DataEvent, ExecutionEvent, ExecutionReport, data::DataCommand, execution::TradingCommand,
    },
    msgbus::{self, MessagingSwitchboard},
    runner::{
        DataCommandSender, TimeEventSender, TradingCommandSender, set_data_cmd_sender,
        set_exec_cmd_sender, set_time_event_sender,
    },
    timer::TimeEventHandler,
};

/// 实盘环境下的 `DataCommandSender` 异步实现。
#[derive(Debug)]
pub struct AsyncDataCommandSender {
    cmd_tx: tokio::sync::mpsc::UnboundedSender<DataCommand>,
}

impl AsyncDataCommandSender {
    #[must_use]
    pub const fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<DataCommand>) -> Self {
        Self { cmd_tx }
    }
}

impl DataCommandSender for AsyncDataCommandSender {
    fn execute(&self, command: DataCommand) {
        if let Err(e) = self.cmd_tx.send(command) {
            log::error!("发送数据命令失败: {e}");
        }
    }
}

/// 实盘环境下的 `TimeEventSender` 异步实现。
#[derive(Debug, Clone)]
pub struct AsyncTimeEventSender {
    time_tx: tokio::sync::mpsc::UnboundedSender<TimeEventHandler>,
}

impl AsyncTimeEventSender {
    #[must_use]
    pub const fn new(time_tx: tokio::sync::mpsc::UnboundedSender<TimeEventHandler>) -> Self {
        Self { time_tx }
    }

    /// 获取底层通道发送端的克隆以供异步使用。
    ///
    /// 这允许异步上下文获取一个可以直接移动到异步任务中的通道发送端，
    /// 从而避免 `RefCell` 借用问题。
    #[must_use]
    pub fn get_channel_sender(&self) -> tokio::sync::mpsc::UnboundedSender<TimeEventHandler> {
        self.time_tx.clone()
    }
}

impl TimeEventSender for AsyncTimeEventSender {
    fn send(&self, handler: TimeEventHandler) {
        if let Err(e) = self.time_tx.send(handler) {
            log::error!("发送时间事件处理程序失败: {e}");
        }
    }
}

/// 实盘环境下的 `TradingCommandSender` 异步实现。
#[derive(Debug)]
pub struct AsyncTradingCommandSender {
    cmd_tx: tokio::sync::mpsc::UnboundedSender<TradingCommand>,
}

impl AsyncTradingCommandSender {
    #[must_use]
    pub const fn new(cmd_tx: tokio::sync::mpsc::UnboundedSender<TradingCommand>) -> Self {
        Self { cmd_tx }
    }
}

impl TradingCommandSender for AsyncTradingCommandSender {
    fn execute(&self, command: TradingCommand) {
        if let Err(e) = self.cmd_tx.send(command) {
            log::error!("发送交易命令失败: {e}");
        }
    }
}

pub trait Runner {
    fn run(&mut self);
}

/// 异步事件循环的通道接收端。
///
/// 可以通过 `take_channels()` 从 `AsyncRunner` 中提取这些接收端，
/// 以便在与消息总线（msgbus）端点相同的线程上直接驱动事件循环。
#[derive(Debug)]
pub struct AsyncRunnerChannels {
    pub time_evt_rx: tokio::sync::mpsc::UnboundedReceiver<TimeEventHandler>,
    pub data_evt_rx: tokio::sync::mpsc::UnboundedReceiver<DataEvent>,
    pub data_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<DataCommand>,
    pub exec_evt_rx: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
    pub exec_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<TradingCommand>,
}

pub struct AsyncRunner {
    channels: AsyncRunnerChannels,
    signal_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
    signal_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

/// 用于从另一个上下文停止 `AsyncRunner` 的句柄。
#[derive(Clone, Debug)]
pub struct AsyncRunnerHandle {
    signal_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

impl AsyncRunnerHandle {
    /// 发送信号停止运行器。
    pub fn stop(&self) {
        if let Err(e) = self.signal_tx.send(()) {
            log::error!("发送关闭信号失败: {e}");
        }
    }
}

impl Default for AsyncRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for AsyncRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(AsyncRunner)).finish()
    }
}

impl AsyncRunner {
    /// 创建一个新的 [`AsyncRunner`] 实例。
    #[must_use]
    pub fn new() -> Self {
        use tokio::sync::mpsc::unbounded_channel; // tokio-import-ok

        let (time_evt_tx, time_evt_rx) = unbounded_channel::<TimeEventHandler>();
        let (data_cmd_tx, data_cmd_rx) = unbounded_channel::<DataCommand>();
        let (data_evt_tx, data_evt_rx) = unbounded_channel::<DataEvent>();
        let (exec_cmd_tx, exec_cmd_rx) = unbounded_channel::<TradingCommand>();
        let (exec_evt_tx, exec_evt_rx) = unbounded_channel::<ExecutionEvent>();
        let (signal_tx, signal_rx) = unbounded_channel::<()>();

        set_time_event_sender(Arc::new(AsyncTimeEventSender::new(time_evt_tx)));
        set_data_cmd_sender(Arc::new(AsyncDataCommandSender::new(data_cmd_tx)));
        set_data_event_sender(data_evt_tx);
        set_exec_cmd_sender(Arc::new(AsyncTradingCommandSender::new(exec_cmd_tx)));
        set_exec_event_sender(exec_evt_tx);

        Self {
            channels: AsyncRunnerChannels {
                time_evt_rx,
                data_evt_rx,
                data_cmd_rx,
                exec_evt_rx,
                exec_cmd_rx,
            },
            signal_rx,
            signal_tx,
        }
    }

    /// 使用内部关闭信号停止运行器。
    pub fn stop(&self) {
        if let Err(e) = self.signal_tx.send(()) {
            log::error!("发送关闭信号失败: {e}");
        }
    }

    /// 返回一个句柄，可用于从另一个上下文停止运行器。
    #[must_use]
    pub fn handle(&self) -> AsyncRunnerHandle {
        AsyncRunnerHandle {
            signal_tx: self.signal_tx.clone(),
        }
    }

    /// 消耗运行器并返回通道接收端，以便直接驱动事件循环。
    ///
    /// 当事件循环需要与消息总线（msgbus）端点（使用线程局部存储）在同一线程上运行时使用此方法。
    #[must_use]
    pub fn take_channels(self) -> AsyncRunnerChannels {
        self.channels
    }

    /// 从通道中排出所有待处理的数据事件并处理它们。
    pub fn drain_pending_data_events(&mut self) {
        let mut count = 0;
        while let Ok(evt) = self.channels.data_evt_rx.try_recv() {
            Self::handle_data_event(evt);
            count += 1;
        }
        if count > 0 {
            log::debug!("已排出 {count} 个待处理的数据事件");
        }
    }

    /// 运行异步运行器事件循环。
    ///
    /// 此方法在异步循环中处理数据事件、时间事件、执行事件和信号事件。
    /// 它将一直运行直到收到信号或事件流关闭。
    pub async fn run(&mut self) {
        log::info!("AsyncRunner 正在启动");

        loop {
            tokio::select! {
                Some(()) = self.signal_rx.recv() => {
                    log::info!("AsyncRunner 收到信号，正在关闭");
                    return;
                },
                Some(handler) = self.channels.time_evt_rx.recv() => {
                    Self::handle_time_event(handler);
                },
                Some(cmd) = self.channels.data_cmd_rx.recv() => {
                    Self::handle_data_command(cmd);
                },
                Some(evt) = self.channels.data_evt_rx.recv() => {
                    Self::handle_data_event(evt);
                },
                Some(cmd) = self.channels.exec_cmd_rx.recv() => {
                    Self::handle_exec_command(cmd);
                },
                Some(evt) = self.channels.exec_evt_rx.recv() => {
                    Self::handle_exec_event(evt);
                },
                else => {
                    log::debug!("AsyncRunner 所有通道已关闭，正在退出");
                    return;
                }
            };
        }
    }

    /// 通过运行回调来处理时间事件。
    #[inline]
    pub fn handle_time_event(handler: TimeEventHandler) {
        handler.run();
    }

    /// 通过发送给 DataEngine 来处理数据命令。
    #[inline]
    pub fn handle_data_command(cmd: DataCommand) {
        msgbus::send_data_command(MessagingSwitchboard::data_engine_execute(), cmd);
    }

    /// 通过发送给适当的 DataEngine 端点来处理数据事件。
    #[inline]
    pub fn handle_data_event(event: DataEvent) {
        match event {
            DataEvent::Data(data) => {
                msgbus::send_data(MessagingSwitchboard::data_engine_process_data(), data);
            }
            DataEvent::Instrument(data) => {
                msgbus::send_any(MessagingSwitchboard::data_engine_process(), &data);
            }
            DataEvent::Response(resp) => {
                msgbus::send_data_response(MessagingSwitchboard::data_engine_response(), resp);
            }
            DataEvent::FundingRate(funding_rate) => {
                msgbus::send_any(MessagingSwitchboard::data_engine_process(), &funding_rate);
            }
            #[cfg(feature = "defi")]
            DataEvent::DeFi(data) => {
                msgbus::send_defi_data(MessagingSwitchboard::data_engine_process_defi_data(), data);
            }
        }
    }

    /// 通过发送给 ExecEngine 来处理执行命令。
    #[inline]
    pub fn handle_exec_command(cmd: TradingCommand) {
        msgbus::send_trading_command(MessagingSwitchboard::exec_engine_execute(), cmd);
    }

    /// 通过发送给适当的引擎端点来处理执行事件。
    #[inline]
    pub fn handle_exec_event(event: ExecutionEvent) {
        match event {
            ExecutionEvent::Order(order_event) => {
                msgbus::send_order_event(MessagingSwitchboard::exec_engine_process(), order_event);
            }
            ExecutionEvent::Report(report) => {
                Self::handle_exec_report(report);
            }
            ExecutionEvent::Account(ref account) => {
                msgbus::send_account_state(
                    MessagingSwitchboard::portfolio_update_account(),
                    account,
                );
            }
        }
    }

    #[inline]
    pub fn handle_exec_report(report: ExecutionReport) {
        let endpoint = MessagingSwitchboard::exec_engine_reconcile_execution_report();
        msgbus::send_execution_report(endpoint, report);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nautilus_common::{
        messages::{
            ExecutionEvent, ExecutionReport,
            data::{SubscribeCommand, SubscribeCustomData},
            execution::TradingCommand,
        },
        timer::{TimeEvent, TimeEventCallback, TimeEventHandler},
    };
    use nautilus_core::{UUID4, UnixNanos};
    use nautilus_model::{
        data::{Data, DataType, quote::QuoteTick},
        enums::{
            AccountType, LiquiditySide, OrderSide, OrderStatus, OrderType, PositionSideSpecified,
            TimeInForce,
        },
        events::{OrderEvent, OrderEventAny, OrderSubmitted, account::state::AccountState},
        identifiers::{
            AccountId, ClientId, ClientOrderId, InstrumentId, PositionId, StrategyId, TradeId,
            TraderId, VenueOrderId,
        },
        reports::{FillReport, OrderStatusReport, PositionStatusReport},
        types::{Money, Price, Quantity},
    };
    use rstest::rstest;
    use ustr::Ustr;

    use super::*;

    // Test fixture for creating test quotes
    fn test_quote() -> QuoteTick {
        QuoteTick {
            instrument_id: InstrumentId::from("EUR/USD.SIM"),
            bid_price: Price::from("1.10000"),
            ask_price: Price::from("1.10001"),
            bid_size: Quantity::from(1_000_000),
            ask_size: Quantity::from(1_000_000),
            ts_event: UnixNanos::default(),
            ts_init: UnixNanos::default(),
        }
    }

    // Test helper to create AsyncRunner with manual channels
    fn create_test_runner(
        time_evt_rx: tokio::sync::mpsc::UnboundedReceiver<TimeEventHandler>,
        data_evt_rx: tokio::sync::mpsc::UnboundedReceiver<DataEvent>,
        data_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<DataCommand>,
        exec_evt_rx: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
        exec_cmd_rx: tokio::sync::mpsc::UnboundedReceiver<TradingCommand>,
        signal_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
        signal_tx: tokio::sync::mpsc::UnboundedSender<()>,
    ) -> AsyncRunner {
        AsyncRunner {
            channels: AsyncRunnerChannels {
                time_evt_rx,
                data_evt_rx,
                data_cmd_rx,
                exec_evt_rx,
                exec_cmd_rx,
            },
            signal_rx,
            signal_tx,
        }
    }

    #[rstest]
    fn test_async_data_command_sender_creation() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncDataCommandSender::new(tx);
        assert!(format!("{sender:?}").contains("AsyncDataCommandSender"));
    }

    #[rstest]
    fn test_async_time_event_sender_creation() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncTimeEventSender::new(tx);
        assert!(format!("{sender:?}").contains("AsyncTimeEventSender"));
    }

    #[rstest]
    fn test_async_time_event_sender_get_channel() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncTimeEventSender::new(tx);
        let channel = sender.get_channel_sender();

        // Verify the channel is functional
        let event = TimeEvent::new(
            Ustr::from("test"),
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
        );
        let callback = TimeEventCallback::from(|_: TimeEvent| {});
        let handler = TimeEventHandler::new(event, callback);

        assert!(channel.send(handler).is_ok());
    }

    #[tokio::test]
    async fn test_async_data_command_sender_execute() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncDataCommandSender::new(tx);

        let command = DataCommand::Subscribe(SubscribeCommand::Data(SubscribeCustomData {
            client_id: Some(ClientId::from("TEST")),
            venue: None,
            data_type: DataType::new("QuoteTick", None),
            command_id: UUID4::new(),
            ts_init: UnixNanos::default(),
            correlation_id: None,
            params: None,
        }));

        sender.execute(command.clone());

        let received = rx.recv().await.unwrap();
        match (received, command) {
            (
                DataCommand::Subscribe(SubscribeCommand::Data(r)),
                DataCommand::Subscribe(SubscribeCommand::Data(c)),
            ) => {
                assert_eq!(r.client_id, c.client_id);
                assert_eq!(r.data_type, c.data_type);
            }
            _ => panic!("Command mismatch"),
        }
    }

    #[tokio::test]
    async fn test_async_time_event_sender_send() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncTimeEventSender::new(tx);

        let event = TimeEvent::new(
            Ustr::from("test"),
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
        );
        let callback = TimeEventCallback::from(|_: TimeEvent| {});
        let handler = TimeEventHandler::new(event, callback);

        sender.send(handler);

        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_runner_shutdown_signal() {
        // Create runner with manual channels to avoid global state
        let (_data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventHandler>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommand>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Start runner
        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        // Send shutdown signal
        signal_tx.send(()).unwrap();

        // Runner should stop quickly
        let result = tokio::time::timeout(Duration::from_millis(100), runner_handle).await;
        assert!(result.is_ok(), "Runner should stop on signal");
    }

    #[tokio::test]
    async fn test_runner_closes_on_channel_drop() {
        let (data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventHandler>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommand>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Start runner
        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        drop(data_tx);

        // Yield to let runner enter event loop before stop signal
        tokio::task::yield_now().await;
        signal_tx.send(()).ok();

        // Runner should stop when channels close or on signal
        let result = tokio::time::timeout(Duration::from_millis(200), runner_handle).await;
        assert!(
            result.is_ok(),
            "Runner should stop when channels close or on signal"
        );
    }

    #[tokio::test]
    async fn test_concurrent_event_sending() {
        let (data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_evt_tx, time_evt_rx) =
            tokio::sync::mpsc::unbounded_channel::<TimeEventHandler>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommand>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        // Setup runner
        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Spawn multiple concurrent senders
        let mut handles = vec![];
        for _ in 0..5 {
            let tx_clone = data_evt_tx.clone();
            let handle = tokio::spawn(async move {
                for _ in 0..20 {
                    let quote = test_quote();
                    tx_clone.send(DataEvent::Data(Data::Quote(quote))).unwrap();
                    tokio::task::yield_now().await;
                }
            });
            handles.push(handle);
        }

        // Start runner in background
        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        // Wait for all senders
        for handle in handles {
            handle.await.unwrap();
        }

        // Yield to let runner enter event loop before stop signal
        tokio::task::yield_now().await;
        signal_tx.send(()).unwrap();

        let _ = tokio::time::timeout(Duration::from_millis(200), runner_handle).await;
    }

    #[rstest]
    #[case(10)]
    #[case(100)]
    #[case(1000)]
    fn test_channel_send_performance(#[case] count: usize) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let quote = test_quote();

        // Send events
        for _ in 0..count {
            tx.send(DataEvent::Data(Data::Quote(quote))).unwrap();
        }

        // Verify all received
        let mut received = 0;
        while rx.try_recv().is_ok() {
            received += 1;
        }

        assert_eq!(received, count);
    }

    #[rstest]
    fn test_async_trading_command_sender_creation() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = AsyncTradingCommandSender::new(tx);
        assert!(format!("{sender:?}").contains("AsyncTradingCommandSender"));
    }

    #[tokio::test]
    async fn test_execution_event_order_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let event = OrderSubmitted::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("S-001"),
            InstrumentId::from("EUR/USD.SIM"),
            ClientOrderId::from("O-001"),
            AccountId::from("SIM-001"),
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
        );

        tx.send(ExecutionEvent::Order(OrderEventAny::Submitted(event)))
            .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Order(OrderEventAny::Submitted(e)) => {
                assert_eq!(e.client_order_id(), ClientOrderId::from("O-001"));
            }
            _ => panic!("Expected OrderSubmitted event"),
        }
    }

    #[tokio::test]
    async fn test_execution_report_order_status_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let report = OrderStatusReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            Some(ClientOrderId::from("O-001")),
            VenueOrderId::from("V-001"),
            OrderSide::Buy,
            OrderType::Market,
            TimeInForce::Gtc,
            OrderStatus::Accepted,
            Quantity::from(100_000),
            Quantity::from(100_000),
            UnixNanos::from(1),
            UnixNanos::from(2),
            UnixNanos::from(3),
            None,
        );

        tx.send(ExecutionEvent::Report(ExecutionReport::Order(Box::new(
            report,
        ))))
        .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Report(ExecutionReport::Order(r)) => {
                assert_eq!(r.venue_order_id.as_str(), "V-001");
                assert_eq!(r.order_status, OrderStatus::Accepted);
            }
            _ => panic!("Expected OrderStatusReport"),
        }
    }

    #[tokio::test]
    async fn test_execution_report_fill() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let report = FillReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            VenueOrderId::from("V-001"),
            TradeId::from("T-001"),
            OrderSide::Buy,
            Quantity::from(100_000),
            Price::from("1.10000"),
            Money::from("10 USD"),
            LiquiditySide::Taker,
            Some(ClientOrderId::from("O-001")),
            None,
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
        );

        tx.send(ExecutionEvent::Report(ExecutionReport::Fill(Box::new(
            report,
        ))))
        .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Report(ExecutionReport::Fill(r)) => {
                assert_eq!(r.venue_order_id.as_str(), "V-001");
                assert_eq!(r.trade_id.to_string(), "T-001");
            }
            _ => panic!("Expected FillReport"),
        }
    }

    #[tokio::test]
    async fn test_execution_report_position() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let report = PositionStatusReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            PositionSideSpecified::Long,
            Quantity::from(100_000),
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
            Some(PositionId::from("P-001")),
            None,
        );

        tx.send(ExecutionEvent::Report(ExecutionReport::Position(Box::new(
            report,
        ))))
        .unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Report(ExecutionReport::Position(r)) => {
                assert_eq!(r.venue_position_id.unwrap().as_str(), "P-001");
            }
            _ => panic!("Expected PositionStatusReport"),
        }
    }

    #[tokio::test]
    async fn test_execution_event_account() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();

        let account_state = AccountState::new(
            AccountId::from("SIM-001"),
            AccountType::Cash,
            vec![],
            vec![],
            true,
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
        );

        tx.send(ExecutionEvent::Account(account_state)).unwrap();

        let received = rx.recv().await.unwrap();
        match received {
            ExecutionEvent::Account(r) => {
                assert_eq!(r.account_id.as_str(), "SIM-001");
            }
            _ => panic!("Expected AccountState"),
        }
    }

    #[tokio::test]
    async fn test_runner_stop_method() {
        let (_data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventHandler>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommand>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        // Use stop via signal_tx directly
        signal_tx.send(()).unwrap();

        let result = tokio::time::timeout(Duration::from_millis(100), runner_handle).await;
        assert!(result.is_ok(), "Runner should stop when stop() is called");
    }

    #[tokio::test]
    async fn test_all_event_types_integration() {
        let (data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (data_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (time_evt_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventHandler>();
        let (exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommand>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        // Send data event
        let quote = test_quote();
        data_evt_tx
            .send(DataEvent::Data(Data::Quote(quote)))
            .unwrap();

        // Send data command
        let command = DataCommand::Subscribe(SubscribeCommand::Data(SubscribeCustomData {
            client_id: Some(ClientId::from("TEST")),
            venue: None,
            data_type: DataType::new("QuoteTick", None),
            command_id: UUID4::new(),
            ts_init: UnixNanos::default(),
            correlation_id: None,
            params: None,
        }));
        data_cmd_tx.send(command).unwrap();

        // Send time event
        let event = TimeEvent::new(
            Ustr::from("test"),
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
        );
        let callback = TimeEventCallback::from(|_: TimeEvent| {});
        let handler = TimeEventHandler::new(event, callback);
        time_evt_tx.send(handler).unwrap();

        // Send execution order event
        let order_event = OrderSubmitted::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("S-001"),
            InstrumentId::from("EUR/USD.SIM"),
            ClientOrderId::from("O-001"),
            AccountId::from("SIM-001"),
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
        );
        exec_evt_tx
            .send(ExecutionEvent::Order(OrderEventAny::Submitted(order_event)))
            .unwrap();

        // Send execution report (OrderStatus)
        let order_status = OrderStatusReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            Some(ClientOrderId::from("O-001")),
            VenueOrderId::from("V-001"),
            OrderSide::Buy,
            OrderType::Market,
            TimeInForce::Gtc,
            OrderStatus::Accepted,
            Quantity::from(100_000),
            Quantity::from(100_000),
            UnixNanos::from(1),
            UnixNanos::from(2),
            UnixNanos::from(3),
            None,
        );
        exec_evt_tx
            .send(ExecutionEvent::Report(ExecutionReport::Order(Box::new(
                order_status,
            ))))
            .unwrap();

        // Send execution report (Fill)
        let fill = FillReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            VenueOrderId::from("V-001"),
            TradeId::from("T-001"),
            OrderSide::Buy,
            Quantity::from(100_000),
            Price::from("1.10000"),
            Money::from("10 USD"),
            LiquiditySide::Taker,
            Some(ClientOrderId::from("O-001")),
            None,
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
        );
        exec_evt_tx
            .send(ExecutionEvent::Report(ExecutionReport::Fill(Box::new(
                fill,
            ))))
            .unwrap();

        // Send execution report (Position)
        let position = PositionStatusReport::new(
            AccountId::from("SIM-001"),
            InstrumentId::from("EUR/USD.SIM"),
            PositionSideSpecified::Long,
            Quantity::from(100_000),
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
            Some(PositionId::from("P-001")),
            None,
        );
        exec_evt_tx
            .send(ExecutionEvent::Report(ExecutionReport::Position(Box::new(
                position,
            ))))
            .unwrap();

        // Send account event
        let account_state = AccountState::new(
            AccountId::from("SIM-001"),
            AccountType::Cash,
            vec![],
            vec![],
            true,
            UUID4::new(),
            UnixNanos::from(1),
            UnixNanos::from(2),
            None,
        );
        exec_evt_tx
            .send(ExecutionEvent::Account(account_state))
            .unwrap();

        // Yield to let runner enter event loop before stop signal
        tokio::task::yield_now().await;
        signal_tx.send(()).unwrap();

        let result = tokio::time::timeout(Duration::from_millis(200), runner_handle).await;
        assert!(
            result.is_ok(),
            "Runner should process all event types and stop cleanly"
        );
    }

    #[tokio::test]
    async fn test_runner_handle_stops_runner() {
        let (_data_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventHandler>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommand>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        // Get handle before moving runner
        let handle = runner.handle();

        let runner_task = tokio::spawn(async move {
            runner.run().await;
        });

        // Use handle to stop
        handle.stop();

        let result = tokio::time::timeout(Duration::from_millis(100), runner_task).await;
        assert!(result.is_ok(), "Runner should stop via handle");
    }

    #[tokio::test]
    async fn test_runner_handle_is_cloneable() {
        let (signal_tx, _signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let handle = AsyncRunnerHandle { signal_tx };

        let handle2 = handle.clone();

        // Both handles should be able to send stop signals
        assert!(handle.signal_tx.send(()).is_ok());
        assert!(handle2.signal_tx.send(()).is_ok());
    }

    #[tokio::test]
    async fn test_runner_processes_events_before_stop() {
        let (data_evt_tx, data_evt_rx) = tokio::sync::mpsc::unbounded_channel::<DataEvent>();
        let (_cmd_tx, data_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<DataCommand>();
        let (_time_tx, time_evt_rx) = tokio::sync::mpsc::unbounded_channel::<TimeEventHandler>();
        let (_exec_evt_tx, exec_evt_rx) = tokio::sync::mpsc::unbounded_channel::<ExecutionEvent>();
        let (_exec_cmd_tx, exec_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<TradingCommand>();
        let (signal_tx, signal_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        let mut runner = create_test_runner(
            time_evt_rx,
            data_evt_rx,
            data_cmd_rx,
            exec_evt_rx,
            exec_cmd_rx,
            signal_rx,
            signal_tx.clone(),
        );

        let handle = runner.handle();

        // Send events before starting runner
        for _ in 0..10 {
            let quote = test_quote();
            data_evt_tx
                .send(DataEvent::Data(Data::Quote(quote)))
                .unwrap();
        }

        let runner_task = tokio::spawn(async move {
            runner.run().await;
        });

        // Yield to let runner enter event loop before stop signal
        tokio::task::yield_now().await;
        handle.stop();

        let result = tokio::time::timeout(Duration::from_millis(200), runner_task).await;
        assert!(result.is_ok(), "Runner should process events and stop");
    }
}
