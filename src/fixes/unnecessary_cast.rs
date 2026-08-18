//! Remove an `as` cast that provably does nothing — the transformation
//! `clippy::unnecessary_cast` names, offered at the cursor rather than behind a
//! full type-check.
//!
//! ```ignore
//! let n = value as u32 as u32;   =>   let n = value as u32;
//! let m = 0u32 as u32;           =>   let m = 0u32;
//! ```
//!
//! Clippy reaches this lint with the full type table, so it can say "the
//! operand is already `u32`" about any expression. Bartizan is built on
//! `ra_ap_syntax`, a lossless *syntax* tree with no type information at all, so
//! that general judgement is unavailable here and guessing at it would be how
//! you delete a truncating cast. What remains is the subset the source text
//! settles on its own: a cast to the same type as the cast directly beneath it,
//! and a numeric literal already written with that exact suffix. Every other
//! shape is declined.
//!
//! Deliberately *not* offered: rewriting an unsuffixed `1 as u32` into `1u32`.
//! That one is squarely inside what clippy already emits a machine-applicable
//! suggestion for, and this server exists for the gaps.
//!
//! [`from_clippy`] lifts the type restriction from the other direction. When the
//! editor hands us a `clippy::unnecessary_cast` diagnostic it is passing on a
//! judgement made *with* the type table, so there is nothing left to prove and
//! the cast can be removed whatever the operand is. That is not a retreat from
//! declining — it is declining to guess when something that knows has already
//! answered.

use ra_ap_syntax::algo::find_node_at_offset;
use ra_ap_syntax::ast::{self, AstNode};
use ra_ap_syntax::{SourceFile, SyntaxKind, SyntaxNode, TextRange, TextSize};

use super::Fix;

pub fn fix(file: &SourceFile, offset: TextSize) -> Option<Fix> {
    let cast = find_node_at_offset::<ast::CastExpr>(file.syntax(), offset)?;
    let operand = cast.expr()?;
    let target = normalized_type(&cast.ty()?);

    // `_` is resolved from context, and the two placeholders in `x as _ as _`
    // need not resolve to the same type. Nothing textual can settle that.
    if target == "_" {
        return None;
    }

    let keep = unparen(operand.clone()).syntax().text().to_string();

    // A cast to the type the operand was *just* cast to. The inner cast already
    // produced this type, so the outer one is a no-op whatever the operand
    // underneath is.
    if let Some(inner) = as_cast(&operand) {
        return (normalized_type(&inner.ty()?) == target).then(|| Fix {
            title: format!("Remove redundant `as {target}`"),
            edits: vec![(cast.syntax().text_range(), keep)],
        });
    }

    // A numeric literal already carrying this exact suffix. `1u32 as u32` is the
    // literal itself; `1u8 as u32` is a real conversion and is left alone.
    if literal_suffix(&operand).is_some_and(|suffix| suffix == target) {
        return Some(Fix {
            title: format!("Remove redundant `as {target}`"),
            edits: vec![(cast.syntax().text_range(), keep)],
        });
    }

    None
}

/// Remove a cast clippy has already judged unnecessary.
///
/// `range` is the diagnostic's span, which clippy reports over the whole cast
/// expression. The operand replaces it verbatim; the only judgement left is
/// whether parentheses around that operand can go, which is a question about
/// precedence and so answerable from syntax alone.
pub fn from_clippy(file: &SourceFile, range: TextRange) -> Option<Fix> {
    let cast = file
        .syntax()
        .covering_element(range)
        .ancestors()
        .find_map(ast::CastExpr::cast)?;
    let operand = cast.expr()?;
    let ty = normalized_type(&cast.ty()?);

    let keep = if binds_at_least_as_tight_as_cast(&operand) {
        unparen(operand).syntax().text().to_string()
    } else {
        // A binary operator, range or closure binds looser than `as`, so the
        // parentheses the author wrote are load-bearing: `x * (a + b) as u32`
        // must not become `x * a + b`.
        operand.syntax().text().to_string()
    };

    Some(Fix {
        title: format!("Remove unnecessary `as {ty}`"),
        edits: vec![(cast.syntax().text_range(), keep)],
    })
}

/// Whether `expr` can stand unparenthesised where a cast stood.
///
/// A cast binds tighter than every binary operator, so only operands that bind
/// at least as tightly are safe to unwrap. Deliberately a whitelist: an
/// unrecognised expression is assumed to need its parentheses.
fn binds_at_least_as_tight_as_cast(expr: &ast::Expr) -> bool {
    let inner = match expr {
        ast::Expr::ParenExpr(p) => match p.expr() {
            Some(inner) => inner,
            None => return false,
        },
        other => other.clone(),
    };
    matches!(
        inner,
        ast::Expr::Literal(_)
            | ast::Expr::PathExpr(_)
            | ast::Expr::CallExpr(_)
            | ast::Expr::MethodCallExpr(_)
            | ast::Expr::FieldExpr(_)
            | ast::Expr::IndexExpr(_)
            | ast::Expr::TupleExpr(_)
            | ast::Expr::ArrayExpr(_)
            | ast::Expr::ParenExpr(_)
            | ast::Expr::MacroExpr(_)
            | ast::Expr::CastExpr(_)
            | ast::Expr::PrefixExpr(_)
            | ast::Expr::RefExpr(_)
            | ast::Expr::TryExpr(_)
            | ast::Expr::AwaitExpr(_)
    )
}

/// Drop parentheses wrapping the expression we keep.
///
/// Unconditionally safe in both accepted branches: what survives is either a
/// literal, which binds tighter than anything, or a cast replacing a cast,
/// which has identical precedence — so no surrounding context can reparse the
/// result differently. Parentheses holding a comment are left in place rather
/// than deleting something a person wrote.
fn unparen(expr: ast::Expr) -> ast::Expr {
    let ast::Expr::ParenExpr(paren) = &expr else {
        return expr;
    };
    if contains_comment(paren.syntax()) {
        return expr;
    }
    match paren.expr() {
        Some(inner) => unparen(inner),
        None => expr,
    }
}

fn contains_comment(node: &SyntaxNode) -> bool {
    node.descendants_with_tokens()
        .any(|element| element.kind() == SyntaxKind::COMMENT)
}

/// The operand as a cast, seeing through any parentheses around it.
fn as_cast(expr: &ast::Expr) -> Option<ast::CastExpr> {
    match expr {
        ast::Expr::CastExpr(c) => Some(c.clone()),
        ast::Expr::ParenExpr(p) => as_cast(&p.expr()?),
        _ => None,
    }
}

/// The suffix on a numeric literal, seeing through parentheses. `None` for every
/// other literal kind — a suffix is the only thing that pins a literal's type
/// without consulting inference.
fn literal_suffix(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::ParenExpr(p) => literal_suffix(&p.expr()?),
        ast::Expr::Literal(lit) => match lit.kind() {
            ast::LiteralKind::IntNumber(n) => n.suffix().map(str::to_owned),
            ast::LiteralKind::FloatNumber(n) => n.suffix().map(str::to_owned),
            _ => None,
        },
        _ => None,
    }
}

/// Type text with all whitespace removed, so `*const u8` and `*const  u8`
/// compare equal. Two types that differ only in whitespace are the same type;
/// anything else — a comment inside the type, a differing path — compares
/// unequal and the fix declines, which is the safe direction.
fn normalized_type(ty: &ast::Type) -> String {
    ty.syntax()
        .text()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ra_ap_syntax::Edition;

    /// Applies `from_clippy` over the span of `cast`, standing in for the
    /// diagnostic range the editor would report for that cast expression.
    fn apply_clippy(text: &str, cast: &str) -> Option<String> {
        let start = text.find(cast).expect("cast source not found in text");
        let range = TextRange::new(
            TextSize::new(start as u32),
            TextSize::new((start + cast.len()) as u32),
        );
        let parse = SourceFile::parse(text, Edition::CURRENT);
        let fix = from_clippy(&parse.tree(), range)?;

        let mut out = text.to_owned();
        let (range, new) = fix.edits.into_iter().next().expect("one edit");
        out.replace_range(usize::from(range.start())..usize::from(range.end()), &new);
        Some(out)
    }

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
    fn repeated_identical_cast() {
        let got = apply("fn f(v: u8) -> u32 { v as u32 as $0u32 }").expect("should apply");
        assert_eq!(got, "fn f(v: u8) -> u32 { v as u32 }");
    }

    #[test]
    fn repeated_identical_cast_through_parens() {
        // The parenthesised inner cast is still recognised, and the parens go
        // with the outer cast — what is left is a cast where a cast was, so
        // nothing around it can reparse.
        let got = apply("fn f(v: u8) -> u32 { (v as u32) as $0u32 }").expect("should apply");
        assert_eq!(got, "fn f(v: u8) -> u32 { v as u32 }");
    }

    #[test]
    fn strips_redundant_parens() {
        let got = apply("fn f(v: u8) -> u32 { ((v as u32)) as $0u32 }").expect("should apply");
        assert_eq!(got, "fn f(v: u8) -> u32 { v as u32 }");
    }

    #[test]
    fn strips_parens_around_a_literal() {
        let got = apply("fn f() -> u32 { (0u32) as $0u32 }").expect("should apply");
        assert_eq!(got, "fn f() -> u32 { 0u32 }");
    }

    #[test]
    fn keeps_parens_holding_a_comment() {
        // Still offered — only the paren stripping is withheld, so the comment
        // survives the rewrite.
        let got =
            apply("fn f(v: u8) -> u32 { (/* keep me */ v as u32) as $0u32 }").expect("applies");
        assert_eq!(got, "fn f(v: u8) -> u32 { (/* keep me */ v as u32) }");
    }

    #[test]
    fn repeated_pointer_cast() {
        let got = apply("fn f(p: *const u8) { let q = p as *const u8 as $0*const u8; }")
            .expect("applies");
        assert!(got.contains("let q = p as *const u8;"), "{got}");
    }

    #[test]
    fn whitespace_in_type_does_not_block_the_match() {
        let got = apply("fn f(p: *const u8) { let q = p as *const  u8 as $0*const u8; }")
            .expect("applies");
        assert!(got.contains("let q = p as *const  u8;"), "{got}");
    }

    #[test]
    fn literal_with_matching_suffix() {
        let got = apply("fn f() -> u32 { 0u32 as $0u32 }").expect("should apply");
        assert_eq!(got, "fn f() -> u32 { 0u32 }");
    }

    #[test]
    fn float_literal_with_matching_suffix() {
        let got = apply("fn f() -> f64 { 1.5f64 as $0f64 }").expect("should apply");
        assert_eq!(got, "fn f() -> f64 { 1.5f64 }");
    }

    #[test]
    fn declines_widening_cast_chain() {
        // `x as u32 as u64` is not `x as u64`: a signed operand sign-extends to
        // u32 first and then zero-extends, which is a different number.
        declines("fn f(v: i8) -> u64 { v as u32 as $0u64 }");
    }

    #[test]
    fn declines_literal_with_different_suffix() {
        declines("fn f() -> u32 { 1u8 as $0u32 }");
    }

    #[test]
    fn declines_unsuffixed_literal() {
        // Removing the cast would hand the literal back to inference, which can
        // pick a different type in a generic call. Clippy's own suggestion adds
        // the suffix instead; that case is deliberately left to it.
        declines("fn f() -> u32 { 1 as $0u32 }");
    }

    #[test]
    fn declines_plain_expression_cast() {
        // The whole point: without types there is no way to know `v` is already
        // a `u32`, so the general case is refused rather than guessed at.
        declines("fn f(v: u32) -> u32 { v as $0u32 }");
    }

    #[test]
    fn declines_inference_placeholder() {
        declines("fn f(v: u8) -> u32 { v as _ as $0_ }");
    }

    #[test]
    fn clippy_removes_a_cast_no_syntax_could_prove() {
        // The case `fix` refuses on purpose: only the type table knows `v` is
        // already a `u32`. With clippy saying so, there is nothing left to prove.
        let got = apply_clippy("fn f(v: u32) -> u32 { v as u32 }", "v as u32").expect("applies");
        assert_eq!(got, "fn f(v: u32) -> u32 { v }");
    }

    #[test]
    fn clippy_keeps_load_bearing_parens() {
        // `as` binds tighter than `*`, so the parentheses are doing real work:
        // dropping them would silently reassociate the arithmetic.
        let got = apply_clippy(
            "fn f(x: u32, a: u32, b: u32) -> u32 { x * (a + b) as u32 }",
            "(a + b) as u32",
        )
        .expect("applies");
        assert_eq!(got, "fn f(x: u32, a: u32, b: u32) -> u32 { x * (a + b) }");
    }

    #[test]
    fn clippy_strips_parens_that_are_not_load_bearing() {
        let got =
            apply_clippy("fn f(v: u32) -> u32 { (v) as u32 }", "(v) as u32").expect("applies");
        assert_eq!(got, "fn f(v: u32) -> u32 { v }");
    }

    #[test]
    fn clippy_removes_a_same_constness_pointer_cast() {
        // Clippy marks exactly this shape MaybeIncorrect, which is why it can be
        // missing from the editor's own quickfix list.
        let got = apply_clippy(
            "fn f(p: *const u8) -> *const u8 { p as *const u8 }",
            "p as *const u8",
        )
        .expect("applies");
        assert_eq!(got, "fn f(p: *const u8) -> *const u8 { p }");
    }

    #[test]
    fn clippy_range_not_covering_a_cast_declines() {
        assert_eq!(apply_clippy("fn f(v: u32) -> u32 { v }", "v }"), None);
    }

    #[test]
    fn declines_when_not_on_a_cast() {
        declines("fn f(v: u8) -> u8 { $0v }");
    }
}
