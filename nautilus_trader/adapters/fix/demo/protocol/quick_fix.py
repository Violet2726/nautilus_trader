import datetime
import json
import logging
import time
import atexit
import socket
import ssl
import threading
from threading import Lock

import quickfix as fix

from protocol.hardware_info import get_hardware_info
from protocol.parse_msg_2json import FIXTreeParser
from protocol.wind_fix_enum import FixMsgType, WindCustomTags, Side, OrdType, SecurityType, BookingType, TimeInForce, \
    HandlInst, get_exchange_info_v3, StorageTopic, UserRequestType, UserStatus, QueryType

# 在程序启动的入口处配置
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s [%(levelname)s] %(message)s',
    datefmt='%Y-%m-%d %H:%M:%S'
)

_tls_tunnel_lock = threading.Lock()
_tls_tunnel_instance = None


class _TLSTunnel:
    def __init__(self, local_host, local_port, remote_host, remote_port):
        self.local_host = local_host
        self.local_port = local_port
        self.remote_host = remote_host
        self.remote_port = remote_port
        self._stop_event = threading.Event()
        self._server_sock = None
        self._thread = None

    def start(self):
        if self._thread is not None:
            return

        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.bind((self.local_host, self.local_port))
        server.listen(10)
        server.settimeout(1.0)
        self._server_sock = server

        logging.log(
            logging.INFO,
            f"TLS tunnel started: {self.local_host}:{self.local_port} -> {self.remote_host}:{self.remote_port}",
        )

        t = threading.Thread(target=self._serve_forever, name="tls-tunnel", daemon=True)
        self._thread = t
        t.start()

    def stop(self):
        self._stop_event.set()
        try:
            if self._server_sock is not None:
                self._server_sock.close()
        except Exception:
            pass

    def _serve_forever(self):
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE

        while not self._stop_event.is_set():
            try:
                client, _ = self._server_sock.accept()
            except socket.timeout:
                continue
            except OSError:
                break

            client.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

            try:
                upstream = socket.create_connection((self.remote_host, self.remote_port), timeout=10)
                try:
                    upstream.settimeout(None)
                except Exception:
                    pass
                upstream_tls = ctx.wrap_socket(upstream, server_hostname="wind.com.cn")
                try:
                    upstream_tls.settimeout(None)
                except Exception:
                    pass
                upstream_tls.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            except Exception:
                logging.log(
                    logging.ERROR,
                    f"TLS tunnel upstream connect failed: {self.remote_host}:{self.remote_port}",
                    exc_info=True,
                )
                try:
                    client.close()
                except Exception:
                    pass
                continue
            try:
                logging.log(
                    logging.INFO,
                    f"TLS tunnel connected: {upstream_tls.version()} {upstream_tls.cipher()}",
                )
            except Exception:
                pass

            threading.Thread(target=self._pipe, args=(client, upstream_tls, "client->server"), daemon=True).start()
            threading.Thread(target=self._pipe, args=(upstream_tls, client, "server->client"), daemon=True).start()

    @staticmethod
    def _pipe(src, dst, direction):
        first = True
        try:
            while True:
                try:
                    data = src.recv(4096)
                except socket.timeout:
                    continue
                if not data:
                    if first:
                        logging.log(logging.INFO, f"TLS tunnel {direction} closed_without_data")
                    else:
                        logging.log(logging.INFO, f"TLS tunnel {direction} closed")
                    break
                if first:
                    first = False
                    try:
                        preview = data[:200]
                        logging.log(logging.INFO, f"TLS tunnel {direction} first_bytes={len(data)} preview={preview!r}")
                    except Exception:
                        pass
                dst.sendall(data)
        except Exception as e:
            try:
                logging.log(logging.INFO, f"TLS tunnel {direction} error: {type(e).__name__}: {e}")
            except Exception:
                pass
        finally:
            try:
                src.shutdown(socket.SHUT_RDWR)
            except Exception:
                pass
            try:
                dst.shutdown(socket.SHUT_RDWR)
            except Exception:
                pass
            try:
                src.close()
            except Exception:
                pass
            try:
                dst.close()
            except Exception:
                pass


def ensure_tls_tunnel(remote_host, remote_port, local_host="127.0.0.1", local_port=16670):
    global _tls_tunnel_instance
    with _tls_tunnel_lock:
        if _tls_tunnel_instance is None:
            _tls_tunnel_instance = _TLSTunnel(
                local_host=local_host,
                local_port=local_port,
                remote_host=remote_host,
                remote_port=remote_port,
            )
            _tls_tunnel_instance.start()
            atexit.register(_tls_tunnel_instance.stop)
    return local_host, local_port


class QMTToFIXAdapter(fix.Application):
    def __init__(self, config_file, login_account, password):
        super().__init__()
        self.login_account = login_account
        self.password = password
        self.session_id = None

        # 用于存储本地成交/资金/委托状态的缓存，供查询接口使用
        self.trade_cache = {
            StorageTopic.ORDERS.value: {},  # 委托记录
            StorageTopic.DEALS.value: {},  # 成交记录
            StorageTopic.POSITIONS.value: {},  # 持仓信息
            StorageTopic.ACCOUNTS.value: {},  # 资金信息
            StorageTopic.EXEC_ORDER.value: {}  # 下单信息
        }
        self.lock = Lock()
        self.hardware_info = get_hardware_info()
        self.parser = FIXTreeParser(config_file)

    # --- QuickFIX 框架回调 ---
    def onCreate(self, session_id):
        logging.log(logging.INFO, f"传输层尝试登录..."
                                  f"session_id:{session_id}，账号:{self.login_account}")

    def onLogon(self, session_id):
        self.session_id = session_id
        logging.log(logging.INFO, f"传输层登录成功，session_id:{session_id}")
        # 应用层登录
        self._app_logon()

    # def _init_data(self):
    #     # 1. 自动同步今日委托
    #     self._query_deal_data(query_type=1, account_id=self.account_id)
    #     # 2. 自动同步今日成交
    #     self._query_deal_data(query_type=2, account_id=self.account_id)
    #     # 3. 同步当前持仓
    #     self._query_positions(query_type=3, account_id=self.account_id)
    #     # 3. 同步当前资金
    #     self._query_positions(query_type=4, account_id=self.account_id)

    def onLogout(self, session_id):
        logging.log(logging.INFO, f" onLogout...")

    def toApp(self, message, session_id):
        pass

    def fromAdmin(self, message, session_id):
        """解析服务器返回的行政报文，用于排查登录失败的具体原因"""
        msg_type = fix.MsgType()
        message.getHeader().getField(msg_type)
        
        logging.log(logging.INFO, f"收到管理报文: {msg_type.getValue()}")
        
        # 如果收到服务器的拒绝(Reject)或注销(Logout)报文，打印具体原因(Tag 58: Text)
        if msg_type.getValue() in [fix.MsgType_Reject, fix.MsgType_Logout]:
            if message.isSetField(58):
                text_field = fix.Text()
                message.getField(text_field)
                logging.log(logging.ERROR, f"服务器返回错误信息 [{msg_type.getValue()}]: {text_field.getValue()}")
            else:
                logging.log(logging.ERROR, f"服务器返回错误信息 [{msg_type.getValue()}]: {message.toString()}")
        elif msg_type.getValue() == fix.MsgType_Logon:
            logging.log(logging.INFO, "收到登录成功响应")

    def toAdmin(self, message, session_id):
        """在发送给服务器的行政报文（如 Logon）中注入必要的认证字段"""
        msg_type = fix.MsgType()
        message.getHeader().getField(msg_type)
        
        # 针对 Logon (35=A) 报文注入万得要求的认证字段
        if msg_type.getValue() == fix.MsgType_Logon:
            logging.log(logging.INFO, "正在发送 Logon 报文")
            try:
                begin_string = None
                try:
                    begin_string = session_id.getBeginString()
                except Exception:
                    pass
                if begin_string and str(begin_string).startswith("FIXT"):
                    if not message.isSetField(1137):
                        message.setField(fix.StringField(1137, "6"))
            except Exception:
                pass
            try:
                message.getHeader().setField(fix.SenderSubID(self.login_account))
                from_ip = self.hardware_info.get("from_ip")
                if from_ip:
                    message.getHeader().setField(fix.SenderLocationID(str(from_ip)))
            except Exception:
                pass
            try:
                if not message.isSetField(553):
                    message.setField(fix.Username(self.login_account))
                if not message.isSetField(554):
                    message.setField(fix.Password(self.password))
            except Exception:
                pass
            try:
                raw_data = json.dumps(self.hardware_info, ensure_ascii=False, separators=(",", ":"))
                raw_data_bytes = raw_data.encode("utf-8")
                message.setField(fix.RawDataLength(len(raw_data_bytes)))
                message.setField(fix.RawData(raw_data))
            except Exception:
                pass
            try:
                msg_str = message.toString()
                parts = []
                for p in msg_str.split("\x01"):
                    if p.startswith("554="):
                        parts.append("554=***")
                    elif p.startswith("96="):
                        parts.append("96=<omitted>")
                    else:
                        parts.append(p)
                masked = "\\x01".join(parts)
                logging.log(logging.INFO, f"Logon 报文: {masked}")
            except Exception:
                pass

    def fromApp(self, message, session_id):
        """核心：在这里处理服务器返回的订单状态和查询结果"""
        msg_type = fix.MsgType()
        message.getHeader().getField(msg_type)
        # logging.log(logging.INFO, f"rec msg_type ：{msg_type}")

        # 应用层认证成功, 开始初始化数据
        if msg_type.getValue() == FixMsgType.USER_RESPONSE.value:
            self._after_login_init(message)

            # Tag 8000: 查询类型 (1:委托, 2:成交)
        elif msg_type.getValue() == FixMsgType.QUERY_RESPONSE.value:
            storge_type = StorageTopic.ORDERS.value if int(message.getField(
                fix.IntField(WindCustomTags.QueryType.value).getField())) == 1 else StorageTopic.DEALS.value
            self._handle_execution_report(message, storge_type)
        # 持仓
        elif msg_type.getValue() == FixMsgType.HOLDING_QUERY_RESPONSE.value:
            self._handle_execution_report(message, StorageTopic.POSITIONS.value)

        # 资金
        elif msg_type.getValue() == FixMsgType.FUNDING_QUERY_RESPONSE.value:
            self._handle_execution_report(message, StorageTopic.ACCOUNTS.value)
        # 成交回报
        elif msg_type.getValue() == FixMsgType.EXECUTION_REPORT.value:
            self._handle_execution_order(message, StorageTopic.EXEC_ORDER.value)
        # 业务消息被拒绝
        elif msg_type.getValue() == FixMsgType.BUSINESS_MESSAGE_REJECT.value:
            business_reject_ref_id = fix.BusinessRejectRefID()
            text = fix.Text()
            message.getField(business_reject_ref_id)
            message.getField(text)
            logging.log(logging.ERROR, f"业务消息被拒绝,订单编号{business_reject_ref_id},{text}")

    def _after_login_init(self, message):
        # 1. 创建字段容器对象
        user_req_type_obj = fix.UserRequestType()
        user_status_obj = fix.UserStatus()
        message.getField(user_req_type_obj)
        message.getField(user_status_obj)
        # 获取实际的 Value 进行比较 (注意：QuickFIX 通常返回字符串)
        req_val = user_req_type_obj.getValue()
        status_val = user_status_obj.getValue()
        # 4. 这里的比较要确保类型一致（建议两边都转为 str 比较最稳妥）
        if (str(req_val) == str(UserRequestType.LOGIN.value)
                and str(status_val) == str(UserStatus.LOGGED_IN.value)):
            logging.log(logging.INFO, "应用层登录认证成功，fix协议可以正常发送...")
            # self._init_data()
        else:
            raise RuntimeError(f"登录认证未通过,状态码: {status_val}")

    def _app_logon(self):
        logging.log(logging.INFO, f"传输层登录成功，"
                                  f"session_id:{self.session_id}，账号:{self.login_account}，"
                                  f"开始发起应用层认证 (BE)...")

        try:
            # 创建 UserRequest<BE> 消息
            message = fix.Message()
            header = message.getHeader()
            header.setField(fix.MsgType(FixMsgType.USER_REQUEST.value))  # 消息类型 BE
            header.setField(fix.SenderSubID(self.login_account))

            # 填充图片 4.4.1 要求的内容
            user_req_id = f"CXL{datetime.datetime.now().strftime('%H%M%S%f')}"
            message.setField(fix.UserRequestID(user_req_id))  # UserRequestID
            message.setField(fix.UserRequestType(UserRequestType.LOGIN.value))  # UserRequestType: 1-登录
            message.setField(fix.Username(self.login_account))  # Username
            message.setField(fix.Password(self.password))  # Password
            # 填充硬件信息 RawData (Tag 96)
            raw_data = json.dumps(self.hardware_info)
            message.setField(fix.RawDataLength(len(raw_data.encode("utf-8"))))  # RawDataLength
            message.setField(fix.RawData(raw_data))  # RawData

            if self.session_id is not None:
                fix.Session.sendToTarget(message, self.session_id)
                logging.log(logging.INFO, f"应用层登录请求已发送，请求ID: {user_req_id}")
            else:
                logging.log(logging.ERROR, "应用层登录失败：传输层会话 SessionID 为空")
        except Exception as e:
            logging.log(logging.ERROR, f"应用层登录失败，{e}")
            print(f"应用层登录失败: {e}")

    def _query_deal_data(self, query_type, account_id):
        """
        构造中的 4.8.1 QueryRequest<U02>
        """
        message = fix.Message()
        header = message.getHeader()
        header.setField(fix.MsgType(FixMsgType.QUERY_REQUEST.value))  # 自定义查询消息类型
        header.setField(fix.SenderSubID(self.login_account))

        # Tag 8000: 查询类型 (1:委托, 2:成交)
        message.setField(fix.IntField(WindCustomTags.QueryType.value, query_type))

        # Tag 11: 唯一查询编号
        query_id = f"QRY{query_type}{datetime.datetime.now().strftime('%H%M%S')}"
        message.setField(fix.ClOrdID(query_id))

        # Tag 1: 账户 ID (根据实际情况填充)
        message.setField(fix.Account(account_id))

        # Tag 916/917: 开始/结束日期 (YYYYMMDD)
        today = datetime.datetime.now().strftime("%Y%m%d")
        message.setField(fix.StringField(WindCustomTags.StartDate.value, today))
        message.setField(fix.StringField(WindCustomTags.EndDate.value, today))

        try:
            if self.session_id is not None:
                fix.Session.sendToTarget(message, self.session_id)
            else:
                 logging.log(logging.ERROR, "查询失败：SessionID 为空，请检查连接是否建立")
        except Exception as e:
            logging.log(logging.ERROR, f"Query QueryRequest<U02> Error: {e}")
        # logging.log(logging.INFO, f"Sent QueryRequest<U02> "
        #                           f"{'委托' if QueryType.ORDER.value == int(query_type) else '成交'}"
        #                           f" Type={query_type}")

    def _query_positions(self, query_type, account_id):
        """
        发送 HoldingQueryRequest<U04>
        用于获取当前账户的全量持仓快照
        发送 FundingQueryRequest<U06>
        用于获取当前账户的资金查询
        """
        message = fix.Message()
        header = message.getHeader()
        # 根据你的协议字典，MsgType 可能是 U04 U06
        msg_type = FixMsgType.HOLDING_QUERY_REQUEST.value if query_type == 3 else FixMsgType.FUNDING_QUERY_REQUEST.value
        header.setField(fix.MsgType(msg_type))
        header.setField(fix.SenderSubID(self.login_account))
        # Tag 1: 账户 ID (根据实际情况填充)
        message.setField(fix.Account(account_id))

        try:
            if self.session_id is not None:
                fix.Session.sendToTarget(message, self.session_id)
            else:
                logging.log(logging.ERROR, "查询失败：SessionID 为空，请检查连接是否建立")
        except Exception as e:
            logging.log(logging.ERROR, f"Query Positions/Account Error: {e}")
        # logging.log(logging.INFO, f"Sent Request {'持仓' if query_type == 3 else '资金'} (35={msg_type})")

    def _handle_execution_report(self, message, storge_type):
        """解析执行报告，更新本地缓存"""
        account_id = message.getField(fix.Account().getField())
        with self.lock:
            self.trade_cache[storge_type][account_id] = self._message_to_dict(message)

    def _handle_execution_order(self, message, storge_type):
        """解析单据，更新本地缓存"""
        cl_order_id = message.getField(fix.ClOrdID().getField())
        with self.lock:
            self.trade_cache[storge_type][cl_order_id] = self._message_to_dict(message)

    # --- 接口 3: passorder (综合下单) ---
    def passorder(self, opt_ype, account_id, order_code, pr_type, volume, user_order_id,
                  price=None, order_type=None, strategy_name=None, quick_trade=None, ):
        """
        映射 QMT 的 passorder 参数到 FIX NewOrderSingle\n
        - opType: 23-买入, 24-卖出\n
        - prType: 11-限价, 14-市价\n
        QMT 字段参考含义\n
        - 23 #opt_ype 操作号\n
        - 1101 #order_type 组合方式 -- 无效\n
        - '1000044' #accountid 资金账号\n
        - 'cu2403.SF' #order_code 品种代码\n
        - 11 #pr_type 报价类型\n
        - 0.0 #price 价格\n
        - 2 #volume 下单量\n
        - '示例下单' #strategyName 策略名称  -- 无效\n
        - 1 #quickTrade 快速下单标记  -- 无效\n
        - '投资备注' #user_order_id 投资备注\n
        """
        # 1. 调用修正后的映射逻辑
        # 注意：这里会自动根据 orderCode 后缀处理 SSE, SZSE, SHN, SZN, HKEX 等
        exchange, sec_type = get_exchange_info_v3(order_code)
        message = fix.Message()
        header = message.getHeader()
        # Tag 35=D (NewOrderSingle)
        header.setField(fix.MsgType(fix.MsgType_NewOrderSingle))
        header.setField(fix.SenderSubID(self.login_account))
        # Tag 11: 订单编号 (保证当天唯一)
        cl_ord_id = f"ORD{datetime.datetime.now().strftime('%H%M%S%f')}"
        message.setField(fix.ClOrdID(cl_ord_id))
        # Tag 21: HandlInst (必须设置)
        # 1: LowTouch(LT) 自动处理, 3: HighTouch(HT) 人工处理
        message.setField(fix.HandlInst(HandlInst.LowTouch.value))
        # Tag 54: Side (1:Buy, 2:Sell, 5:Sell short)
        side = Side.BUY.value if int(opt_ype) == 23 else Side.SELL.value
        message.setField(fix.Side(side))
        # Tag 60: TransactTime (格式必须为 YYYYMMDD-HH:MM:SS)
        utc_now = datetime.datetime.utcnow().strftime("%Y%m%d-%H:%M:%S")
        # 使用通用 Field 赋值方法，强制把 Tag 60 设置为字符串
        message.setField(fix.StringField(60, utc_now))
        # Tag 38: OrderQty (委托数量)
        message.setField(fix.OrderQty(float(volume)))
        # Tag 40: OrdType (1:市价单, 2:限价单)
        ord_type = OrdType.LIMIT.value if int(pr_type) == 11 else OrdType.MARKET.value
        message.setField(fix.OrdType(ord_type))
        # Tag 44: Price (限价单必填)
        if ord_type == OrdType.LIMIT.value:
            if price is None:
                raise ValueError("Price is required for limit orders")
            message.setField(fix.Price(float(price)))
        # Tag 55: Symbol (Wind 代码类型)
        message.setField(fix.Symbol(order_code))
        # Tag 207: SecurityExchange (Wind 交易所类型)
        message.setField(fix.SecurityExchange(exchange))
        # Tag 59: TimeInForce (0:GFD 当日有效 1:GTC 取消前有效)
        message.setField(fix.TimeInForce(TimeInForce.GFD.value))
        # Tag 167: SecurityType CS:股票  FUT:期货  OPT:期权
        message.setField(fix.SecurityType(SecurityType.COMMON_STOCK.value))
        # Tag 775: BookingType 0:cash(现金交易) 1:swap(收益互换)
        message.setField(fix.BookingType(BookingType.CASH.value))
        # Tag 1: Account (账号 ID)
        message.setField(fix.Account(str(account_id)))
        # Tag 58: Text (备注)
        if user_order_id is not None and user_order_id != "":
            message.setField(fix.Text(str(user_order_id)))

        # 10. 发送消息
        try:
            if self.session_id is not None:
                fix.Session.sendToTarget(message, self.session_id)
            else:
                logging.log(logging.ERROR, "下单失败：SessionID 为空，请检查连接是否建立")
        except Exception as e:
            logging.log(logging.ERROR, f"Send passorder Error: {e}")
        # logging.log(logging.INFO, f"sent passorder: ClOrdID {cl_ord_id}")
        result: str = self._get_value_waite_time(StorageTopic.EXEC_ORDER.value, cl_ord_id)
        # OrderID 获取 TAG 37
        if result is not None and result != {}:
            result = json.loads(result).get('OrderID')
            # logging.log(logging.INFO, f"Receive passorder response: ClOrdID {cl_ord_id} match OrderID {result}")
        return result

    # --- 接口 4: cancel (撤单) ---
    def cancel(self, order_id, account_id, side):
        """
        映射 QMT 的撤单请求
        - orderId 委托号 \n
        - accountId 资金账号 \n
        - side 交易方向 1:Buy  2:Sell  5:Seel short \n
        """
        message = fix.Message()
        header = message.getHeader()
        # 对应文档 4.5.2: MsgType=F
        header.setField(fix.MsgType(fix.MsgType_OrderCancelRequest))
        header.setField(fix.SenderSubID(self.login_account))
        # Tag 11: 撤单请求编号，保证当天唯一
        cancel_req_id = f"CXL{datetime.datetime.now().strftime('%H%M%S%f')}"
        message.setField(fix.ClOrdID(cancel_req_id))
        # Tag 41: 填写柜台合同号 (对应文档描述中的 OrderID 字段)
        message.setField(fix.OrigClOrdID(str(order_id)))
        # Tag 54: 交易方向
        message.setField(fix.Side(str(side)))
        # Tag 60: 订单时间 (格式: YYYYMMDD-HH:MM:SS)
        # 注意：FIX 默认通常要求 UTC 时间
        utc_now = datetime.datetime.utcnow().strftime("%Y%m%d-%H:%M:%S")
        message.setField(fix.StringField(60, utc_now))
        # Tag 1: 账号 ID
        message.setField(fix.Account(str(account_id)))
        # 9. 发送撤单请求
        try:
            if self.session_id is not None:
                fix.Session.sendToTarget(message, self.session_id)
            else:
                logging.log(logging.ERROR, "撤单失败：SessionID 为空，请检查连接是否建立")
        except Exception as e:
            logging.log(logging.ERROR, f"Send cancel Error: {e}")
        # logging.log(logging.INFO, f"sent cancel: ClOrdID {cancel_req_id}")
        result: str = self._get_value_waite_time(StorageTopic.EXEC_ORDER.value, cancel_req_id)
        # OrderID 获取 TAG 37
        if result is not None and result != {}:
            result = json.loads(result).get('OrderID')
            # logging.log(logging.INFO, f"Receive cancel response: ClOrdID {cancel_req_id} match OrderID {result}")
        return result

    # --- 接口 5: get_trade_detail_data (查询) ---
    def get_trade_detail_data(self, account_id, str_data_type, str_account_type=None):
        """
        从本地缓存中读取数据\n
        - account_id 资金账号\n
        - str_account_type 账号类型 -- 无效\n
            - 'STOCK'：股票\n
        - strDatatype 数据类型\n
            - ACCOUNT：账号对象或信用账号对象 -- 资金\n
            - POSITION：持仓 --\n
            - ORDER：委托 --\n
            - DEAL ：成交 --\n
        """
        result = {}
        if str_data_type == 'ACCOUNT':
            self.trade_cache[StorageTopic.ACCOUNTS.value][account_id] = {}
            self._query_positions(query_type=4, account_id=account_id)
            #  轮询直到获取值或超时
            result = self._get_value_waite_time(StorageTopic.ACCOUNTS.value, account_id)
        if str_data_type == 'POSITION':
            self.trade_cache[StorageTopic.POSITIONS.value][account_id] = {}
            self._query_positions(query_type=3, account_id=account_id)
            #  轮询直到获取值或超时
            result = self._get_value_waite_time(StorageTopic.POSITIONS.value, account_id)
        elif str_data_type == 'ORDER':
            self.trade_cache[StorageTopic.ORDERS.value][account_id] = {}
            self._query_deal_data(query_type=1, account_id=account_id)
            #  轮询直到获取值或超时
            result = self._get_value_waite_time(StorageTopic.ORDERS.value, account_id)
        elif str_data_type == 'DEAL':
            self.trade_cache[StorageTopic.DEALS.value][account_id] = {}
            self._query_deal_data(query_type=2, account_id=account_id)
            #  轮询直到获取值或超时
            result = self._get_value_waite_time(StorageTopic.DEALS.value, account_id)
        return result

    def _get_value_waite_time(self, cache_key, sub_key, timeout=30):
        """等待缓存中指定键的值被设置"""
        start_time = time.time()
        while time.time() - start_time < timeout:
            # 在循环中检查值，注意锁的范围要小，避免阻塞其他线程写入
            with self.lock:
                result = self.trade_cache.get(cache_key).get(sub_key)
                # 如果值不再是 None 且不是空字典，说明数据已返回
                if result != {} and result is not None:
                    return result
            time.sleep(0.5)  # 缩短步长，提高响应灵敏度
        logging.log(logging.ERROR, f"查询超时: 一级key:{cache_key},二级key:{sub_key}")
        return {}  # 或者根据业务需求返回 None

    def _message_to_dict(self, message):
        tree_json = self.parser.parse(message)
        return json.dumps(tree_json, indent=4)
