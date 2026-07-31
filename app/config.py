"""Database configuration from config.toml and environment variables.

Priority (lowest to highest):
  1. /opt/slugaudit-mcp/config.toml
  2. $SLUGAUDIT_CONFIG (custom config path)
  3. Environment variables (PGHOST, PGPORT, PGDATABASE, PGUSER, PGPASSWORD)
"""

import logging
import os
import stat
import tomllib
from dataclasses import dataclass

logger = logging.getLogger("slugaudit-mcp.config")

# Bits that must be clear: config.toml holds a plaintext DB password, so it
# must not be readable or writable by anyone other than its owner.
_INSECURE_MODE_BITS = stat.S_IRWXG | stat.S_IRWXO


@dataclass
class Config:
    """Database connection configuration."""
    host: str = "localhost"
    port: int = 5432
    database: str = ""
    user: str = ""
    password: str = ""
    pool_min: int = 1
    pool_max: int = 5

    @property
    def is_configured(self) -> bool:
        return bool(self.host and self.database and self.user)


_config: Config | None = None


def _check_config_file_permissions(path: str) -> None:
    """Refuse a config file that anyone but its owner can read or write.

    config.toml stores a plaintext database password. A world- or
    group-readable copy turns a credentials leak into "any other local
    account reads one file" — fail loudly instead of loading it silently.
    """
    mode = stat.S_IMODE(os.stat(path).st_mode)
    if mode & _INSECURE_MODE_BITS:
        raise RuntimeError(
            f"Refusing to read {path}: it is readable or writable by group "
            f"or other (mode {oct(mode)}). It stores a plaintext database "
            f"password. Fix with: chmod 600 {path}"
        )


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
            _check_config_file_permissions(config_path)
            with open(config_path, "rb") as f:
                toml = tomllib.load(f)
            db = toml.get("database", {})
            cfg.host = cfg.host or db.get("host", "localhost")
            if not os.environ.get("PGPORT"):
                cfg.port = int(db.get("port", 5432))
            cfg.database = cfg.database or db.get("database", "")
            cfg.user = cfg.user or db.get("user", "")
            cfg.password = cfg.password or db.get("password", "")
            cfg.pool_min = int(db.get("pool_min", cfg.pool_min))
            cfg.pool_max = int(db.get("pool_max", cfg.pool_max))

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
