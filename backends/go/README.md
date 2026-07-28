# ciac-backend-go-external-demo

A real, standalone external backend for [ciac](https://github.com/jrcalgo/ciac)
— v0.8 external-backend protocol M3. This directory is a self-contained
Go module, deliberately **not** listed in the root `Cargo.toml`'s
`[workspace] members`: it has no Rust dependency at build time or
runtime, and is invoked purely as a subprocess speaking JSON over
stdin/stdout, exactly like any other `ciac-backend-<target>`
executable `ciac_codegen::external::ExternalBackend` finds on `$PATH`.

**Renamed at `24UpdatePlan.md` M1**, from `ciac-backend-go`/`--target
go`: that id now reaches the real, bundled Go parity backend
(`crates/ciac-backend-go`), a different, much more capable
implementation living in the internal registry. This directory is
unchanged in every other respect — same scope, same protocol, same
worked example for third-party backend authors — see
[docs/external-backends.md](../../docs/external-backends.md) for the
full disclosure of why two Go code generators exist in this repo and
how they relate.

## Build

```sh
go build -o ciac-backend-go-external-demo .
```

## Scope

Deliberately narrow, matching the v0.8 M6 Go backend spike
(`docs/history/backend-spike-report.md` in the ciac repo): one service, one
record, one api, no capabilities. A request for anything wider (auth,
db, cache, queue, object store, email, search, external HTTP, or typed
handlers) is refused on stderr with a non-zero exit rather than
attempted.

## Try it

```sh
# from the ciac repo root
go build -C backends/go -o /tmp/ciac-backend-go-external-demo .
PATH="/tmp:$PATH" cargo run -q -p ciac -- build --target go-external-demo -o /tmp/ping-out examples/ping.ciac
cd /tmp/ping-out && go build -o ping . && ./ping &
curl -s localhost:8000/health
curl -s -X POST localhost:8000/echo -d '{"id":"abc","text":"hello"}'
```
