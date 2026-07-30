# ciac-sema

Name resolution, validation passes, and lowering to the CIaC IR.

This is an internal crate of the [ciac](https://github.com/jrcalgo/ciac-compiler)
compiler. Its public API tracks the compiler's own version and is not
independently semver-stable — it may change in any compiler release,
including patch releases, as the compiler's internal architecture
evolves.
