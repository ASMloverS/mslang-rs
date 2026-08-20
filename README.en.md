# mslang

A scripting language blending Go-style syntax with Python dynamic features, implemented in Rust.

## Example

```ms
const GREETING = "Hello, mslang!"

fn greet(names) {
    result = []
    for name in names {
        result.push(GREETING + " " + name)
    }
    return result
}

for msg in greet(["Alice", "Bob", "Charlie"]) {
    print(msg)
}
```

## Features

| Category | Features |
|---|---|
| Types | Dynamic — int, float, bool, string, nil, list, dict, tuple, set |
| Functions | First-class, closures, anonymous, default/variadic params, multiple returns |
| Control flow | if/elif/else, while, for..in, break/continue, ternary expression |
| Advanced | List comprehension, slicing, generators/yield, decorators, with, defer |
| OOP | Python-style class, single inheritance, magic methods, operator overloading |
| Error handling | try/except/finally, throw, defer |
| Concurrency | async/await, go coroutines, channels |
| Modules | import, from...import, import as |

## Execution Model

```
Source (.ms) → Lexer → Token → Parser → AST → Compiler → Bytecode → VM
```

- Bytecode compilation + stack-based virtual machine
- Reference counting + mark-sweep GC
- Brace-delimited blocks, no semicolons, `#` line comments
- Script mode — top-down execution from file

## CLI

```
ms run script.ms      Run a script
ms eval "1 + 2"       Evaluate an expression
ms repl               Interactive REPL
ms check script.ms    Syntax check
ms version            Print version
```

## Build

```
cargo build
cargo test
```

## Tests

The `.ms` corpus lives under `tests/ms/` (core / functions / advanced / oop /
errors / stdlib / concurrency / modules / negative), executed one by one via
`ms run` by the `tests/ms_corpus.rs` harness:

```
cargo test --test ms_corpus
```

- Regular cases: must pass (exit 0); cases with a `.expected` sidecar have
  stdout compared
- `negative/` cases: expected to fail, stderr must contain the substrings
  from the `.expected` file
- Demo scripts live in `examples/`

## Documentation

| Doc | Content |
|---|---|
| [00-overview](docs/mslang/00-overview.md) | Language overview |
| [01-lexical](docs/mslang/01-lexical.md) | Lexical specification |
| [02-types](docs/mslang/02-types.md) | Type system |
| [03-syntax](docs/mslang/03-syntax.md) | Syntax specification |
| [04-functions](docs/mslang/04-functions.md) | Function system |
| [05-control-flow](docs/mslang/05-control-flow.md) | Control flow |
| [06-oop](docs/mslang/06-oop.md) | Object-oriented programming |
| [07-advanced](docs/mslang/07-advanced.md) | Advanced features |
| [08-concurrency](docs/mslang/08-concurrency.md) | Concurrency model |
| [09-modules](docs/mslang/09-modules.md) | Module system |
| [10-builtins](docs/mslang/10-builtins.md) | Built-in functions & stdlib |
| [11-bytecode-vm](docs/mslang/11-bytecode-vm.md) | Bytecode & VM design |
| [12-implementation-plan](docs/mslang/12-implementation-plan.md) | Implementation plan |
| [Task Index](docs/mslang/tasks/README.md) | 58 implementation tasks |

## License

MIT
