# pdxscript

A standalone Rust library for PDXScript game-definition files and CWTools config syntax.
The crate has no game semantics, installation lookup, or file I/O. Callers provide decoded UTF-8
strings. It preserves item order and duplicate keys. It does not parse save files.

```rust
use pdxscript::script::{parse, serialize};

let document = parse("cost = 9007199254740993\ncost = 2", "example.txt")?;
assert!(document.diagnostics.is_empty());
let canonical = serialize(&document.items)?;
# Ok::<(), pdxscript::SyntaxError>(())
```

`script` provides owned syntax trees, checked constructors, canonical serialization, traversal,
conditional-region readers, and textual numeral helpers. Repairs (stray closing braces, missing
closing braces, and documented missing-assignment cases) are returned as diagnostics. Other syntax
errors return `SyntaxError`; nesting failures have their own `ErrorKind`. Numerals retain exact
canonical decimal digits. Arithmetic projection to `f64` is explicit and checked.

Serialization promises semantic round trips, not byte preservation. Comments and whitespace are
normally discarded; verbatim conditional regions preserve their body exactly. `without_spans`
removes location metadata for comparison. `walk_items` visits in pre-order with explicit child
context, skip/stop controls, and a policy for verbatim regions. `item_children` returns owned copies.

```rust
let document = pdxscript::cwt::parse("## cardinality = 0..1\nx = <tradition>", "rules.cwt");
assert!(document.diagnostics.is_empty());
assert_eq!(document.nodes[0].annotations[0].text, "cardinality = 0..1");
```

`cwt` preserves raw scalar spellings, `=`/`==`, bare values, nested blocks, and `##`, `###`, and
`####` annotations. It returns partial nodes plus explicit diagnostics, with no script repairs.
Consumers must surface diagnostics. Ownership of claims, annotation interpretation, and game
knowledge belong to consumers such as Atlas. CWT writing is not implemented.

Both modules share private source scanning and cursor operations. Their token boundaries, scalar
interpretation, and repair rules remain independent. Source spans use half-open UTF-8 byte offsets
and one-based lines. A file-leading BOM is removed only at the document boundary. The shared nesting
limit is 1,000 bodies; parsers and script serialization use explicit stacks.

## Provenance and port choices

Ported from `@pdx-ts/pdxscript` and the SDK's CWT reader at
`yeager-j/pdx-ts-sdk` source revision `400e92f00afb524371198bedd4f9cddfcab48f78`
(local source: `pdx-sdk/packages/pdxscript` and `packages/codegen-cwt/src/cwt`).
Original copyright and MIT terms are in LICENSE. GRAMMAR.md preserves the upstream grammar.

Rust uses enums, `Result`, owned strings, typed operators, and separate `scalar`, `boolean`, and
`numeral` constructors. All script items carry optional spans rather than only entry line numbers.
The CWT reader additionally retains four-hash documentation, escaped quotes, and end spans; it
reports incomplete input without silently repairing it. The PDXScript serializer remains compatible
with the pinned TypeScript canonical output. UTF-16-only unpaired surrogates are outside Rust `str`.

## Checks

```
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --example conformance --locked
PDX_SDK_PATH=/path/to/pdx-sdk node tools/typescript-conformance.mjs
STELLARIS_PATH=/path/to/Stellaris PDX_SDK_PATH=/path/to/pdx-sdk node tools/typescript-conformance.mjs --vanilla
```

The game-free suite includes 98 frozen TypeScript grammar inputs, Rust API tests, and generated
properties. The optional development comparison requires Node and the pinned SDK checkout with its
dev dependencies. `--vanilla` compares both parsers across every shipped `common/**/*.txt` input,
then runs the pinned TypeScript corpus fixpoint and independent jomini differential with its named
differences. It fails when the installation is missing. No game launch is needed, and game files
are not committed. Updating fixtures requires an explicit `--write-fixtures` run and diff review.
