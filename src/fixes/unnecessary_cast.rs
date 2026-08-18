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

use ra_ap_syntax::algo::find_node_at_offset;
use ra_ap_syntax::ast::{self, AstNode};
use ra_ap_syntax::{SourceFile, TextSize};

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

    let keep = operand.syntax().text().to_string();

    // A cast to the type the operand was *just* cast to. The inner cast already
    // produced this type, so the outer one is a no-op whatever the operand
    // underneath is. Parentheses are preserved rather than unwrapped: the
    // replacement keeps the operand's own text, so precedence cannot shift.
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
        let got = apply("fn f(v: u8) -> u32 { (v as u32) as $0u32 }").expect("should apply");
        assert_eq!(got, "fn f(v: u8) -> u32 { (v as u32) }");
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
    fn declines_when_not_on_a_cast() {
        declines("fn f(v: u8) -> u8 { $0v }");
    }
}
