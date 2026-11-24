//! Inspect the model-facing engine sites for invariant 8 (ADR-0041).
//!
//! The Rust type checker guards ordinary calls. This check names the audited
//! constructor, template and argument sink. It covers syntax at those sites
//! and the ordinary CLI prefix. It does not resolve arbitrary macro expansions
//! or trace the origin of arbitrary Rust values.

use syn::{spanned::Spanned, visit::Visit};

use crate::{invariants::Violation, workspace::Source};

const ASSET: &str = "engines/identity-appendix.md";

pub(crate) fn check(source: &Source, file: &syn::File, out: &mut Vec<Violation>) {
    let mut audit = Audit {
        source,
        out,
        function: String::new(),
    };
    audit.visit_file(file);
}

struct Audit<'a> {
    source: &'a Source,
    out: &'a mut Vec<Violation>,
    function: String,
}

impl Audit<'_> {
    fn at(&self, file: &str, function: &str) -> bool {
        self.source.path.ends_with(file) && self.function == function
    }

    fn cli_prefix(&self) -> bool {
        self.source.path.ends_with("adapter.rs")
            && matches!(
                self.function.as_str(),
                "claude_arguments" | "conformance_arguments"
            )
    }

    fn refuse(&mut self, node: &impl Spanned, message: &'static str) {
        self.out.push(Violation {
            rule: "prompt-literal",
            file: self.source.path.clone(),
            line: node.span().start().line.max(1),
            message: message.to_owned(),
        });
    }
}

impl<'ast> Visit<'ast> for Audit<'_> {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let test_only = module.attrs.iter().any(|attr| {
            matches!(&attr.meta, syn::Meta::List(list)
                if list.path.is_ident("cfg") && list.tokens.to_string() == "test")
        });
        if !test_only {
            syn::visit::visit_item_mod(self, module);
        }
    }

    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        let enclosing = std::mem::replace(&mut self.function, function.sig.ident.to_string());
        if self.at("adapter.rs", "send_identity")
            && !single_method(&function.block, "prompt", "append_argument").is_some_and(|call| {
                call.args.len() == 1 && call.args.first().is_some_and(|arg| is_path(arg, "args"))
            })
        {
            self.refuse(function, "send_identity must forward only the typed prompt");
        }
        if self.at("adapter.rs", "build_plan")
            && !function.block.stmts.iter().any(|statement| {
                matches!(statement, syn::Stmt::Expr(expr, _) if is_call(expr, "send_identity"))
            })
        {
            self.refuse(function, "build_plan must send the rendered identity through send_identity");
        }
        syn::visit::visit_item_fn(self, function);
        self.function = enclosing;
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        let enclosing = std::mem::replace(&mut self.function, function.sig.ident.to_string());
        if self.at("prompt.rs", "append_argument")
            && !single_method(&function.block, "argv", "push").is_some_and(|call| {
                call.args.len() == 1 && call.args.first().is_some_and(rendered_field)
            })
        {
            self.refuse(
                function,
                "append_argument must forward only the rendered prompt field",
            );
        }
        syn::visit::visit_impl_item_fn(self, function);
        self.function = enclosing;
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if self.source.path.ends_with("prompt.rs") {
            if item.ident == "BUNDLED_IDENTITY" && !bundled(&item.expr) {
                self.refuse(
                    item,
                    "bundle the identity from prompts/engines/identity-appendix.md",
                );
            }
            if item.ident == "IDENTITY_APPENDIX" && literal(&item.expr).as_deref() != Some(ASSET) {
                self.refuse(
                    item,
                    "load the home override at engines/identity-appendix.md",
                );
            }
        }
        syn::visit::visit_item_const(self, item);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        let init = local.init.as_ref().map(|init| init.expr.as_ref());
        if self.at("prompt.rs", "identity") {
            if binding(&local.pat, "template") && !init.is_some_and(loaded_template) {
                self.refuse(
                    local,
                    "load identity through eph_fs::home::prompt with the bundled asset",
                );
            }
            if binding(&local.pat, "rendered") && !init.is_some_and(rendered_template) {
                self.refuse(
                    local,
                    "construct the rendered prompt by expanding the loaded template",
                );
            }
        }
        if self.at("adapter.rs", "build_plan") {
            if binding(&local.pat, "args") {
                self.refuse(
                    local,
                    "keep the adapter's guarded arguments through model send",
                );
            }
            if let syn::Pat::Tuple(tuple) = &local.pat
                && tuple
                    .elems
                    .first()
                    .is_some_and(|item| binding(item, "prompt"))
                && !init.is_some_and(|expr| is_call(without_try(expr), "identity"))
            {
                self.refuse(
                    local,
                    "obtain the model-facing value from the identity renderer",
                );
            }
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        match last(&call.func).as_deref() {
            Some("build_plan")
                if self.at("adapter.rs", "plan")
                    && !call.args.iter().nth(2).is_some_and(prefix_call) =>
            {
                self.refuse(
                    call,
                    "pass only the adapter's ordinary CLI prefix to build_plan",
                );
            }
            Some("RenderedPrompt")
                if !self.at("prompt.rs", "identity")
                    || call.args.len() != 1
                    || !call
                        .args
                        .first()
                        .is_some_and(|expr| is_path(expr, "rendered")) =>
            {
                self.refuse(
                    call,
                    "construct RenderedPrompt only from the identity renderer",
                );
            }
            Some("send_identity")
                if !self.at("adapter.rs", "build_plan")
                    || call.args.len() != 2
                    || !call
                        .args
                        .first()
                        .is_some_and(|expr| is_path(expr, "prompt"))
                    || !call.args.last().is_some_and(|expr| {
                        matches!(expr, syn::Expr::Reference(reference)
                            if reference.mutability.is_some() && is_path(&reference.expr, "args"))
                    }) =>
            {
                self.refuse(
                    call,
                    "send only the rendered prompt through the engine argv boundary",
                );
            }
            _ => {}
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_struct(&mut self, value: &'ast syn::ExprStruct) {
        if value
            .path
            .segments
            .last()
            .is_some_and(|piece| piece.ident == "SpawnPlan")
            && (!self.at("adapter.rs", "build_plan")
                || !value.fields.iter().any(|field| {
                    matches!(&field.member, syn::Member::Named(name) if name == "args")
                        && is_path(&field.expr, "args")
                }))
        {
            self.refuse(
                value,
                "construct a spawn plan only from build_plan's guarded arguments",
            );
        }
        syn::visit::visit_expr_struct(self, value);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "append_argument"
            && (!self.at("adapter.rs", "send_identity")
                || !is_path(&call.receiver, "prompt")
                || call.args.len() != 1
                || !call.args.first().is_some_and(|expr| is_path(expr, "args")))
        {
            self.refuse(call, "unpack a rendered prompt only in send_identity");
        }
        if self.at("adapter.rs", "build_plan") && is_path(&call.receiver, "args") {
            self.refuse(
                call,
                "append model-facing arguments only through send_identity",
            );
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_assign(&mut self, assignment: &'ast syn::ExprAssign) {
        if self.at("adapter.rs", "build_plan") && argument_target(&assignment.left) {
            self.refuse(
                assignment,
                "keep the guarded argument list through model send",
            );
        }
        syn::visit::visit_expr_assign(self, assignment);
    }

    fn visit_expr_lit(&mut self, node: &'ast syn::ExprLit) {
        if self.cli_prefix()
            && let syn::Lit::Str(text) = &node.lit
            && !ordinary_cli_word(&text.value())
        {
            self.refuse(
                node,
                "keep prompt prose in prompts/ and send a rendered prompt",
            );
        }
        syn::visit::visit_expr_lit(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if self.cli_prefix() {
            let args = node.parse_body_with(
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
            );
            if node.path.is_ident("vec")
                && let Ok(args) = args
            {
                for arg in &args {
                    self.visit_expr(arg);
                }
            } else {
                self.refuse(node, "keep CLI argument prefixes explicit; send model text through the rendered prompt");
            }
        }
        syn::visit::visit_macro(self, node);
    }
}

fn ordinary_cli_word(word: &str) -> bool {
    matches!(
        word,
        "--cd"
            | "--permission-mode"
            | "default"
            | "acceptEdits"
            | "auto"
            | "--setting-sources="
            | "--settings"
            | "--plugin-dir"
            | "--append-system-prompt"
    )
}

fn prefix_call(expr: &syn::Expr) -> bool {
    is_call(expr, "claude_arguments")
        || matches!(expr, syn::Expr::MethodCall(call)
            if call.method == "conformance_arguments" && is_path(&call.receiver, "self"))
}

fn single_method<'a>(
    block: &'a syn::Block,
    receiver: &str,
    method: &str,
) -> Option<&'a syn::ExprMethodCall> {
    match block.stmts.as_slice() {
        [syn::Stmt::Expr(syn::Expr::MethodCall(call), _)]
            if call.method == method && is_path(&call.receiver, receiver) =>
        {
            Some(call)
        }
        _ => None,
    }
}

fn rendered_field(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::MethodCall(call) if call.method == "into" && call.args.is_empty()
        && matches!(call.receiver.as_ref(), syn::Expr::Field(field)
            if is_path(&field.base, "self")
            && matches!(&field.member, syn::Member::Unnamed(index) if index.index == 0)))
}

fn binding(pat: &syn::Pat, name: &str) -> bool {
    matches!(pat, syn::Pat::Ident(ident) if ident.ident == name)
}

fn bundled(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Macro(value) if value.mac.path.is_ident("include_str")
    && value.mac.parse_body::<syn::LitStr>().is_ok_and(|path| {
        path.value() == format!("../../../prompts/{ASSET}")
    }))
}

fn literal(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(value) if let syn::Lit::Str(text) = &value.lit => Some(text.value()),
        _ => None,
    }
}

fn loaded_template(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Call(call) if is_path(&call.func, "prompt")
        && call.args.len() == 3
        && call.args.iter().nth(1).is_some_and(|expr| is_path(expr, "IDENTITY_APPENDIX"))
        && call.args.last().is_some_and(|expr| is_path(expr, "BUNDLED_IDENTITY")))
}

fn rendered_template(expr: &syn::Expr) -> bool {
    matches!(without_try(expr), syn::Expr::MethodCall(mapped) if mapped.method == "map_err"
    && matches!(mapped.receiver.as_ref(), syn::Expr::Call(call)
        if is_path(&call.func, "render") && call.args.len() == 2
        && call.args.first().is_some_and(|expr| {
            matches!(expr, syn::Expr::Reference(reference)
                if matches!(reference.expr.as_ref(), syn::Expr::Field(field)
                    if is_path(&field.base, "template")
                    && matches!(&field.member, syn::Member::Named(name) if name == "text")))
        })))
}

fn without_try(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Try(value) => &value.expr,
        _ => expr,
    }
}

fn is_call(expr: &syn::Expr, name: &str) -> bool {
    matches!(expr, syn::Expr::Call(call) if is_path(&call.func, name))
}

fn is_path(expr: &syn::Expr, name: &str) -> bool {
    matches!(expr, syn::Expr::Path(value) if value.path.is_ident(name))
}

fn argument_target(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Index(index) => argument_target(&index.expr),
        syn::Expr::Paren(paren) => argument_target(&paren.expr),
        _ => is_path(expr, "args"),
    }
}

fn last(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(value) => value
            .path
            .segments
            .last()
            .map(|piece| piece.ident.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn violations(file: &str, text: &str) -> Vec<Violation> {
        let source = Source {
            path: format!("crates/eph-engines/src/{file}").into(),
            text: text.into(),
        };
        let tree = syn::parse_file(text).unwrap();
        let mut out = Vec::new();
        check(&source, &tree, &mut out);
        out
    }

    #[test]
    fn the_real_renderer_and_argument_boundary_pass() {
        for (file, text) in [
            (
                "prompt.rs",
                include_str!("../../crates/eph-engines/src/prompt.rs"),
            ),
            (
                "adapter.rs",
                include_str!("../../crates/eph-engines/src/adapter.rs"),
            ),
        ] {
            assert!(violations(file, text).is_empty());
        }
    }

    #[test]
    fn audits_inline_production_modules_and_skips_test_modules() {
        let faults = violations(
            "adapter.rs",
            "mod production {\n    fn bypass() { send_identity(prompt, &mut args); }\n}\n\
             #[cfg(test)]\nmod tests {\n    fn bypass() { send_identity(prompt, &mut args); }\n}",
        );
        assert_eq!(faults.len(), 1, "{faults:?}");
        assert_eq!(faults[0].rule, "prompt-literal");
        assert_eq!(faults[0].line, 2);
    }

    #[test]
    fn permits_ordinary_assignments_outside_the_guarded_arguments() {
        let faults = violations(
            "adapter.rs",
            "fn build_plan() {\n    scratch = value;\n    send_identity(prompt, &mut args);\n}\n\
             fn another_function() { args = values; }",
        );
        assert!(faults.is_empty(), "{faults:?}");
    }

    #[test]
    fn the_sender_requires_the_exact_forwarding_method_and_argument() {
        for (body, expected) in [
            ("prompt.append_argument(args, extra);", 2),
            ("other.append_argument(args);", 2),
            ("prompt.append_argument(wrong_args);", 2),
            ("prompt.another_method(args);", 1),
        ] {
            let faults = violations("adapter.rs", &format!("fn send_identity() {{\n{body}\n}}"));
            assert_eq!(faults.len(), expected, "{body}: {faults:?}");
            assert_eq!(faults[0].line, 1, "{body}");
            assert_eq!(
                faults[0].message, "send_identity must forward only the typed prompt",
                "{body}"
            );
            if expected == 2 {
                assert_eq!(faults[1].line, 2, "{body}");
                assert_eq!(
                    faults[1].message, "unpack a rendered prompt only in send_identity",
                    "{body}"
                );
            }
        }
    }

    #[test]
    fn each_planted_literal_names_the_rule_file_and_line() {
        for (file, text, line) in [
            ("adapter.rs", "fn build_plan() {\nlet result = args;\n}", 1),
            (
                "prompt.rs",
                "\nconst IDENTITY_APPENDIX: &str = another_path();",
                2,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nsend_identity(prompt, &mut args);\nlet args = vec![\"inline\"];\n}",
                3,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nlet (prompt, condition) = inline();\nsend_identity(prompt, &mut args);\n}",
                2,
            ),
            (
                "prompt.rs",
                "\nconst BUNDLED_IDENTITY: &str = \"an inline prompt\";",
                2,
            ),
            (
                "prompt.rs",
                "fn identity() {\nlet template = prompt(config, WRONG, BUNDLED_IDENTITY);\n}",
                2,
            ),
            (
                "prompt.rs",
                "fn identity() {\nlet rendered = \"an inline prompt\".to_owned();\n}",
                2,
            ),
            (
                "prompt.rs",
                "fn identity() {\nRenderedPrompt(format!(\"an inline prompt\"));\n}",
                2,
            ),
            (
                "prompt.rs",
                "fn another_function() {\nRenderedPrompt(rendered);\n}",
                2,
            ),
            (
                "prompt.rs",
                "fn identity() {\nRenderedPrompt(rendered, extra);\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nsend_identity(\"an inline prompt\", &mut args);\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn bypass() {\nsend_identity(prompt, &mut args);\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nsend_identity(prompt, extra, &mut args);\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nsend_identity(prompt, &mut args);\nargs.push(\"inline\".into());\n}",
                3,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nsend_identity(prompt, &mut args);\nSpawnPlan { args: vec![\"inline\"] };\n}",
                3,
            ),
            (
                "adapter.rs",
                "fn claude_arguments() {\nvec![\"--cd\", \"inline positional prompt\"];\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn conformance_arguments() {\nformat!(\"inline {}\", label);\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn bypass() {\nprompt.append_argument(args);\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn send_identity() {\nprompt.append_argument(args);\nargs.push(\"inline\");\n}",
                1,
            ),
            (
                "prompt.rs",
                "impl RenderedPrompt {\nfn append_argument(self, argv: &mut Vec<String>) { argv.push(\"inline\".into()); }\n}",
                2,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nsend_identity(prompt, &mut args);\nargs = vec![\"inline\"];\n}",
                3,
            ),
            (
                "adapter.rs",
                "fn build_plan() {\nsend_identity(prompt, &mut args);\n(args)[0] = \"inline\";\n}",
                3,
            ),
            (
                "adapter.rs",
                "fn plan() {\nbuild_plan(config, binary, vec![\"inline\"], env, None);\n}",
                2,
            ),
        ] {
            let faults = violations(file, text);
            assert_eq!(faults.len(), 1, "{text}");
            assert_eq!(faults[0].rule, "prompt-literal");
            assert!(faults[0].file.ends_with(file));
            assert_eq!(faults[0].line, line, "{text}");
        }
    }
}
