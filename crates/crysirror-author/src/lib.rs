// SPDX-License-Identifier: MPL-2.0
//! Optional TS/JS authoring: source -> JavaScript -> validated project data.
//! This is an offline builder, not a legacy VM or a per-frame scripting host.
#![forbid(unsafe_code)]

use std::{fs::File, io::Read, path::Path, rc::Rc};

use anyhow::{bail, ensure, Context as _, Result};
use boa_engine::{module::IdleModuleLoader, Context, Source};
use crysirror_runtime::{model::MAX_PROJECT_BYTES, Project};
use oxc::{
    allocator::Allocator,
    ast::ast::{ImportExpression, TSImportEqualsDeclaration},
    ast_visit::Visit,
    codegen::Codegen,
    parser::Parser,
    semantic::SemanticBuilder,
    span::SourceType,
    transformer::{TransformOptions, Transformer},
};

pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct ModuleCheck {
    unsupported: bool,
}
impl<'a> Visit<'a> for ModuleCheck {
    fn visit_import_expression(&mut self, _: &ImportExpression<'a>) {
        self.unsupported = true;
    }
    fn visit_ts_import_equals_declaration(&mut self, _: &TSImportEqualsDeclaration<'a>) {
        self.unsupported = true;
    }
}

/// Transpile a single classic script. Oxc removes TS types; it does not type-check.
/// Modules, JSX and package resolution are deliberately outside this profile.
pub fn transpile(source: &str, path: &Path) -> Result<String> {
    ensure!(
        source.len() <= MAX_SOURCE_BYTES,
        "authoring source exceeds 1 MiB"
    );
    let source_type = match path.extension().and_then(|s| s.to_str()) {
        Some("ts") => SourceType::ts().with_module(false),
        Some("js") => SourceType::cjs(),
        _ => bail!("authoring expects a .ts or .js classic script"),
    };
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    ensure!(
        parsed.errors.is_empty(),
        "parse {}: {:?}",
        path.display(),
        parsed.errors
    );
    let mut program = parsed.program;
    ensure!(
        !program.body.iter().any(|s| s.is_module_declaration()),
        "module imports/exports are not supported by the classic-script authoring profile"
    );
    let mut modules = ModuleCheck::default();
    modules.visit_program(&program);
    ensure!(
        !modules.unsupported,
        "module loading is not supported by the classic-script authoring profile"
    );
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&program);
    ensure!(
        semantic.errors.is_empty(),
        "semantic {}: {:?}",
        path.display(),
        semantic.errors
    );
    let options = TransformOptions::default();
    let transformed = Transformer::new(&allocator, path, &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    ensure!(
        transformed.errors.is_empty(),
        "transform {}: {:?}",
        path.display(),
        transformed.errors
    );
    Ok(Codegen::new().build(&program).code)
}

/// Evaluate buildProject() in a fresh Boa context with no native host bindings.
/// Loop/stack limits are defensive guards, not a time or heap sandbox. Only run
/// trusted authoring code; use process isolation for untrusted generated source.
pub fn build_project(source: &str, path: &Path) -> Result<Project> {
    let javascript = transpile(source, path)?;
    // Boa's default loader reads the current directory. Disable it explicitly,
    // including imports created through eval/Function at execution time.
    let mut context = Context::builder()
        .module_loader(Rc::new(IdleModuleLoader))
        .build()
        .map_err(|e| anyhow::anyhow!("create authoring context: {e}"))?;
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(100_000);
    context.runtime_limits_mut().set_recursion_limit(128);
    context.runtime_limits_mut().set_stack_size_limit(4096);
    // Capture the intrinsic before user code can replace JSON/stringify. Both
    // execution and serialization run within the same guarded evaluation.
    let script = format!(
        "(function () {{\n'use strict';\nconst __crysirrorStringify = JSON.stringify;\n\
         return (function () {{\n{javascript}\n\
         return __crysirrorStringify(buildProject());\n}})();\n}})()"
    );
    let result = context
        .eval(Source::from_bytes(&script))
        .map_err(|e| anyhow::anyhow!("execute {}: {e}", path.display()))?;
    let string = result
        .as_string()
        .context("buildProject() must return JSON project data synchronously")?;
    ensure!(
        string.len() <= MAX_PROJECT_BYTES,
        "generated project exceeds 4 MiB"
    );
    let json = string
        .to_std_string()
        .context("project JSON contains invalid Unicode")?;
    ensure!(
        json.len() <= MAX_PROJECT_BYTES,
        "generated project exceeds 4 MiB"
    );
    let project: Project = serde_json::from_str(&json).context("generated project JSON")?;
    project.validate().context("generated project validation")?;
    Ok(project)
}

pub fn build_file(path: &Path) -> Result<Project> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .take(MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_SOURCE_BYTES,
        "authoring source exceeds 1 MiB"
    );
    build_project(
        std::str::from_utf8(&bytes).context("authoring source must be UTF-8")?,
        path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../examples/desktop/arena.ts");

    #[test]
    fn typescript_builds_data_that_the_runtime_executes() {
        let project = build_project(EXAMPLE, Path::new("arena.ts")).unwrap();
        assert_eq!(project.scene3d.as_ref().unwrap().meshes.len(), 10);
        let dir = tempfile::tempdir().unwrap();
        let anchor =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/project.json");
        let mut runtime = crysirror_runtime::Runtime::with_registry(
            project,
            &anchor,
            dir.path(),
            Default::default(),
        )
        .unwrap();
        runtime.tick(16, Default::default()).unwrap();
        assert!(runtime.all_halted());
        let save = runtime.snapshot().unwrap();
        runtime.restore(&save).unwrap();
    }

    #[test]
    fn javascript_and_typescript_share_the_data_contract() {
        let js = transpile(EXAMPLE, Path::new("arena.ts")).unwrap();
        let ts_project = build_project(EXAMPLE, Path::new("arena.ts")).unwrap();
        let js_project = build_project(&js, Path::new("arena.js")).unwrap();
        assert_eq!(
            ts_project.fingerprint().unwrap(),
            js_project.fingerprint().unwrap()
        );
    }

    #[test]
    fn invalid_data_and_missing_entrypoint_fail() {
        for source in [
            "function buildProject() { return {}; }",
            "const x = 1;",
            "async function buildProject() { return {}; }",
            "function buildProject() {}",
            "function buildProject() { throw new Error('bad recipe'); }",
        ] {
            assert!(build_project(source, Path::new("bad.js")).is_err());
        }
        let source = EXAMPLE.replace("crysirror-script-v1", "rfvp");
        let p = build_project(&source, Path::new("native.ts")).unwrap();
        assert!(p.require_builtin_adapters().is_err());
    }

    #[test]
    fn syntax_and_unsupported_source_profiles_fail() {
        for (source, name) in [
            ("function {", "bad.ts"),
            ("import foo from 'foo';", "module.js"),
            ("export const x = 1", "module.ts"),
            ("void import('./other.js');", "dynamic.js"),
            ("import other = require('./other');", "require.ts"),
            (EXAMPLE, "arena.tsx"),
        ] {
            assert!(transpile(source, Path::new(name)).is_err(), "{name}");
        }
        assert!(transpile(&" ".repeat(MAX_SOURCE_BYTES + 1), Path::new("big.js")).is_err());
    }

    #[test]
    fn loops_and_recursion_are_guarded() {
        for source in [
            "function buildProject() { while (true) {} }",
            "function buildProject() { return buildProject(); }",
        ] {
            assert!(build_project(source, Path::new("limit.js")).is_err());
        }
    }

    #[test]
    fn no_ambient_node_or_browser_capabilities() {
        let probe = "if ([typeof process, typeof require, typeof fetch, typeof window].some(x => x !== 'undefined')) throw Error('ambient host');";
        build_project(&format!("{probe}\n{EXAMPLE}"), Path::new("arena.ts")).unwrap();
    }
}
