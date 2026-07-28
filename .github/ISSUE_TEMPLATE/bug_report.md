---
name: Bug report
about: Something in ciac itself (a compiler, backend, or CLI behavior) is wrong
title: ""
labels: bug
assignees: ""
---

**What happened**
A clear description of the incorrect behavior.

**`.ciac` source that reproduces it**

```ciac
// the smallest program that shows the problem
```

**Command run**

```sh
ciac ...
```

**Expected vs actual**
What you expected `ciac` to do, and what it did instead (paste the
exact output, or the generated code that's wrong).

**Environment**
- `ciac --version` output:
- OS:
- Target(s) affected (python/rust/typescript/go/java, if relevant):

**Found during a dogfooding session?**
If this came out of a [`DOGFOODING.md`](../../DOGFOODING.md) session,
add the `dogfooding` label and link the session's feedback log entry.
