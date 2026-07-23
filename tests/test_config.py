"""Tests for database configuration loading."""

import os
import unittest

from app.config import load_config


class TestConfig(unittest.TestCase):
    """Config loading from env vars and config file."""

    def setUp(self) -> None:
        # Save original env vars
        self._saved = {}
        for var in ("PGHOST", "PGPORT", "PGDATABASE", "PGUSER", "PGPASSWORD"):
            self._saved[var] = os.environ.get(var)

        # Prevent fallback to config.toml during tests
        self._saved["SLUGAUDIT_CONFIG"] = os.environ.get("SLUGAUDIT_CONFIG")
        if "SLUGAUDIT_CONFIG" in os.environ:
            del os.environ["SLUGAUDIT_CONFIG"]

        # Set test values
        os.environ["PGHOST"] = "test-host"
        os.environ["PGDATABASE"] = "test-db"
        os.environ["PGUSER"] = "test-user"
        os.environ["PGPASSWORD"] = "test-pass"
        os.environ["PGPORT"] = "6543"

        # Reset global cache and prevent config.toml fallback
        import app.config
        app.config._config = None
        app.config._find_config = lambda: None

    def tearDown(self) -> None:
        # Restore original env vars
        for var, val in self._saved.items():
            if val is not None:
                os.environ[var] = val
            else:
                os.environ.pop(var, None)

        # Reset cache
        import app.config
        app.config._config = None

    def test_loads_from_env_vars(self) -> None:
        cfg = load_config()
        self.assertEqual(cfg.host, "test-host")
        self.assertEqual(cfg.port, 6543)
        self.assertEqual(cfg.database, "test-db")
        self.assertEqual(cfg.user, "test-user")
        self.assertEqual(cfg.password, "test-pass")
        self.assertTrue(cfg.is_configured)

    def test_caches_config(self) -> None:
        cfg1 = load_config()
        cfg2 = load_config()
        self.assertIs(cfg1, cfg2)

    def test_not_configured_when_empty(self) -> None:
        # Clear all required fields (also ensure no config.toml fallback)
        for var in ("PGHOST", "PGDATABASE", "PGUSER"):
            os.environ.pop(var)
        import app.config
        app.config._config = None

        cfg = load_config()
        self.assertFalse(cfg.is_configured)

    def test_port_defaults_to_5432(self) -> None:
        if "PGPORT" in os.environ:
            del os.environ["PGPORT"]
        import app.config
        app.config._config = None

        cfg = load_config()
        self.assertEqual(cfg.port, 5432)

    def test_fields_default_to_empty_string(self) -> None:
        # Clear all env vars (also ensure no config.toml fallback)
        if "SLUGAUDIT_CONFIG" in os.environ:
            del os.environ["SLUGAUDIT_CONFIG"]
        for var in ("PGHOST", "PGDATABASE", "PGUSER", "PGPASSWORD"):
            os.environ.pop(var, None)
        import app.config
        app.config._config = None

        cfg = load_config()
        self.assertEqual(cfg.database, "")
        self.assertEqual(cfg.user, "")
        self.assertEqual(cfg.password, "")

    def test_env_var_override_takes_priority(self) -> None:
        self.assertEqual(load_config().port, 6543)


if __name__ == "__main__":
    unittest.main()
