use crate::graph::resolver::get_resolver;

#[test]
fn python_relative_and_absolute_forms() {
    let resolver = get_resolver("python");
    assert_eq!(
        resolver
            .extract_reference("from . import foo")
            .expect("reference should extract")
            .text,
        "."
    );
    assert_eq!(
        resolver
            .extract_reference("from .bar import baz")
            .expect("reference should extract")
            .text,
        ".bar"
    );
    assert_eq!(
        resolver
            .extract_reference("from ..pkg.mod import qux")
            .expect("reference should extract")
            .text,
        "..pkg.mod"
    );
    assert_eq!(
        resolver
            .extract_reference("import os")
            .expect("reference should extract")
            .text,
        "os"
    );
    assert_eq!(
        resolver
            .extract_reference("import numpy as np")
            .expect("reference should extract")
            .text,
        "numpy"
    );
    assert_eq!(
        resolver
            .extract_reference("from collections import OrderedDict")
            .expect("reference should extract")
            .text,
        "collections"
    );
}

#[test]
fn javascript_and_typescript_quoted_paths() {
    let resolver = get_resolver("javascript");
    assert_eq!(
        resolver
            .extract_reference("import x from './utils';")
            .expect("reference should extract")
            .text,
        "./utils"
    );
    assert_eq!(
        resolver
            .extract_reference("import { y } from '../lib/helper';")
            .expect("reference should extract")
            .text,
        "../lib/helper"
    );
    assert_eq!(
        resolver
            .extract_reference("import React from 'react';")
            .expect("reference should extract")
            .text,
        "react"
    );
    assert_eq!(
        resolver
            .extract_reference("import type { Y } from '../types';")
            .expect("reference should extract")
            .text,
        "../types"
    );
    assert_eq!(
        resolver
            .extract_reference(r#"import z from "lodash";"#)
            .expect("reference should extract")
            .text,
        "lodash"
    );
}

#[test]
fn rust_use_paths() {
    let resolver = get_resolver("rust");
    assert_eq!(
        resolver
            .extract_reference("use crate::baz::qux;")
            .expect("reference should extract")
            .text,
        "crate::baz::qux"
    );
    assert_eq!(
        resolver
            .extract_reference("use std::collections::HashMap;")
            .expect("reference should extract")
            .text,
        "std::collections::HashMap"
    );
    assert_eq!(
        resolver
            .extract_reference("use super::thing;")
            .expect("reference should extract")
            .text,
        "super::thing"
    );
    assert_eq!(
        resolver
            .extract_reference("use self::inner as renamed;")
            .expect("reference should extract")
            .text,
        "self::inner"
    );
    assert_eq!(
        resolver
            .extract_reference("pub use crate::foo::Bar;")
            .expect("reference should extract")
            .text,
        "crate::foo::Bar"
    );
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
