"""slugaudit-mcp requires POSIX (fcntl-based locking); prove that's enforced
with a clear error instead of a bare ImportError from deep inside fcntl."""

import unittest
from unittest.mock import patch

import mcp_server


class TestPlatformCheck(unittest.TestCase):
    def test_posix_passes_without_exiting(self) -> None:
        with patch.object(mcp_server.os, "name", "posix"):
            mcp_server._require_posix()  # must not raise

    def test_non_posix_exits_with_a_clear_message(self) -> None:
        with patch.object(mcp_server.os, "name", "nt"):
            with self.assertRaises(SystemExit) as ctx:
                mcp_server._require_posix()
        self.assertIn("Linux or macOS", str(ctx.exception))
        self.assertIn("fcntl", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
