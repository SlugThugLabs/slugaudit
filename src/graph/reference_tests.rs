use super::*;

fn text(language: &str, raw: &str) -> String {
    extract(language, raw).expect("must parse").text
}

#[test]
fn python_relative_and_absolute_forms() {
    assert_eq!(text("python", "from . import foo"), ".");
    assert_eq!(text("python", "from .bar import baz"), ".bar");
    assert_eq!(text("python", "from ..pkg.mod import qux"), "..pkg.mod");
    assert_eq!(text("python", "import os"), "os");
    assert_eq!(text("python", "import numpy as np"), "numpy");
    assert_eq!(
        text("python", "from collections import OrderedDict"),
        "collections"
    );
}

#[test]
fn javascript_and_typescript_quoted_paths() {
    assert_eq!(text("javascript", "import x from './utils';"), "./utils");
    assert_eq!(
        text("javascript", "import { y } from '../lib/helper';"),
        "../lib/helper"
    );
    assert_eq!(text("javascript", "import React from 'react';"), "react");
    assert_eq!(
        text("typescript", "import type { Y } from '../types';"),
        "../types"
    );
    assert_eq!(text("typescript", r#"import z from "lodash";"#), "lodash");
}

#[test]
fn rust_use_paths() {
    assert_eq!(text("rust", "use crate::baz::qux;"), "crate::baz::qux");
    assert_eq!(
        text("rust", "use std::collections::HashMap;"),
        "std::collections::HashMap"
    );
    assert_eq!(text("rust", "use super::thing;"), "super::thing");
    assert_eq!(text("rust", "use self::inner as renamed;"), "self::inner");
    assert_eq!(text("rust", "pub use crate::foo::Bar;"), "crate::foo::Bar");
}

#[test]
fn an_unsupported_language_returns_none() {
    assert_eq!(extract("go", "import \"fmt\""), None);
    assert_eq!(extract("c", "#include <stdio.h>"), None);
}

#[test]
fn malformed_text_returns_none_rather_than_panicking() {
    assert_eq!(extract("javascript", "import x from broken"), None);
    assert_eq!(extract("rust", ""), None);
    assert_eq!(extract("python", ""), None);
}
