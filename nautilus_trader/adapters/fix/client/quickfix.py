from __future__ import annotations


try:
    import quickfix as fix
except ModuleNotFoundError:
    fix = None
