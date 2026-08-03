# CIaC Examples

The example corpus is split by deployment shape:

- `single-service/` contains programs that compile to one deployable service.
- `multi-service/` contains `project` programs with two or more service blocks.

Only immediate `.ciac` files in those two directories are corpus entrypoints.
Nested `.ciac` files are imported fragments for modular examples and should not
be compiled on their own.
