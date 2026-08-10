use crate::graph::resolver::get_resolver;

#[test]
fn python_relative_and_absolute_forms() {
    let resolver = get_resolver("python");
    assert_eq!(resolver.extract_reference("from . import foo").unwrap().text, ".");
    assert_eq!(
        resolver.extract_reference("from .bar import baz").unwrap().text,
        ".bar"
    );
    assert_eq!(
        resolver.extract_reference("from ..pkg.mod import qux").unwrap().text,
        "..pkg.mod"
    );
    assert_eq!(
        resolver.extract_reference("import os").unwrap().text,
        "os"
    );
    assert_eq!(
        resolver.extract_reference("import numpy as np").unwrap().text,
        "numpy"
    );
    assert_eq!(
        resolver
            .extract_reference("from collections import OrderedDict")
            .unwrap()
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
            .unwrap()
            .text,
        "./utils"
    );
    assert_eq!(
        resolver
            .extract_reference("import { y } from '../lib/helper';")
            .unwrap()
            .text,
        "../lib/helper"
    );
    assert_eq!(
        resolver.extract_reference("import React from 'react';").unwrap().text,
        "react"
    );
    assert_eq!(
        resolver
            .extract_reference("import type { Y } from '../types';")
            .unwrap()
            .text,
        "../types"
    );
    assert_eq!(
        resolver
            .extract_reference(r#"import z from "lodash";"#)
            .unwrap()
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
            .unwrap()
            .text,
        "crate::baz::qux"
    );
    assert_eq!(
        resolver
            .extract_reference("use std::collections::HashMap;")
            .unwrap()
            .text,
        "std::collections::HashMap"
    );
    assert_eq!(
        resolver
            .extract_reference("use super::thing;")
            .unwrap()
            .text,
        "super::thing"
    );
    assert_eq!(
        resolver
            .extract_reference("use self::inner as renamed;")
            .unwrap()
            .text,
        "self::inner"
    );
    assert_eq!(
        resolver
            .extract_reference("pub use crate::foo::Bar;")
            .unwrap()
            .text,
        "crate::foo::Bar"
    );
}

#[test]
fn an_unsupported_language_returns_none() {
    let resolver = get_resolver("go");
    assert_eq!(resolver.extract_reference("import \"fmt\""), None);
    let resolver = get_resolver("c");
    assert_eq!(resolver.extract_reference("#include <stdio.h>"), None);
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
