use crate::graph::resolver::get_resolver;
use slugaudit_test_check::{Check, TestResult};

#[test]
fn python_relative_and_absolute_forms() -> TestResult {
    let resolver = get_resolver("python");
    assert_eq!(
        resolver
            .extract_reference("from . import foo")
            .check("reference should extract")?
            .text,
        "."
    );
    assert_eq!(
        resolver
            .extract_reference("from .bar import baz")
            .check("reference should extract")?
            .text,
        ".bar"
    );
    assert_eq!(
        resolver
            .extract_reference("from ..pkg.mod import qux")
            .check("reference should extract")?
            .text,
        "..pkg.mod"
    );
    assert_eq!(
        resolver
            .extract_reference("import os")
            .check("reference should extract")?
            .text,
        "os"
    );
    assert_eq!(
        resolver
            .extract_reference("import numpy as np")
            .check("reference should extract")?
            .text,
        "numpy"
    );
    assert_eq!(
        resolver
            .extract_reference("from collections import OrderedDict")
            .check("reference should extract")?
            .text,
        "collections"
    );
    Ok(())
}

#[test]
fn javascript_and_typescript_quoted_paths() -> TestResult {
    let resolver = get_resolver("javascript");
    assert_eq!(
        resolver
            .extract_reference("import x from './utils';")
            .check("reference should extract")?
            .text,
        "./utils"
    );
    assert_eq!(
        resolver
            .extract_reference("import { y } from '../lib/helper';")
            .check("reference should extract")?
            .text,
        "../lib/helper"
    );
    assert_eq!(
        resolver
            .extract_reference("import React from 'react';")
            .check("reference should extract")?
            .text,
        "react"
    );
    assert_eq!(
        resolver
            .extract_reference("import type { Y } from '../types';")
            .check("reference should extract")?
            .text,
        "../types"
    );
    assert_eq!(
        resolver
            .extract_reference(r#"import z from "lodash";"#)
            .check("reference should extract")?
            .text,
        "lodash"
    );
    Ok(())
}

#[test]
fn rust_use_paths() -> TestResult {
    let resolver = get_resolver("rust");
    assert_eq!(
        resolver
            .extract_reference("use crate::baz::qux;")
            .check("reference should extract")?
            .text,
        "crate::baz::qux"
    );
    assert_eq!(
        resolver
            .extract_reference("use std::collections::HashMap;")
            .check("reference should extract")?
            .text,
        "std::collections::HashMap"
    );
    assert_eq!(
        resolver
            .extract_reference("use super::thing;")
            .check("reference should extract")?
            .text,
        "super::thing"
    );
    assert_eq!(
        resolver
            .extract_reference("use self::inner as renamed;")
            .check("reference should extract")?
            .text,
        "self::inner"
    );
    assert_eq!(
        resolver
            .extract_reference("pub use crate::foo::Bar;")
            .check("reference should extract")?
            .text,
        "crate::foo::Bar"
    );
    Ok(())
}

#[test]
fn an_unsupported_language_returns_none() {
    let resolver = get_resolver("go");
    assert_eq!(resolver.extract_reference("import \"fmt\""), None);
    // Call-based import languages (lua/ruby/r) have neither import-shaped
    // nodes nor a keyword pattern in the generic resolver, so extraction
    // fails honestly instead of guessing at a module name.
    let resolver = get_resolver("lua");
    assert_eq!(resolver.extract_reference("require(\"x\")"), None);
}

#[test]
fn malformed_text_returns_none_rather_than_panicking() {
    let resolver = get_resolver("javascript");
    assert_eq!(resolver.extract_reference("import x from broken"), None);
    let resolver = get_resolver("rust");
    assert_eq!(resolver.extract_reference(""), None);
    let resolver = get_resolver("python");
    assert_eq!(resolver.extract_reference(""), None);
}
