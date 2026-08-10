"""Core package module for the multilang fixture."""

import os

from .helpers import helper_value
from .circular_a import a_value


def core_value() -> int:
    return helper_value() + a_value() + len(os.environ)
