"""slugaudit-mcp server package.

Cleanly separated modules for config, state, connection pooling,
auto-sync, tool definitions, handlers, and server.
"""

from app.config import load_config, Config
from app.state import ProjectState, load_state, save_state
from app.pool import get_connection, release_connection, init_pool
from app.sync import ensure_synced
from app.tools import TOOLS, validate_paths, validate_pattern, validate_sql_query
from app.handlers import HANDLERS
from app.server import run_server

__all__ = [
    "Config",
    "load_config",
    "ProjectState",
    "load_state",
    "save_state",
    "get_connection",
    "release_connection",
    "init_pool",
    "ensure_synced",
    "TOOLS",
    "validate_paths",
    "validate_pattern",
    "validate_sql_query",
    "HANDLERS",
    "run_server",
]
