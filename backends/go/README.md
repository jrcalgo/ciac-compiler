# ciac-backend-go

A real, standalone external backend for [ciac](https://github.com/jrcalgo/ciac)
— v0.8 external-backend protocol M3. This directory is a self-contained
Go module, deliberately **not** listed in the root `Cargo.toml`'s
`[workspace] members`: it has no Rust dependency at build time or
runtime, and is invoked purely as a subprocess speaking JSON over
stdin/stdout, exactly like any other `ciac-backend-<target>`
executable `ciac_codegen::external::ExternalBackend` finds on `$PATH`.

## Build

```sh
go build -o ciac-backend-go .
```

## Scope

Deliberately narrow, matching the v0.8 M6 Go backend spike
(`docs/backend-spike-report.md` in the ciac repo): one service, one
record, one api, no capabilities. A request for anything wider (auth,
db, cache, queue, object store, email, search, external HTTP, or typed
handlers) is refused on stderr with a non-zero exit rather than
attempted.

## Try it

```sh
# from the ciac repo root
go build -C backends/go -o /tmp/ciac-backend-go .
PATH="/tmp:$PATH" cargo run -q -p ciac -- build --target go -o /tmp/ping-out examples/ping.ciac
cd /tmp/ping-out && go build -o ping . && ./ping &
curl -s localhost:8000/health
curl -s -X POST localhost:8000/echo -d '{"id":"abc","text":"hello"}'
```
