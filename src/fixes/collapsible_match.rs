//! Collapse an `if let` that forms the whole of a match arm into the arm's own
//! pattern — the transformation `clippy::collapsible_match` describes but ships
//! no suggestion for.
//!
//! ```ignore
//! Event::ShmFormat { format } => {          Event::ShmFormat { format: WEnum::Value(f) } => {
//!     if let WEnum::Value(f) = format {  =>     state.shm_formats.push(f as u32);
//!         state.shm_formats.push(f as u32); }
//!     }
//! }
//! ```
//!
//! Clippy declines to emit a replacement because the rewrite is not
//! unconditionally safe: it changes which arm a value matches. Every `return
//! None` below is one of the cases it was right to worry about.

use ra_ap_syntax::algo::find_node_at_offset;
use ra_ap_syntax::ast::{self, AstNode, HasName};
use ra_ap_syntax::{SourceFile, TextSize};

use super::Fix;

pub fn fix(file: &SourceFile, offset: TextSize) -> Option<Fix> {
    let arm = find_node_at_offset::<ast::MatchArm>(file.syntax(), offset)?;

    // A guard is evaluated against the binding we are about to dissolve.
    if arm.guard().is_some() {
        return None;
    }

    let outer_pat = arm.pat()?;
    let ast::Expr::BlockExpr(arm_block) = arm.expr()? else {
        return None;
    };

    let inner_if = sole_if_expr(&arm_block)?;

    // An `else` runs when the inner pattern fails. Collapsing makes the arm stop
    // matching instead, so control would fall to a later arm — not the same thing.
    if inner_if.else_branch().is_some() {
        return None;
    }

    let ast::Expr::LetExpr(let_expr) = inner_if.condition()? else {
        return None;
    };
    let inner_pat = let_expr.pat()?;
    let bound = plain_path_name(&let_expr.expr()?)?;
    let then_block = inner_if.then_branch()?;

    // The binding has to be a bare `name`. `name @ subpat`, `ref name` and
    // `mut name` each mean something the collapsed form would not preserve.
    let ident = outer_pat
        .syntax()
        .descendants()
        .filter_map(ast::IdentPat::cast)
        .find(|p| p.name().is_some_and(|n| n.text() == bound))?;
    if ident.pat().is_some() || ident.ref_token().is_some() || ident.mut_token().is_some() {
        return None;
    }

    // Collapsing consumes the binding, so any surviving reference to it would
    // dangle. Conservative on purpose: decline if the name appears at all,
    // rather than trying to distinguish uses from shadowing rebinds.
    if then_block
        .syntax()
        .descendants()
        .filter_map(ast::NameRef::cast)
        .any(|n| n.text() == bound)
    {
        return None;
    }

    let inner_text = inner_pat.syntax().text().to_string();

    // A record-pattern shorthand `Foo { name }` must become `Foo { name: PAT }`;
    // everywhere else the binding is replaced outright.
    let (pat_range, pat_text) = match ident.syntax().parent().and_then(ast::RecordPatField::cast) {
        Some(field) if field.name_ref().is_none() => {
            (field.syntax().text_range(), format!("{bound}: {inner_text}"))
        }
        _ => (ident.syntax().text_range(), inner_text),
    };

    Some(Fix {
        title: "Collapse `if let` into the match arm pattern".to_owned(),
        edits: vec![
            (pat_range, pat_text),
            (
                arm_block.syntax().text_range(),
                dedent(&then_block.syntax().text().to_string()),
            ),
        ],
    })
}

/// The single `if` expression forming the whole of a block, written either as
/// the tail expression or as the only statement. Anything else in the block
/// would be discarded by the rewrite.
fn sole_if_expr(block: &ast::BlockExpr) -> Option<ast::IfExpr> {
    let list = block.stmt_list()?;
    let mut statements = list.statements();
    match (statements.next(), statements.next(), list.tail_expr()) {
        (None, _, Some(ast::Expr::IfExpr(e))) => Some(e),
        (Some(ast::Stmt::ExprStmt(s)), None, None) => match s.expr()? {
            ast::Expr::IfExpr(e) => Some(e),
            _ => None,
        },
        _ => None,
    }
}

/// The name of an unqualified single-segment path expression, if that is what
/// this is. `Some(x)`, `self.field` and `foo::BAR` all yield `None`.
fn plain_path_name(expr: &ast::Expr) -> Option<String> {
    let ast::Expr::PathExpr(p) = expr else {
        return None;
    };
    let path = p.path()?;
    if path.qualifier().is_some() {
        return None;
    }
    Some(path.segment()?.name_ref()?.text().to_string())
}

/// Remove one indent level from every line but the first. The inner block was
/// nested one deeper than the arm body replacing it.
fn dedent(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if i == 0 {
            out.push_str(line);
        } else {
            out.push_str(line.strip_prefix("    ").unwrap_or(line));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ra_ap_syntax::Edition;

    /// Applies the fix at the `$0` marker, or returns None if it declines.
    fn apply(marked: &str) -> Option<String> {
        let offset = TextSize::new(marked.find("$0").expect("no $0 marker") as u32);
        let text = marked.replace("$0", "");
        let parse = SourceFile::parse(&text, Edition::CURRENT);
        let fix = fix(&parse.tree(), offset)?;

        let mut edits = fix.edits;
        edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start()));
        let mut out = text;
        for (range, new) in edits {
            out.replace_range(usize::from(range.start())..usize::from(range.end()), &new);
        }
        Some(out)
    }

    fn declines(marked: &str) {
        assert_eq!(apply(marked), None, "expected the fix to decline");
    }

    #[test]
    fn record_pattern_shorthand() {
        // The shape that prompted all this, from MinkaCap's Dispatch impl.
        let got = apply(
            r#"
fn f(e: Event) {
    match e {
        Event::ShmFormat { format } => {
            if let WEnum::Value(v) = $0format {
                sink(v as u32);
            }
        }
        _ => {}
    }
}
"#,
        )
        .expect("should apply");
        assert!(
            got.contains("Event::ShmFormat { format: WEnum::Value(v) } =>"),
            "pattern not collapsed:\n{got}"
        );
        assert!(!got.contains("if let"), "if let not removed:\n{got}");
        assert!(got.contains("sink(v as u32);"), "body lost:\n{got}");
    }

    #[test]
    fn tuple_struct_pattern() {
        let got = apply(
            r#"
fn f(x: Option<Inner>) {
    match x {
        Some(inner) => {
            if let Inner::A(n) = $0inner {
                sink(n);
            }
        }
        None => {}
    }
}
"#,
        )
        .expect("should apply");
        assert!(got.contains("Some(Inner::A(n)) =>"), "not collapsed:\n{got}");
    }

    #[test]
    fn declines_with_else() {
        // `else` runs on inner-pattern failure; collapsing would fall through to
        // a later arm instead, which is a different program.
        declines(
            r#"
fn f(x: Option<Inner>) {
    match x {
        Some(inner) => {
            if let Inner::A(n) = $0inner {
                sink(n);
            } else {
                other();
            }
        }
        None => {}
    }
}
"#,
        );
    }

    #[test]
    fn declines_when_binding_used_in_body() {
        declines(
            r#"
fn f(x: Option<Inner>) {
    match x {
        Some(inner) => {
            if let Inner::A(n) = $0inner {
                sink(n, inner);
            }
        }
        None => {}
    }
}
"#,
        );
    }

    #[test]
    fn declines_with_extra_statements() {
        declines(
            r#"
fn f(x: Option<Inner>) {
    match x {
        Some(inner) => {
            log();
            if let Inner::A(n) = $0inner {
                sink(n);
            }
        }
        None => {}
    }
}
"#,
        );
    }

    #[test]
    fn declines_with_guard() {
        declines(
            r#"
fn f(x: Option<Inner>) {
    match x {
        Some(inner) if ready() => {
            if let Inner::A(n) = $0inner {
                sink(n);
            }
        }
        None => {}
    }
}
"#,
        );
    }

    #[test]
    fn declines_on_ref_binding() {
        declines(
            r#"
fn f(x: Option<Inner>) {
    match x {
        Some(ref inner) => {
            if let Inner::A(n) = $0inner {
                sink(n);
            }
        }
        None => {}
    }
}
"#,
        );
    }

    #[test]
    fn declines_when_scrutinee_is_not_the_binding() {
        // Nothing to fold into the pattern if the `if let` tests something else.
        declines(
            r#"
fn f(x: Option<Inner>) {
    match x {
        Some(inner) => {
            if let Inner::A(n) = $0other() {
                sink(n);
            }
        }
        None => {}
    }
}
"#,
        );
    }
}
