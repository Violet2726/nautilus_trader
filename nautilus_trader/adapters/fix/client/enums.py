from __future__ import annotations

from enum import Enum


class FixMsgType(Enum):
    HEARTBEAT = "0"
    LOGON = "A"
    LOGOUT = "5"
    NEW_ORDER_SINGLE = "D"
    ORDER_CANCEL_REQUEST = "F"
    EXECUTION_REPORT = "8"
    BUSINESS_MESSAGE_REJECT = "j"
    USER_REQUEST = "BE"
    USER_RESPONSE = "BF"
    QUERY_REQUEST = "U02"
    QUERY_RESPONSE = "U03"
    HOLDING_QUERY_REQUEST = "U04"
    HOLDING_QUERY_RESPONSE = "U05"
    FUNDING_QUERY_REQUEST = "U06"
    FUNDING_QUERY_RESPONSE = "U07"


class UserRequestType(Enum):
    LOGIN = 1
    LOGOUT = 2


class QueryType(Enum):
    ORDER = 1
    EXECUTION = 2


class StorageTopic(Enum):
    ORDERS = "orders"
    DEALS = "deals"
    POSITIONS = "positions"
    ACCOUNTS = "accounts"
    EXEC_ORDER = "execOrder"


class OrdType(Enum):
    LIMIT = "2"
    MARKET = "1"


class Side(Enum):
    BUY = "1"
    SELL = "2"
    SELL_SHORT = "5"


class OrdStatus(Enum):
    NEW = "0"
    PARTIALLY_FILLED = "1"
    FILLED = "2"
    DONE_FOR_DAY = "3"
    CANCELED = "4"
    PENDING_CANCEL = "6"
    STOPPED = "7"
    REJECTED = "8"
    EXPIRED = "C"
    PENDING_REPLACE = "E"


class ExecType(Enum):
    NEW = "0"
    DONE_FOR_DAY = "3"
    CANCELED = "4"
    REPLACED = "5"
    PENDING_CANCEL = "6"
    STOPPED = "7"
    REJECTED = "8"
    SUSPENDED = "9"
    PENDING_NEW = "A"
    EXPIRED = "C"
    PENDING_REPLACE = "E"
    ORDER_STATUS = "I"
    TRADE = "F"


class BookingType(Enum):
    CASH = 0
    SWAP = 1


class TimeInForce(Enum):
    GFD = "0"
    GTC = "1"
    GTD = "5"


class HandlInst(Enum):
    LowTouch = "1"
    HighTouch = "3"


class SecurityType(Enum):
    COMMON_STOCK = "CS"
    FUTURE = "FUT"
    OPTION = "OPT"
    CASH_BOUND = "CASHBOUND"
    PLEDGED_REPO = "PLEDGEDREPO"


class UserStatus(Enum):
    LOGGED_IN = "1"
    NOT_LOGGED_IN = "2"
    USER_NOT_EXIST = "3"
    AUTH_FAILED = "4"
    CREDENTIAL_CHANGE_REQUIRED = "5"
    OTHER_ERROR = "6"


class WindCustomTags(Enum):
    QueryType = 8000
    StartDate = 916
    EndDate = 917
