# ciac-ir

Typed system-graph intermediate representation for the CIaC compiler.

This is an internal crate of the [ciac](https://github.com/jrcalgo/ciac-compiler)
compiler. Its public API tracks the compiler's own version and is not
independently semver-stable — it may change in any compiler release,
including patch releases, as the compiler's internal architecture
evolves.

If you're authoring an out-of-tree code-generation backend, see
[`docs/external-backends.md`](https://github.com/jrcalgo/ciac-compiler/blob/main/docs/external-backends.md)
in the main repository — this crate is one of the two this IR-consuming
integration path actually depends on.
