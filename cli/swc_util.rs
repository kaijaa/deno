// Copyright 2018-2020 the Deno authors. All rights reserved. MIT license.

#![allow(unused)]

use crate::swc_common;
use crate::swc_common::comments::CommentKind;
use crate::swc_common::comments::Comments;
use crate::swc_common::errors::Diagnostic;
use crate::swc_common::errors::DiagnosticBuilder;
use crate::swc_common::errors::Emitter;
use crate::swc_common::errors::Handler;
use crate::swc_common::errors::HandlerFlags;
use crate::swc_common::BytePos;
use crate::swc_common::FileName;
use crate::swc_common::Globals;
use crate::swc_common::SourceMap;
use crate::swc_common::Span;
use crate::swc_ecma_ast;
use crate::swc_ecma_parser::lexer::Lexer;
use crate::swc_ecma_parser::JscTarget;
use crate::swc_ecma_parser::Parser;
use crate::swc_ecma_parser::Session;
use crate::swc_ecma_parser::SourceFileInput;
use crate::swc_ecma_parser::Syntax;
use crate::swc_ecma_parser::TsConfig;
use swc_ecma_visit::Node;
use swc_ecma_visit::Visit;

use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Clone, Debug)]
pub struct SwcDiagnosticBuffer {
  pub diagnostics: Vec<Diagnostic>,
}

impl Error for SwcDiagnosticBuffer {}

impl fmt::Display for SwcDiagnosticBuffer {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let msg = self
      .diagnostics
      .iter()
      .map(|d| d.message())
      .collect::<Vec<String>>()
      .join(",");

    f.pad(&msg)
  }
}

#[derive(Clone)]
pub struct SwcErrorBuffer(Arc<RwLock<SwcDiagnosticBuffer>>);

impl SwcErrorBuffer {
  pub fn default() -> Self {
    Self(Arc::new(RwLock::new(SwcDiagnosticBuffer {
      diagnostics: vec![],
    })))
  }
}

impl Emitter for SwcErrorBuffer {
  fn emit(&mut self, db: &DiagnosticBuilder) {
    self.0.write().unwrap().diagnostics.push((**db).clone());
  }
}

impl From<SwcErrorBuffer> for SwcDiagnosticBuffer {
  fn from(buf: SwcErrorBuffer) -> Self {
    let s = buf.0.read().unwrap();
    s.clone()
  }
}

/// Low-level utility structure with common AST parsing functions.
///
/// Allows to build more complicated parser by providing a callback
/// to `parse_module`.
pub struct AstParser {
  pub buffered_error: SwcErrorBuffer,
  pub source_map: Arc<SourceMap>,
  pub handler: Handler,
  pub comments: Comments,
  pub globals: Globals,
}

impl AstParser {
  pub fn new() -> Self {
    let buffered_error = SwcErrorBuffer::default();

    let handler = Handler::with_emitter_and_flags(
      Box::new(buffered_error.clone()),
      HandlerFlags {
        dont_buffer_diagnostics: true,
        can_emit_warnings: true,
        ..Default::default()
      },
    );

    AstParser {
      buffered_error,
      source_map: Arc::new(SourceMap::default()),
      handler,
      comments: Comments::default(),
      globals: Globals::new(),
    }
  }

  pub fn parse_module<F, R>(
    &self,
    file_name: &str,
    source_code: &str,
    callback: F,
  ) -> R
  where
    F: FnOnce(Result<swc_ecma_ast::Module, SwcDiagnosticBuffer>) -> R,
  {
    swc_common::GLOBALS.set(&self.globals, || {
      let swc_source_file = self.source_map.new_source_file(
        FileName::Custom(file_name.to_string()),
        source_code.to_string(),
      );

      let buffered_err = self.buffered_error.clone();
      let session = Session {
        handler: &self.handler,
      };

      let mut ts_config = TsConfig::default();
      ts_config.dynamic_import = true;
      let syntax = Syntax::Typescript(ts_config);

      let lexer = Lexer::new(
        session,
        syntax,
        JscTarget::Es2019,
        SourceFileInput::from(&*swc_source_file),
        Some(&self.comments),
      );

      let mut parser = Parser::new_from(session, lexer);

      let parse_result =
        parser
          .parse_module()
          .map_err(move |mut err: DiagnosticBuilder| {
            err.cancel();
            SwcDiagnosticBuffer::from(buffered_err)
          });

      callback(parse_result)
    })
  }

  pub fn get_span_location(&self, span: Span) -> swc_common::Loc {
    self.source_map.lookup_char_pos(span.lo())
  }

  pub fn get_span_comments(
    &self,
    span: Span,
  ) -> Vec<swc_common::comments::Comment> {
    let maybe_comments = self.comments.take_leading_comments(span.lo());

    if let Some(comments) = maybe_comments {
      // clone the comments and put them back in map
      let to_return = comments.clone();
      self.comments.add_leading(span.lo(), comments);
      to_return
    } else {
      vec![]
    }
  }
}

struct DependencyVisitor {
  dependencies: Vec<String>,
  analyze_dynamic_imports: bool,
}

impl Visit for DependencyVisitor {
  fn visit_import_decl(
    &mut self,
    import_decl: &swc_ecma_ast::ImportDecl,
    _parent: &dyn Node,
  ) {
    let src_str = import_decl.src.value.to_string();
    self.dependencies.push(src_str);
  }

  fn visit_named_export(
    &mut self,
    named_export: &swc_ecma_ast::NamedExport,
    _parent: &dyn Node,
  ) {
    if let Some(src) = &named_export.src {
      let src_str = src.value.to_string();
      self.dependencies.push(src_str);
    }
  }

  fn visit_export_all(
    &mut self,
    export_all: &swc_ecma_ast::ExportAll,
    _parent: &dyn Node,
  ) {
    let src_str = export_all.src.value.to_string();
    self.dependencies.push(src_str);
  }

  fn visit_call_expr(
    &mut self,
    call_expr: &swc_ecma_ast::CallExpr,
    _parent: &dyn Node,
  ) {
    if !self.analyze_dynamic_imports {
      return;
    }

    use swc_ecma_ast::Expr::*;
    use swc_ecma_ast::ExprOrSuper::*;

    let boxed_expr = match call_expr.callee.clone() {
      Super(_) => return,
      Expr(boxed) => boxed,
    };

    match &*boxed_expr {
      Ident(ident) => {
        if &ident.sym.to_string() != "import" {
          return;
        }
      }
      _ => return,
    };

    if let Some(arg) = call_expr.args.get(0) {
      match &*arg.expr {
        Lit(lit) => {
          if let swc_ecma_ast::Lit::Str(str_) = lit {
            let src_str = str_.value.to_string();
            self.dependencies.push(src_str);
          }
        }
        _ => return,
      }
    }
  }
}

/// Given file name and source code return vector
/// of unresolved import specifiers.
///
/// Returned vector may contain duplicate entries.
///
/// Second argument allows to configure if dynamic
/// imports should be analyzed.
///
/// NOTE: Only statically analyzable dynamic imports
/// are considered; ie. the ones that have plain string specifier:
///
///    await import("./fizz.ts")
///
/// These imports will be ignored:
///
///    await import(`./${dir}/fizz.ts`)
///    await import("./" + "fizz.ts")
#[allow(unused)]
pub fn analyze_dependencies(
  source_code: &str,
  analyze_dynamic_imports: bool,
) -> Result<Vec<String>, SwcDiagnosticBuffer> {
  let parser = AstParser::new();
  parser.parse_module("root.ts", source_code, |parse_result| {
    let module = parse_result?;
    let mut collector = DependencyVisitor {
      dependencies: vec![],
      analyze_dynamic_imports,
    };
    collector.visit_module(&module, &module);
    Ok(collector.dependencies)
  })
}

#[test]
fn test_analyze_dependencies() {
  let source = r#"
import { foo } from "./foo.ts";
export { bar } from "./foo.ts";
export * from "./bar.ts";
"#;

  let dependencies =
    analyze_dependencies(source, false).expect("Failed to parse");
  assert_eq!(
    dependencies,
    vec![
      "./foo.ts".to_string(),
      "./foo.ts".to_string(),
      "./bar.ts".to_string(),
    ]
  );
}

#[test]
fn test_analyze_dependencies_dyn_imports() {
  let source = r#"
import { foo } from "./foo.ts";
export { bar } from "./foo.ts";
export * from "./bar.ts";

const a = await import("./fizz.ts");
const a = await import("./" + "buzz.ts");
"#;

  let dependencies =
    analyze_dependencies(source, true).expect("Failed to parse");
  assert_eq!(
    dependencies,
    vec![
      "./foo.ts".to_string(),
      "./foo.ts".to_string(),
      "./bar.ts".to_string(),
      "./fizz.ts".to_string(),
    ]
  );
}

#[derive(Clone, Debug, PartialEq)]
enum DependencyKind {
  Import,
  Export,
}

#[derive(Clone, Debug, PartialEq)]
struct DependencyDescriptor {
  span: Span,
  specifier: String,
  kind: DependencyKind,
}

struct NewDependencyVisitor {
  dependencies: Vec<DependencyDescriptor>,
}

impl Visit for NewDependencyVisitor {
  fn visit_import_decl(
    &mut self,
    import_decl: &swc_ecma_ast::ImportDecl,
    _parent: &dyn Node,
  ) {
    let src_str = import_decl.src.value.to_string();
    self.dependencies.push(DependencyDescriptor {
      specifier: src_str,
      kind: DependencyKind::Import,
      span: import_decl.span,
    });
  }

  fn visit_named_export(
    &mut self,
    named_export: &swc_ecma_ast::NamedExport,
    _parent: &dyn Node,
  ) {
    if let Some(src) = &named_export.src {
      let src_str = src.value.to_string();
      self.dependencies.push(DependencyDescriptor {
        specifier: src_str,
        kind: DependencyKind::Export,
        span: named_export.span,
      });
    }
  }

  fn visit_export_all(
    &mut self,
    export_all: &swc_ecma_ast::ExportAll,
    _parent: &dyn Node,
  ) {
    let src_str = export_all.src.value.to_string();
    self.dependencies.push(DependencyDescriptor {
      specifier: src_str,
      kind: DependencyKind::Export,
      span: export_all.span,
    });
  }
}

fn get_deno_types(parser: &AstParser, span: Span) -> Option<String> {
  let comments = parser.get_span_comments(span);

  if comments.is_empty() {
    return None;
  }

  // @deno-types must directly prepend import statement - hence
  // checking last comment for span
  let last = comments.last().unwrap();
  let comment = last.text.trim_start();

  if comment.starts_with("@deno-types") {
    let split: Vec<&str> = comment.split("=").collect();
    assert_eq!(split.len(), 2);
    let specifier_in_quotes = split.get(1).unwrap().to_string();
    let specifier = specifier_in_quotes
      .trim_start_matches("\"")
      .trim_start_matches("\'")
      .trim_end_matches("\"")
      .trim_end_matches("\'")
      .to_string();
    return Some(specifier);
  }

  None
}

#[derive(Clone, Debug, PartialEq)]
struct ImportDescriptor {
  specifier: String,
  deno_types: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum TsReferenceKind {
  Lib,
  Types,
  Path,
}

#[derive(Clone, Debug, PartialEq)]
struct TsReferenceDescriptor {
  kind: TsReferenceKind,
  specifier: String,
}

#[allow(unused)]
fn analyze_dependencies_and_references(
  source_code: &str,
  analyze_dynamic_imports: bool,
) -> Result<
  (Vec<ImportDescriptor>, Vec<TsReferenceDescriptor>),
  SwcDiagnosticBuffer,
> {
  let parser = AstParser::new();
  parser.parse_module("root.ts", source_code, |parse_result| {
    let module = parse_result?;
    let mut collector = NewDependencyVisitor {
      dependencies: vec![],
    };
    let module_span = module.span;
    collector.visit_module(&module, &module);

    let dependency_descriptors = collector.dependencies;

    // for each import check if there's relevant @deno-types directive
    let imports = dependency_descriptors
      .iter()
      .map(|mut desc| {
        if desc.kind == DependencyKind::Import {
          let deno_types = get_deno_types(&parser, desc.span);
          ImportDescriptor {
            specifier: desc.specifier.to_string(),
            deno_types,
          }
        } else {
          ImportDescriptor {
            specifier: desc.specifier.to_string(),
            deno_types: None,
          }
        }
      })
      .collect();

    // analyze comment from beginning of the file and find TS directives
    eprintln!("module span {:?}", module_span);
    let comments = parser
      .comments
      .take_leading_comments(module_span.lo())
      .unwrap_or_else(|| vec![]);

    let mut references = vec![];
    for comment in comments {
      if comment.kind != CommentKind::Line {
        continue;
      }

      let text = comment.text.to_string();
      let (kind, specifier_in_quotes) =
        if text.starts_with("/ <reference path=") {
          (
            TsReferenceKind::Path,
            text.trim_start_matches("/ <reference path="),
          )
        } else if text.starts_with("/ <reference lib=") {
          (
            TsReferenceKind::Lib,
            text.trim_start_matches("/ <reference lib="),
          )
        } else if text.starts_with("/ <reference types=") {
          (
            TsReferenceKind::Types,
            text.trim_start_matches("/ <reference types="),
          )
        } else {
          continue;
        };
      let specifier = specifier_in_quotes
        .trim_end_matches("/>")
        .trim_end()
        .trim_start_matches("\"")
        .trim_start_matches("\'")
        .trim_end_matches("\"")
        .trim_end_matches("\'")
        .to_string();

      references.push(TsReferenceDescriptor { kind, specifier });
    }
    Ok((imports, references))
  })
}

#[test]
fn test_analyze_dependencies_and_directives() {
  let source = r#"
// This comment is placed to make sure that directives are parsed
// even when they start on non-first line
  
/// <reference lib="dom" />
/// <reference types="./type_reference.d.ts" />
/// <reference path="./type_reference/dep.ts" />
// @deno-types="./type_definitions/foo.d.ts"
import { foo } from "./type_definitions/foo.js";
// @deno-types="./type_definitions/fizz.d.ts"
import "./type_definitions/fizz.js";

/// <reference path="./type_reference/dep2.ts" />

import * as qat from "./type_definitions/qat.ts";

console.log(foo);
console.log(fizz);
console.log(qat.qat);  
"#;

  let (imports, references) =
    analyze_dependencies_and_references(source, true).expect("Failed to parse");

  assert_eq!(
    imports,
    vec![
      ImportDescriptor {
        specifier: "./type_definitions/foo.js".to_string(),
        deno_types: Some("./type_definitions/foo.d.ts".to_string())
      },
      ImportDescriptor {
        specifier: "./type_definitions/fizz.js".to_string(),
        deno_types: Some("./type_definitions/fizz.d.ts".to_string())
      },
      ImportDescriptor {
        specifier: "./type_definitions/qat.ts".to_string(),
        deno_types: None
      },
    ]
  );

  // According to TS docs (https://www.typescriptlang.org/docs/handbook/triple-slash-directives.html)
  // directives that are not at the top of the file are ignored, so only
  // 3 references should be captured instead of 4.
  assert_eq!(
    references,
    vec![
      TsReferenceDescriptor {
        specifier: "dom".to_string(),
        kind: TsReferenceKind::Lib,
      },
      TsReferenceDescriptor {
        specifier: "./type_reference.d.ts".to_string(),
        kind: TsReferenceKind::Types,
      },
      TsReferenceDescriptor {
        specifier: "./type_reference/dep.ts".to_string(),
        kind: TsReferenceKind::Path,
      },
    ]
  );
}
