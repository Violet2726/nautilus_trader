from xtquant import xttrader
from xtquant.xttype import StockAccount

from nautilus_trader.adapters.thinktrader.client.common import BaseMixin
from nautilus_trader.common.enums import LogColor


class ThinkTraderClientConnectionMixin(BaseMixin):
    """管理与 MiniQmt 的连接"""
    
    async def _connect(self) -> None:
        """建立连接"""
        print(f"DEBUG: Starting connection to MiniQmt at {self._miniqmt_path}")
        self._log.info(f"正在连接到 MiniQmt: {self._miniqmt_path}")
        
        print("DEBUG: Initializing XtQuantTrader...")
        self._trader = xttrader.XtQuantTrader(
            self._miniqmt_path,
            self._session_id,
        )
        self._trader.register_callback(self._callback)
        
        print("DEBUG: Starting XtQuantTrader...")
        self._trader.start()
        
        print("DEBUG: Calling trader.connect()...")
        connect_result = self._trader.connect()
        print(f"DEBUG: trader.connect() result: {connect_result}")
        if connect_result != 0:
            raise ConnectionError(f"连接 MiniQmt 失败，错误码: {connect_result}")
        
        print(f"DEBUG: Subscribing to account: {self._account_id}")
        self._account = StockAccount(self._account_id)
        subscribe_result = self._trader.subscribe(self._account)
        print(f"DEBUG: trader.subscribe() result: {subscribe_result}")
        if subscribe_result != 0:
            raise ConnectionError(f"订阅账户失败，错误码: {subscribe_result}")
        
        self._is_connected.set()
        print("DEBUG: Connection successful, Event set.")
        self._log.info(
            f"已连接到 MiniQmt，账户: {self._account_id}",
            LogColor.GREEN,
        )
    
    async def _disconnect(self) -> None:
        """断开连接"""
        if self._trader:
            self._trader.unsubscribe(self._account)
            self._trader.stop()
        self._is_connected.clear()
        self._log.info("已断开与 MiniQmt 的连接")
    
    def _check_connection(self) -> bool:
        """检查连接状态"""
        return self._is_connected.is_set()
