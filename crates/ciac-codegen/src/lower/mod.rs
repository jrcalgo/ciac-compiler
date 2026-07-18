//! Backend-shared HIR lowering (`22UpdatePlan.md` Pillar 3, all three
//! parts):
//!
//! - `scan.rs` (Part 1, v0.22 M3) — [`Needs`]/[`scan`]/
//!   [`field_access_enum_name`]: the one traversal answering "what
//!   does this handler body touch?", computed once so a backend can't
//!   independently drift on which verbs it actually scans (the
//!   correctness argument for `ciac sim`'s `unguarded_verbs` refusal).
//! - `dispatch.rs` (Parts 2-3, this continuation) — the shared
//!   statement/expression walker: block/tail shaping, precedence,
//!   enum-literal use-site recovery, float-literal fidelity,
//!   divergence truncation. [`lower_body_expr`]/[`lower_body_stmt`]
//!   are the two entry points a backend's own `render()` calls.
//! - `host_syntax.rs` (Part 3) — the [`HostSyntax`] trait: roughly 50
//!   leaf constructor methods a target implements against the walker
//!   above. `ciac-backend-python`'s `PySyntax` and
//!   `ciac-backend-rust`'s `RustSyntax` are today's two
//!   implementations; a third backend's own lowering is "implement
//!   this trait," not "write a second walker."
//! - `identity.rs` (Part 3) — [`IdentitySyntax`]/
//!   [`IdentitySyntaxStatement`]: the contract's own always-compiled,
//!   never-registered reference implementation, proven from both
//!   orientations against the real example corpus in
//!   `tests/tests/host_syntax_identity.rs` — so the *contract* has a
//!   golden, not just its two consumers.
//!
//! `model.rs` is target-neutral by test, not convention: this module
//! is the other half of that claim for handler-body lowering — every
//! HIR shape's *walk* is defined exactly once, and a backend supplies
//! only the leaf spelling its own language needs.

mod dispatch;
mod host_syntax;
mod identity;
mod scan;

pub use dispatch::{
    apply_dest, fidelity_checked_float, indent_lines, lower_block_expr, lower_block_stmt,
    lower_body_expr, lower_body_stmt, lower_expr_any, lower_scalar, lower_tail, strip_outer_parens,
    Dest, Wrap,
};
pub use host_syntax::{
    HostSyntax, IndexKey, LoweredPredTerm, LoweredPredicate, MatchArm, Orientation, PredValue,
};
pub use identity::{IdentitySyntax, IdentitySyntaxStatement};
pub use scan::{field_access_enum_name, scan, Needs};
