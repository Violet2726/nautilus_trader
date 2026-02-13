from __future__ import annotations

import contextlib
import datetime
import json
import time
from dataclasses import dataclass
from threading import Lock
from typing import Any

from nautilus_trader.adapters.fix.client.enums import BookingType
from nautilus_trader.adapters.fix.client.enums import FixMsgType
from nautilus_trader.adapters.fix.client.enums import HandlInst
from nautilus_trader.adapters.fix.client.enums import OrdType
from nautilus_trader.adapters.fix.client.enums import QueryType
from nautilus_trader.adapters.fix.client.enums import SecurityType
from nautilus_trader.adapters.fix.client.enums import Side
from nautilus_trader.adapters.fix.client.enums import StorageTopic
from nautilus_trader.adapters.fix.client.enums import TimeInForce
from nautilus_trader.adapters.fix.client.enums import UserRequestType
from nautilus_trader.adapters.fix.client.enums import UserStatus
from nautilus_trader.adapters.fix.client.enums import WindCustomTags
from nautilus_trader.adapters.fix.client.hardware import get_exchange_info_v3
from nautilus_trader.adapters.fix.client.hardware import get_hardware_info
from nautilus_trader.adapters.fix.client.parser import FIXTreeParser
from nautilus_trader.adapters.fix.client.quickfix import fix


@dataclass(frozen=True)
class FixCredentials:
    username: str
    password: str


class WindFixApplication:
    def __init__(self, dictionary_path: str, credentials: FixCredentials):
        if fix is None:
            raise ModuleNotFoundError("quickfix is required to use the FIX adapter")

        self._fix = fix
        self._credentials = credentials
        self._dictionary_path = dictionary_path

        self.session_id: Any = None
        self.lock = Lock()
        self.hardware_info = get_hardware_info()
        self.parser = FIXTreeParser(dictionary_path)

        self.trade_cache: dict[str, dict[str, Any]] = {
            StorageTopic.ORDERS.value: {},
            StorageTopic.DEALS.value: {},
            StorageTopic.POSITIONS.value: {},
            StorageTopic.ACCOUNTS.value: {},
            StorageTopic.EXEC_ORDER.value: {},
        }

        self.on_execution_report: Any = None

    def _as_application(self) -> Any:
        fix_mod = self._fix

        outer = self

        def onCreate(self: Any, session_id: Any) -> None:
            return None

        def onLogon(self: Any, session_id: Any) -> None:
            outer.session_id = session_id
            outer._app_logon()

        def onLogout(self: Any, session_id: Any) -> None:
            return None

        def toApp(self: Any, message: Any, session_id: Any) -> None:
            return None

        def fromAdmin(self: Any, message: Any, session_id: Any) -> None:
            return None

        def toAdmin(self: Any, message: Any, session_id: Any) -> None:
            outer._to_admin(message, session_id)
            return None

        def fromApp(self: Any, message: Any, session_id: Any) -> None:
            outer._from_app(message)
            return None

        app_cls = type(
            "_App",
            (fix_mod.Application,),
            {
                "onCreate": onCreate,
                "onLogon": onLogon,
                "onLogout": onLogout,
                "toApp": toApp,
                "fromAdmin": fromAdmin,
                "toAdmin": toAdmin,
                "fromApp": fromApp,
            },
        )
        return app_cls()

    def _to_admin(self, message: Any, session_id: Any) -> None:
        fix_mod = self._fix
        msg_type = fix_mod.MsgType()
        message.getHeader().getField(msg_type)
        if msg_type.getValue() != fix_mod.MsgType_Logon:
            return

        with contextlib.suppress(Exception):
            begin_string = None
            with contextlib.suppress(Exception):
                begin_string = session_id.getBeginString()
            if begin_string and str(begin_string).startswith("FIXT") and not message.isSetField(1137):
                message.setField(fix_mod.StringField(1137, "6"))

        with contextlib.suppress(Exception):
            message.getHeader().setField(fix_mod.SenderSubID(self._credentials.username))
            from_ip = self.hardware_info.get("from_ip")
            if from_ip:
                message.getHeader().setField(fix_mod.SenderLocationID(str(from_ip)))

        with contextlib.suppress(Exception):
            if not message.isSetField(553):
                message.setField(fix_mod.Username(self._credentials.username))
            if not message.isSetField(554):
                message.setField(fix_mod.Password(self._credentials.password))

        with contextlib.suppress(Exception):
            raw_data = json.dumps(self.hardware_info, ensure_ascii=False, separators=(",", ":"))
            raw_data_bytes = raw_data.encode("utf-8")
            message.setField(fix_mod.RawDataLength(len(raw_data_bytes)))
            message.setField(fix_mod.RawData(raw_data))

    def _from_app(self, message: Any) -> None:
        fix_mod = self._fix
        msg_type = fix_mod.MsgType()
        message.getHeader().getField(msg_type)
        msg_type_val = msg_type.getValue()

        if msg_type_val == FixMsgType.USER_RESPONSE.value:
            self._after_login_init(message)
            return

        if msg_type_val == FixMsgType.QUERY_RESPONSE.value:
            storge_type = (
                StorageTopic.ORDERS.value
                if int(message.getField(fix_mod.IntField(WindCustomTags.QueryType.value).getField()))
                == QueryType.ORDER.value
                else StorageTopic.DEALS.value
            )
            self._handle_execution_report(message, storge_type)
            return

        if msg_type_val == FixMsgType.HOLDING_QUERY_RESPONSE.value:
            self._handle_execution_report(message, StorageTopic.POSITIONS.value)
            return

        if msg_type_val == FixMsgType.FUNDING_QUERY_RESPONSE.value:
            self._handle_execution_report(message, StorageTopic.ACCOUNTS.value)
            return

        if msg_type_val == FixMsgType.EXECUTION_REPORT.value:
            self._handle_execution_order(message, StorageTopic.EXEC_ORDER.value)
            if self.on_execution_report is not None:
                with contextlib.suppress(Exception):
                    self.on_execution_report(message.toString())
            return

    def _after_login_init(self, message: Any) -> None:
        fix_mod = self._fix
        user_req_type_obj = fix_mod.UserRequestType()
        user_status_obj = fix_mod.UserStatus()
        message.getField(user_req_type_obj)
        message.getField(user_status_obj)
        req_val = user_req_type_obj.getValue()
        status_val = user_status_obj.getValue()
        if str(req_val) != str(UserRequestType.LOGIN.value) or str(status_val) != str(UserStatus.LOGGED_IN.value):
            raise RuntimeError(f"Login rejected: {status_val}")

    def _app_logon(self) -> None:
        fix_mod = self._fix
        message = fix_mod.Message()
        header = message.getHeader()
        header.setField(fix_mod.MsgType(FixMsgType.USER_REQUEST.value))
        header.setField(fix_mod.SenderSubID(self._credentials.username))

        user_req_id = f"CXL{datetime.datetime.now(tz=datetime.UTC).strftime('%H%M%S%f')}"
        message.setField(fix_mod.UserRequestID(user_req_id))
        message.setField(fix_mod.UserRequestType(UserRequestType.LOGIN.value))
        message.setField(fix_mod.Username(self._credentials.username))
        message.setField(fix_mod.Password(self._credentials.password))
        raw_data = json.dumps(self.hardware_info)
        message.setField(fix_mod.RawDataLength(len(raw_data.encode("utf-8"))))
        message.setField(fix_mod.RawData(raw_data))

        if self.session_id is not None:
            fix_mod.Session.sendToTarget(message, self.session_id)

    def _handle_execution_report(self, message: Any, storge_type: str) -> None:
        fix_mod = self._fix
        account_id = message.getField(fix_mod.Account().getField())
        with self.lock:
            self.trade_cache[storge_type][account_id] = self._message_to_dict(message)

    def _handle_execution_order(self, message: Any, storge_type: str) -> None:
        fix_mod = self._fix
        cl_order_id = message.getField(fix_mod.ClOrdID().getField())
        with self.lock:
            self.trade_cache[storge_type][cl_order_id] = self._message_to_dict(message)

    def _message_to_dict(self, message: Any) -> str:
        tree_json = self.parser.parse(message)
        return json.dumps(tree_json, indent=4)

    def _get_value_waite_time(self, cache_key: str, sub_key: str, timeout: float = 30) -> Any:
        start_time = time.time()
        while time.time() - start_time < timeout:
            with self.lock:
                result = self.trade_cache.get(cache_key, {}).get(sub_key)
                if result is not None and result != {}:
                    return result
            time.sleep(0.5)
        return {}

    def passorder(
        self,
        opt_ype: int,
        account_id: str,
        order_code: str,
        pr_type: int,
        volume: float,
        user_order_id: str,
        price: float | None = None,
    ) -> str | None:
        fix_mod = self._fix
        exchange, _ = get_exchange_info_v3(order_code)

        message = fix_mod.Message()
        header = message.getHeader()
        header.setField(fix_mod.MsgType(fix_mod.MsgType_NewOrderSingle))
        header.setField(fix_mod.SenderSubID(self._credentials.username))

        cl_ord_id = str(user_order_id)
        message.setField(fix_mod.ClOrdID(cl_ord_id))
        message.setField(fix_mod.HandlInst(HandlInst.LowTouch.value))
        side = Side.BUY.value if int(opt_ype) == 23 else Side.SELL.value
        message.setField(fix_mod.Side(side))
        utc_now = datetime.datetime.now(tz=datetime.UTC).strftime("%Y%m%d-%H:%M:%S")
        message.setField(fix_mod.StringField(60, utc_now))
        message.setField(fix_mod.OrderQty(float(volume)))
        ord_type = OrdType.LIMIT.value if int(pr_type) == 11 else OrdType.MARKET.value
        message.setField(fix_mod.OrdType(ord_type))
        if ord_type == OrdType.LIMIT.value:
            if price is None:
                raise ValueError("Price is required for limit orders")
            message.setField(fix_mod.Price(float(price)))

        message.setField(fix_mod.Symbol(order_code))
        message.setField(fix_mod.SecurityExchange(exchange))
        message.setField(fix_mod.TimeInForce(TimeInForce.GFD.value))
        message.setField(fix_mod.SecurityType(SecurityType.COMMON_STOCK.value))
        message.setField(fix_mod.BookingType(BookingType.CASH.value))
        message.setField(fix_mod.Account(str(account_id)))

        try:
            if self.session_id is not None:
                fix_mod.Session.sendToTarget(message, self.session_id)
        except Exception:
            return None

        result: Any = self._get_value_waite_time(StorageTopic.EXEC_ORDER.value, cl_ord_id)
        if result is not None and result != {}:
            return json.loads(result).get("OrderID")
        return None

    def cancel(self, order_id: str, account_id: str, side: str) -> str | None:
        fix_mod = self._fix
        message = fix_mod.Message()
        header = message.getHeader()
        header.setField(fix_mod.MsgType(fix_mod.MsgType_OrderCancelRequest))
        header.setField(fix_mod.SenderSubID(self._credentials.username))

        cancel_req_id = f"CXL{datetime.datetime.now(tz=datetime.UTC).strftime('%H%M%S%f')}"
        message.setField(fix_mod.ClOrdID(cancel_req_id))
        message.setField(fix_mod.OrigClOrdID(str(order_id)))
        message.setField(fix_mod.Side(str(side)))
        utc_now = datetime.datetime.now(tz=datetime.UTC).strftime("%Y%m%d-%H:%M:%S")
        message.setField(fix_mod.StringField(60, utc_now))
        message.setField(fix_mod.Account(str(account_id)))

        try:
            if self.session_id is not None:
                fix_mod.Session.sendToTarget(message, self.session_id)
        except Exception:
            return None

        result: Any = self._get_value_waite_time(StorageTopic.EXEC_ORDER.value, cancel_req_id)
        if result is not None and result != {}:
            return json.loads(result).get("OrderID")
        return None

    def get_trade_detail_data(self, account_id: str, str_data_type: str) -> Any:
        if fix is None:
            raise ModuleNotFoundError("quickfix is required to use the FIX adapter")

        result: Any = {}
        if str_data_type == "ACCOUNT":
            self.trade_cache[StorageTopic.ACCOUNTS.value][account_id] = {}
            self._query_positions(query_type=4, account_id=account_id)
            result = self._get_value_waite_time(StorageTopic.ACCOUNTS.value, account_id)
        if str_data_type == "POSITION":
            self.trade_cache[StorageTopic.POSITIONS.value][account_id] = {}
            self._query_positions(query_type=3, account_id=account_id)
            result = self._get_value_waite_time(StorageTopic.POSITIONS.value, account_id)
        elif str_data_type == "ORDER":
            self.trade_cache[StorageTopic.ORDERS.value][account_id] = {}
            self._query_deal_data(query_type=1, account_id=account_id)
            result = self._get_value_waite_time(StorageTopic.ORDERS.value, account_id)
        elif str_data_type == "DEAL":
            self.trade_cache[StorageTopic.DEALS.value][account_id] = {}
            self._query_deal_data(query_type=2, account_id=account_id)
            result = self._get_value_waite_time(StorageTopic.DEALS.value, account_id)
        return result

    def _query_deal_data(self, query_type: int, account_id: str) -> None:
        fix_mod = self._fix
        message = fix_mod.Message()
        header = message.getHeader()
        header.setField(fix_mod.MsgType(FixMsgType.QUERY_REQUEST.value))
        header.setField(fix_mod.SenderSubID(self._credentials.username))

        message.setField(fix_mod.IntField(WindCustomTags.QueryType.value, query_type))
        query_id = f"QRY{query_type}{datetime.datetime.now(tz=datetime.UTC).strftime('%H%M%S')}"
        message.setField(fix_mod.ClOrdID(query_id))
        message.setField(fix_mod.Account(account_id))

        today = datetime.datetime.now(tz=datetime.UTC).strftime("%Y%m%d")
        message.setField(fix_mod.StringField(WindCustomTags.StartDate.value, today))
        message.setField(fix_mod.StringField(WindCustomTags.EndDate.value, today))

        if self.session_id is not None:
            fix_mod.Session.sendToTarget(message, self.session_id)

    def _query_positions(self, query_type: int, account_id: str) -> None:
        fix_mod = self._fix
        message = fix_mod.Message()
        header = message.getHeader()
        msg_type = (
            FixMsgType.HOLDING_QUERY_REQUEST.value
            if query_type == 3
            else FixMsgType.FUNDING_QUERY_REQUEST.value
        )
        header.setField(fix_mod.MsgType(msg_type))
        header.setField(fix_mod.SenderSubID(self._credentials.username))
        message.setField(fix_mod.Account(account_id))

        if self.session_id is not None:
            fix_mod.Session.sendToTarget(message, self.session_id)
