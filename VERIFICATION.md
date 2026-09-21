# SDK-523 parser verification

Verified against SDK revision `400e92f00afb524371198bedd4f9cddfcab48f78`.

- 98 focused upstream inputs agree on normalized trees, canonical bytes, repair categories,
  source lines, and error locations. These inputs are frozen in the game-free test suite.
- 2,058 installed Stellaris `common/**/*.txt` inputs agree with the TypeScript parser.
- The upstream full-corpus fixpoint and independent jomini differential both pass. The pinned
  differential keeps its explicit known jomini limitations; no new exclusions were introduced.
- Rust tests cover constructors, raw conditional regions, traversal, CWT annotations and errors,
  generated trees/text, numeric precision, and the 1,000-body nesting boundary.
- Formatting, all Rust tests, and Clippy with warnings denied pass.

The installed corpus was read from a local Stellaris installation.
This is syntax evidence, not game execution or a claim of correctness for every Clausewitz game.
Use the commands in README.md to reproduce the comparisons; no game files are included in Git.
