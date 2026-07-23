"""Database configuration from config.toml and environment variables.

Priority (lowest to highest):
  1. /opt/slugaudit-mcp/config.toml
  2. $SLUGAUDIT_CONFIG (custom config path)
  3. Environment variables (PGHOST, PGPORT, PGDATABASE, PGUSER, PGPASSWORD)
"""

import logging
import os
import tomllib
from dataclasses import dataclass

logger = logging.getLogger("slugaudit-mcp.config")


@dataclass
class Config:
    """Database connection configuration."""
    host: str = "localhost"
    port: int = 5432
    database: str = ""
    user: str = ""
    password: str = ""

    @property
    def is_configured(self) -> bool:
        return bool(self.host and self.database and self.user)


_config: Config | None = None


def load_config() -> Config:
    """Load database config once and cache it."""
    global _config
    if _config is not None:
        return _config

    cfg = Config(
        host=os.environ.get("PGHOST", ""),
        port=int(os.environ.get("PGPORT", "5432")),
        database=os.environ.get("PGDATABASE", ""),
        user=os.environ.get("PGUSER", ""),
        password=os.environ.get("PGPASSWORD", ""),
    )

    # If any required field is missing from env vars, try config file
    if not cfg.host or not cfg.database or not cfg.user:
        config_path = _find_config()
        if config_path:
            with open(config_path, "rb") as f:
                toml = tomllib.load(f)
            db = toml.get("database", {})
            cfg.host = cfg.host or db.get("host", "localhost")
            if not os.environ.get("PGPORT"):
                cfg.port = int(db.get("port", 5432))
            cfg.database = cfg.database or db.get("database", "")
            cfg.user = cfg.user or db.get("user", "")
            cfg.password = cfg.password or db.get("password", "")

    _config = cfg
    return cfg


def _find_config() -> str | None:
    """Find config.toml file."""
    env_path = os.environ.get("SLUGAUDIT_CONFIG")
    if env_path and os.path.exists(env_path):
        return env_path

    default = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "config.toml")
    if os.path.exists(default):
        return default

    return None
