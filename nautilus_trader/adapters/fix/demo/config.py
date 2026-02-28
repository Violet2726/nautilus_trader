# -*- coding: utf-8 -*-
"""
统一配置文件 - 支持多账号配置
"""


class AccountConfig:
    """单个账号配置类"""
    
    def __init__(self, wind_account, password, trading_account, group, bind_ip):
        self.WIND_LOGIN_ACCOUNT = wind_account  # Wind 通道登录账号
        self.WIND_PASSWORD = password           # Wind 通道密码
        self.TRADING_ACCOUNT = trading_account  # 交易账号
        self.ACCOUNT_GROUP = group              # 账号组别
        self.BIND_IP = bind_ip                  # 账号绑定的公网 IP
    
    def __repr__(self):
        return f"AccountConfig(group={self.ACCOUNT_GROUP}, trading_account={self.TRADING_ACCOUNT})"


class ServerConfig:
    """服务器配置类（所有账号共享）"""
    HOST = "114.80.213.49"          # FIX 服务器地址
    PORT = 16669                    # FIX 服务器端口


class PathsConfig:
    """配置文件路径类（所有账号共享）"""
    WIND_FIX_CONFIG = "wind_fix_config.cfg"  # Wind FIX 协议配置文件
    WIND_FIX_DICT = "FIX44.xml"              # Wind FIX 协议字典


class TimeoutConfig:
    """超时配置类（所有账号共享）"""
    LOGIN_WAIT = 3                       # 登录等待时间（秒）
    CONNECTION_TIMEOUT = 5               # 网络连接超时（秒）
    MAX_LOGIN_WAIT = 30                  # 最大登录等待时间（秒）


class Config:
    """配置管理类 - 提供多账号支持"""
    
    # ============ 服务器配置（共享） ============
    Server = ServerConfig()
    Paths = PathsConfig()
    Timeout = TimeoutConfig()
    
    # ============ 账号配置 ============
    # 账号 A 组
    ACCOUNT_A = AccountConfig(
        wind_account="HA2032139003",
        password="60374602",
        trading_account="690",
        group="A",
        bind_ip="124.160.32.18"
    )
    
    # 账号 B 组
    ACCOUNT_B = AccountConfig(
        wind_account="HA2032139004",
        password="23339198",
        trading_account="693",
        group="B",
        bind_ip="223.6.252.104"
    )
    
    # 默认账号
    Account = ACCOUNT_B
    
    # ============ 账号映射表 ============
    ACCOUNTS = {
        "A": ACCOUNT_A,
        "B": ACCOUNT_B,
        "690": ACCOUNT_A,
        "693": ACCOUNT_B,
    }
    
    @classmethod
    def get_account(cls, group_or_account):
        """
        根据组别或交易账号获取账号配置
        
        Args:
            group_or_account: 组别（如"A"、"B"）或交易账号（如"690"、"693"）
        
        Returns:
            AccountConfig 对象
        
        Example:
            config = Config.get_account("B")
            config = Config.get_account("693")
        """
        return cls.ACCOUNTS.get(str(group_or_account), cls.ACCOUNT_B)
    
    @classmethod
    def use_account(cls, group_or_account):
        """
        切换当前使用的账号
        
        Args:
            group_or_account: 组别或交易账号
        
        Example:
            Config.use_account("A")
            Config.use_account("690")
        """
        cls.Account = cls.get_account(group_or_account)
        return cls.Account
    
    @classmethod
    def list_accounts(cls):
        """列出所有可用账号"""
        print("可用账号列表:")
        print("-" * 60)
        for key, acc in cls.ACCOUNTS.items():
            if key in ["A", "B"]:  # 只打印组别，避免重复
                print(f"组别：{acc.ACCOUNT_GROUP}")
                print(f"  交易账号：{acc.TRADING_ACCOUNT}")
                print(f"  Wind 账号：{acc.WIND_LOGIN_ACCOUNT}")
                print(f"  绑定 IP: {acc.BIND_IP}")
                print("-" * 60)


# 便捷访问函数
def get_account(group_or_account):
    """获取指定账号配置"""
    return Config.get_account(group_or_account)


def use_account(group_or_account):
    """切换当前账号"""
    return Config.use_account(group_or_account)


def list_accounts():
    """列出所有账号"""
    Config.list_accounts()
