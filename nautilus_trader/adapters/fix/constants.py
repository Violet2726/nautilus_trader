from typing import Final

from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import Venue

FIX: Final[str] = "FIX"
FIX_VENUE: Final[Venue] = Venue(FIX)
FIX_CLIENT_ID: Final[ClientId] = ClientId(FIX)

