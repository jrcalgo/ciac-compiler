# ciac

The CIaC compiler: one `.ciac` source file compiles to five
production-quality backends — Python, Rust, TypeScript, Go, Java —
generated at parity, each idiomatic in its own ecosystem, byte-identical
on every rebuild. The same file runs as a deterministic simulation of
the whole system, failures and all, with no database, broker, or
Docker. `ciac build` emits a real deploy: compose today, Kubernetes or
Terraform or CI on request.

```sh
cargo install ciac
ciac new my-app && cd my-app && ciac check main.ciac
```

Full documentation, guides, and examples: <https://github.com/jrcalgo/ciac-compiler>.
