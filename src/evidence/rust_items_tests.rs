use crate::evidence::extract;
use crate::model::{
    CommentPayload, EvidenceItem, EvidenceOrigin, EvidencePayload, StructurePayload, SymbolPayload,
};
use slugaudit_test_check::{Check, TestResult, ensure, ensure_eq};

fn extract_rust_evidence(source: &str) -> TestResult<Vec<EvidenceItem>> {
    let (items, truncated) = extract("rust", source).check("extract rust evidence")?;
    ensure(!truncated, "fixture extraction must not truncate")?;
    Ok(items)
}

fn find_structure<'a>(items: &'a [EvidenceItem], name: &str) -> TestResult<&'a StructurePayload> {
    items
        .iter()
        .find_map(|item| match &item.payload {
            EvidencePayload::Structure(s) if s.name.as_deref() == Some(name) => Some(s),
            _ => None,
        })
        .check(&format!("Structure item `{name}` present"))
}

fn find_symbol<'a>(items: &'a [EvidenceItem], name: &str) -> TestResult<&'a SymbolPayload> {
    items
        .iter()
        .find_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) if s.name == name => Some(s),
            _ => None,
        })
        .check(&format!("Symbol item `{name}` present"))
}

fn find_symbol_item<'a>(items: &'a [EvidenceItem], name: &str) -> TestResult<&'a EvidenceItem> {
    items
        .iter()
        .find(|item| matches!(&item.payload, EvidencePayload::Symbol(s) if s.name == name))
        .check(&format!("Symbol EvidenceItem `{name}` present"))
}

#[test]
fn rust_visibility_decorators_and_bridged_docs_populate_structure_and_fields() -> TestResult {
    let source = concat!(
        "/// Public function doc.\n",
        "#[must_use]\n",
        "#[inline]\n",
        "pub fn public_fn() -> u8 { 1 }\n",
        "pub(crate) struct CrateStruct {\n",
        "    /// Documented field.\n",
        "    #[serde(default)]\n",
        "    pub(super) count: usize,\n",
        "    secret: String,\n",
        "}\n",
    );
    let items = extract_rust_evidence(source)?;

    let pub_fn_struct = find_structure(&items, "public_fn")?;
    ensure_eq(&pub_fn_struct.visibility.as_deref(), &Some("pub"), "vis")?;
    ensure_eq(
        &pub_fn_struct.decorators,
        &vec!["#[must_use]".to_owned(), "#[inline]".to_owned()],
        "Structure decorators carry outer #[...] attributes in source order",
    )?;
    ensure_eq(
        &pub_fn_struct.doc_comment.as_deref(),
        &Some("/// Public function doc."),
        "Structure public_fn doc_comment across outer attributes",
    )?;

    let pub_fn_sym = find_symbol(&items, "public_fn")?;
    ensure_eq(&pub_fn_sym.visibility.as_deref(), &Some("pub"), "sym vis")?;
    ensure_eq(
        &pub_fn_sym.doc.as_deref(),
        &Some("/// Public function doc."),
        "Symbol public_fn doc across outer attributes",
    )?;

    let crate_struct = find_structure(&items, "CrateStruct")?;
    ensure_eq(
        &crate_struct.visibility.as_deref(),
        &Some("pub(crate)"),
        "CrateStruct visibility",
    )?;
    let crate_struct_sym = find_symbol(&items, "CrateStruct")?;
    ensure_eq(&crate_struct_sym.kind.as_str(), &"Struct", "Struct symbol")?;

    let count_field = find_symbol(&items, "count")?;
    ensure_eq(
        &count_field.visibility.as_deref(),
        &Some("pub(super)"),
        "field count visibility",
    )?;
    ensure_eq(
        &count_field.type_annotation.as_deref(),
        &Some("usize"),
        "field count type_annotation",
    )?;
    ensure_eq(
        &count_field.doc.as_deref(),
        &Some("/// Documented field."),
        "field count doc across field attribute",
    )?;

    let secret_field = find_symbol(&items, "secret")?;
    ensure_eq(&secret_field.visibility, &None, "private field visibility")?;
    ensure_eq(
        &secret_field.type_annotation.as_deref(),
        &Some("String"),
        "private field type_annotation",
    )?;
    ensure_eq(&secret_field.doc, &None, "undocumented field doc")?;
    Ok(())
}

#[test]
fn rust_visibility_populates_restricted_and_private_items_and_symbols() -> TestResult {
    let source = concat!(
        "pub(in crate::inner) enum ScopedEnum { A }\n",
        "fn private_fn() {}\n",
        "pub type Alias = u64;\n",
        "pub(crate) const LIMIT: usize = 16;\n",
        "pub trait Runner {}\n",
        "pub mod submod;\n",
    );
    let items = extract_rust_evidence(source)?;

    ensure_eq(
        &find_structure(&items, "ScopedEnum")?.visibility.as_deref(),
        &Some("pub(in crate::inner)"),
        "ScopedEnum Structure visibility",
    )?;
    ensure_eq(
        &find_symbol(&items, "ScopedEnum")?.kind.as_str(),
        &"Enum",
        "ScopedEnum Symbol kind",
    )?;
    ensure_eq(
        &find_structure(&items, "private_fn")?.visibility,
        &None,
        "private fn visibility",
    )?;
    let alias_sym = find_symbol(&items, "Alias")?;
    ensure_eq(&alias_sym.visibility.as_deref(), &Some("pub"), "Alias vis")?;
    ensure_eq(
        &alias_sym.type_annotation.as_deref(),
        &Some("u64"),
        "Alias type_annotation",
    )?;
    let limit_sym = find_symbol(&items, "LIMIT")?;
    ensure_eq(
        &limit_sym.visibility.as_deref(),
        &Some("pub(crate)"),
        "LIMIT vis",
    )?;
    ensure_eq(
        &find_structure(&items, "Runner")?.visibility.as_deref(),
        &Some("pub"),
        "Runner Structure vis",
    )?;
    ensure_eq(
        &find_symbol(&items, "Runner")?.kind.as_str(),
        &"Trait",
        "Runner Symbol kind",
    )?;
    ensure_eq(
        &find_structure(&items, "submod")?.visibility.as_deref(),
        &Some("pub"),
        "submod Structure vis",
    )?;
    ensure_eq(
        &find_symbol(&items, "submod")?.kind.as_str(),
        &"Module",
        "submod Symbol kind",
    )?;
    Ok(())
}

#[test]
fn rust_statics_surface_with_visibility_type_annotation_and_doc() -> TestResult {
    let source = concat!(
        "/// Global default timeout.\n",
        "pub static DEFAULT_TIMEOUT_MS: u64 = 5_000;\n",
        "static mut COUNTER: usize = 0;\n",
    );
    let items = extract_rust_evidence(source)?;

    let immutable_item = find_symbol_item(&items, "DEFAULT_TIMEOUT_MS")?;
    ensure_eq(
        &immutable_item.origin,
        &EvidenceOrigin::GenericWalker,
        "static_item extracted by GenericWalker",
    )?;
    let immutable = find_symbol(&items, "DEFAULT_TIMEOUT_MS")?;
    ensure_eq(&immutable.kind.as_str(), &"Constant", "immutable static")?;
    ensure_eq(&immutable.visibility.as_deref(), &Some("pub"), "vis")?;
    ensure_eq(&immutable.type_annotation.as_deref(), &Some("u64"), "ty")?;
    ensure_eq(
        &immutable.doc.as_deref(),
        &Some("/// Global default timeout."),
        "doc",
    )?;

    let mutable = find_symbol(&items, "COUNTER")?;
    ensure_eq(&mutable.kind.as_str(), &"Variable", "static mut kind")?;
    ensure_eq(&mutable.visibility, &None, "private static mut vis")?;
    ensure_eq(&mutable.type_annotation.as_deref(), &Some("usize"), "ty")?;
    Ok(())
}

#[test]
fn rust_destructuring_let_and_parameters_extract_every_bound_identifier_and_types() -> TestResult {
    let source = concat!(
        "struct Pair { x: i32, y: i32 }\n",
        "fn unpack(&self, (left, right): (u32, &str), Pair { x, y: renamed }: Pair) {\n",
        "    let (first, mut second): (usize, String) = make_pair();\n",
        "    let Pair { x: px, y } = build();\n",
        "    let Some(unwrapped) = maybe_val else { return; };\n",
        "    let _ = ignored_rhs_ident;\n",
        "}\n",
    );
    let items = extract_rust_evidence(source)?;
    let vars: Vec<(&str, Option<&str>)> = items
        .iter()
        .filter(|item| item.origin == EvidenceOrigin::GenericWalker)
        .filter_map(|item| match &item.payload {
            EvidencePayload::Symbol(s) if s.kind == "Variable" => {
                Some((s.name.as_str(), s.type_annotation.as_deref()))
            }
            _ => None,
        })
        .collect();

    ensure_eq(
        &vars,
        &vec![
            ("left", Some("(u32, &str)")),
            ("right", Some("(u32, &str)")),
            ("x", Some("Pair")),
            ("renamed", Some("Pair")),
            ("first", Some("(usize, String)")),
            ("second", Some("(usize, String)")),
            ("px", None),
            ("y", None),
            ("unwrapped", None),
        ],
        "every destructured identifier is extracted without struct types, field labels, or wildcard RHS",
    )?;
    Ok(())
}

#[test]
fn detached_doc_comment_across_blank_line_does_not_attach_to_following_field() -> TestResult {
    let source = concat!(
        "struct Holder {\n",
        "    /// Orphan comment separated by blank line.\n",
        "\n",
        "    pub value: u32,\n",
        "}\n",
    );
    let items = extract_rust_evidence(source)?;
    ensure_eq(
        &find_symbol(&items, "value")?.doc,
        &None,
        "blank line detaches preceding doc comment from field",
    )?;
    Ok(())
}

#[test]
fn inner_module_doc_comments_and_inner_attrs_never_leak_onto_following_items() -> TestResult {
    let source = concat!(
        "//! Module-level inner line doc.\n",
        "/*! Module-level inner block doc. */\n",
        "#![allow(dead_code)]\n",
        "pub fn first_item() {}\n",
    );
    let items = extract_rust_evidence(source)?;

    let first_struct = find_structure(&items, "first_item")?;
    ensure_eq(
        &first_struct.doc_comment,
        &None,
        "inner //! and /*! docs must never attach to Structure.doc_comment",
    )?;
    ensure(
        first_struct.decorators.is_empty(),
        "inner #![...] attribute must never attach to Structure.decorators",
    )?;
    ensure_eq(
        &find_symbol(&items, "first_item")?.doc,
        &None,
        "inner //! and /*! docs must never attach to Symbol.doc",
    )?;

    let comments: Vec<&CommentPayload> = items
        .iter()
        .filter_map(|item| match &item.payload {
            EvidencePayload::Comment(c) => Some(c),
            _ => None,
        })
        .collect();
    ensure_eq(&comments.len(), &2usize, "both inner doc comments indexed")?;
    ensure(
        comments.iter().all(|c| c.kind == "Doc"),
        "inner doc comments still classify as Doc in Comment evidence",
    )?;
    Ok(())
}

#[test]
fn extracts_trait_signatures_associated_types_unions_and_enum_variants() -> TestResult {
    let source = concat!(
        "/// Trait doc.\n",
        "pub trait Parser {\n",
        "    /// Associated error type.\n",
        "    type Err: core::fmt::Debug;\n",
        "    /// Required trait method.\n",
        "    #[must_use]\n",
        "    fn parse(&self, input: &str) -> Result<u32, Self::Err>;\n",
        "    fn provided(&self) -> bool { true }\n",
        "}\n",
        "pub union RawWord {\n",
        "    /// 64-bit word view.\n",
        "    pub bits: u64,\n",
        "    pub bytes: [u8; 8],\n",
        "}\n",
        "pub enum Mode {\n",
        "    /// Fast mode.\n",
        "    Fast,\n",
        "    Thorough(u8),\n",
        "}\n",
    );
    let items = extract_rust_evidence(source)?;

    let assoc_ty = find_symbol(&items, "Err")?;
    ensure_eq(&assoc_ty.kind.as_str(), &"Type", "associated_type is Type")?;
    ensure_eq(
        &assoc_ty.type_annotation.as_deref(),
        &Some("core::fmt::Debug"),
        "associated_type bounds captured in type_annotation",
    )?;
    ensure_eq(
        &assoc_ty.doc.as_deref(),
        &Some("/// Associated error type."),
        "associated_type doc",
    )?;

    let parse_struct = find_structure(&items, "parse")?;
    ensure_eq(&parse_struct.kind.as_str(), &"Method", "trait sig Method")?;
    ensure_eq(
        &parse_struct.decorators,
        &vec!["#[must_use]".to_owned()],
        "trait sig decorators",
    )?;
    ensure_eq(
        &parse_struct.signature.as_deref(),
        &Some("fn parse(&self, input: &str) -> Result<u32, Self::Err>"),
        "trait sig signature",
    )?;
    ensure_eq(
        &parse_struct.doc_comment.as_deref(),
        &Some("/// Required trait method."),
        "trait sig doc_comment",
    )?;
    let parse_sym = find_symbol(&items, "parse")?;
    ensure_eq(&parse_sym.kind.as_str(), &"Function", "trait sig Symbol")?;
    ensure_eq(
        &parse_sym.doc.as_deref(),
        &Some("/// Required trait method."),
        "trait sig Symbol doc",
    )?;
    ensure_eq(
        &find_structure(&items, "provided")?.kind.as_str(),
        &"Method",
        "provided trait fn is Method",
    )?;

    ensure_eq(
        &find_structure(&items, "RawWord")?.kind.as_str(),
        &"Union",
        "union Structure kind",
    )?;
    ensure_eq(
        &find_symbol(&items, "RawWord")?.kind.as_str(),
        &"Type",
        "union Symbol kind",
    )?;
    let bits_field = find_symbol(&items, "bits")?;
    ensure_eq(&bits_field.kind.as_str(), &r#"Other("Field")"#, "union fld")?;
    ensure_eq(
        &bits_field.doc.as_deref(),
        &Some("/// 64-bit word view."),
        "union field doc",
    )?;

    ensure_eq(
        &find_symbol_item(&items, "Fast")?.origin,
        &EvidenceOrigin::GenericWalker,
        "enum_variant uses GenericWalker origin",
    )?;
    let fast_variant = find_symbol(&items, "Fast")?;
    ensure_eq(&fast_variant.kind.as_str(), &"Constant", "variant Constant")?;
    ensure_eq(
        &fast_variant.doc.as_deref(),
        &Some("/// Fast mode."),
        "variant doc",
    )?;
    ensure_eq(
        &find_symbol(&items, "Thorough")?.kind.as_str(),
        &"Constant",
        "tuple variant Constant",
    )?;
    Ok(())
}

#[test]
fn extracts_trait_and_inherent_impls_and_macros() -> TestResult {
    let source = concat!(
        "pub enum Mode { Fast }\n",
        "impl core::fmt::Display for Mode {\n",
        "    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { Ok(()) }\n",
        "}\n",
        "impl Mode {\n",
        "    pub fn is_fast(&self) -> bool { true }\n",
        "}\n",
        "/// Exported declarative macro.\n",
        "#[macro_export]\n",
        "macro_rules! build_mode {\n",
        "    () => { Mode::Fast };\n",
        "}\n",
    );
    let items = extract_rust_evidence(source)?;

    ensure_eq(
        &find_structure(&items, "is_fast")?.kind.as_str(),
        &"Method",
        "impl fn is Method",
    )?;
    let trait_impl = find_structure(&items, "core::fmt::Display for Mode")?;
    ensure_eq(&trait_impl.kind.as_str(), &"Impl", "trait impl kind")?;
    ensure_eq(
        &trait_impl.signature.as_deref(),
        &Some("impl core::fmt::Display for Mode"),
        "trait impl signature",
    )?;
    let inherent_impl = items
        .iter()
        .find_map(|item| match &item.payload {
            EvidencePayload::Structure(s)
                if s.kind == "Impl" && s.name.as_deref() == Some("Mode") =>
            {
                Some(s)
            }
            _ => None,
        })
        .check("inherent impl Structure for Mode")?;
    ensure_eq(&inherent_impl.kind.as_str(), &"Impl", "inherent impl kind")?;

    let macro_struct = find_structure(&items, "build_mode")?;
    ensure_eq(&macro_struct.kind.as_str(), &"Macro", "macro Structure")?;
    ensure_eq(
        &macro_struct.decorators,
        &vec!["#[macro_export]".to_owned()],
        "macro decorators",
    )?;
    ensure_eq(
        &macro_struct.signature.as_deref(),
        &Some("macro_rules! build_mode"),
        "macro signature stops before rule body",
    )?;
    ensure_eq(
        &find_symbol(&items, "build_mode")?.kind.as_str(),
        &"Macro",
        "macro Symbol",
    )?;
    Ok(())
}
