# PDXScript grammar, as parsed by @pdx-ts/pdxscript

The whole language, in EBNF-ish form. Terminals in caps; trivia (whitespace,
`\r`, semicolons, and `#` comments to end of line — vanilla gfx files end
statements with `;`, the format treats it as whitespace) can appear between
any two tokens and is dropped. A UTF-8 BOM opening a *file* is stripped —
it states that document's encoding. Elsewhere `U+FEFF` is ordinary text, so a
region body opening with one keeps it. A key holding one is written quoted,
since a quoted key round-trips and an entry records no quoting flag; a *value*
that would open a document with the mark is refused instead, because quoting
it would change the tree a reparse gives back.

```
file       = item* EOF               (a container body without braces)
entry      = KEY op value
op         = "=" | ">" | "<" | ">=" | "<=" | "!="
value      = SCALAR container        (header form: hsv { 0.63 0.13 0.5 })
           | SCALAR | MATH | container
container  = "{" item* "}"
region     = "[[" "!"? NAME "]" REGION-TEXT "]"   (conditional text; see below)
item       = entry | container | region | SCALAR | MATH

KEY        = unquoted token (`@name` keys define variables) | quoted string
             (kept as its text; re-emitted quoted when it cannot be bare)
SCALAR     = quoted string | unquoted token
MATH       = ("@[" | "@\[") any characters "]"   (verbatim, single token —
             the escaped form defers evaluation until after $PARAM$
             substitution in scripted effects)
```

The top level really is `item*`, not `entry*`: vanilla ships all-scalar files
(`job_tags/00_tags.txt` is a bare word list) and anonymous top-level
containers (`gamesetup_settings.txt`).

## Conditional regions

`[[POP_GROUP] ... ]` (negated `[[!POP_GROUP] ... ]`) appears throughout
Stellaris `common/scripted_effects`. It is not a block: the engine splices the
region's **text** into the call site when the parameter is defined, and parses
afterwards. Brace balance therefore need only hold *after* substitution, and
mods rely on that — Gigastructural Engineering opens `set_name = {` inside one
region and closes it inside another, so every define/undefine combination
balances and none of the three regions does on its own.

So a region's body is text, and it is read as a tree only when the text is
already a balanced item sequence:

- balanced (all 50 vanilla occurrences, 638 of Gigastructural's 650) — a
  `param` node, its body parsed like any other, `$NAME$` substitution tokens
  inside it ordinary unquoted scalars;
- not balanced, or readable only by repairing brace balance — a `param-text`
  node holding the body verbatim. `regionScalars()` reads such a body flat
  (the sanctioned way to ask what it names); there is nothing else to say
  about it, because the engine has not decided either.

Finding the region's closing `]` is textual for the same reason: `]` inside a
quoted string, a `#` comment, or `@[ ... ]` math does not close it, a nested
`[[NAME]` opener raises the depth, and braces are not counted at all.

## Token classification

An unquoted token becomes, in order of precedence:

| Test                        | Kind                           |
| --------------------------- | ------------------------------ |
| `yes` / `no`                | `bool`                         |
| `[+-]?(\d+(\.\d+)?\|\.\d+)` | `num` (vanilla writes `+0.10`) |
| leading `@`                 | `var` (an `@name` reference)   |
| anything else               | `str`                          |

A quoted token is always `str` (so `"yes"` is a string, not a bool).
Date-like tokens (`2200.01.01`) are strings — date semantics are the game's,
not the syntax's. `|`, `:`, `.`, `-`, `$`, and non-ASCII bytes stay inside
unquoted tokens (Stellaris script values:
`value:job_weights_research_modifier|JOB|head_researcher|`).

A `num` carries its **lexeme**, not a JS `number`. A double cannot hold every
numeral a game file may contain — `9007199254740993` becomes `…992`, a
400-digit integer becomes `Infinity` — and a JS number has no PDXScript
spelling above `1e21` or below `1e-7`. So the digits are kept as written,
minus the spellings that say nothing: a leading `+`, leading and trailing
zeros, and `-0` are canonicalized away *textually*, never by round-tripping
through `Number`. `+0.10` and `0.100` are both `0.1`; `9007199254740993`
is itself. `numberValue()` / `tryNumberValue()` are the projection for
consumers doing arithmetic, and they refuse rather than round.

Quoted strings: no escape *processing*, but `\` skips the next character when
scanning for the closing quote, so `"Joe \"Captain\" Rogers"` is one token.
Content is kept raw and re-emitted verbatim — decoding escapes is a consumer
concern. Quotes may span lines and contain arbitrary bytes.

## Disambiguation rules

- **Entry vs bare item inside a container:** a token followed by an operator
  starts an entry; otherwise it is a bare item. This is what makes
  `prerequisites = { tech_stingers OR = { ... } }` parse: `tech_stingers` is
  a bare scalar item, `OR = { ... }` an entry item, one container holds both.
  Bare items may be quoted (`paradoxplaza_store_url ""` — shipped Stellaris
  DLC metadata with a missing `=`).
- **Header containers:** at *value position*, a scalar followed by `{` on
  the *same line* forms one headed container:
  `color = hsv { 0.63 0.13 0.5 }`. The rule is positional and open-ended
  (`rgb` incl. 4-component, `hsv360`, `hex`, `LIST`, `cylindrical`, ...) —
  jomini's rule plus the same-line constraint, which keeps a scalar value
  followed by a bare container item on the next line unambiguous (property
  testing found the fusion). `name = rgb` followed by anything but a
  same-line `{` is the plain scalar `rgb`. At *item position* a scalar then
  a container stays two bare items.
- **`@[` vs `@name`:** `@` followed by `[` opens an inline-math token that
  runs verbatim to the first `]`. Any other `@` token is a `var` scalar (or a
  variable-defining key, when in key position).

## Malformed-but-shipped input: repair with diagnostics

Paradox ships files the game repairs rather than rejects. The parser does
the same — parse succeeds, and each repair is recorded as a diagnostic
(never silent; strict callers fail on any diagnostic):

- **Stray closing brace** (`color = { 121 163 114 } } army_names = ...`,
  EU4 `verona.txt`): skipped.
- **Unclosed containers at EOF** (Stellaris ships one:
  `scripted_loc_ruloc.txt` is missing its final `}`): auto-closed, reported
  with the opening line.
- **Operator-less entry on one line** (Stellaris ships these at top level and nested;
  nested repair requires an entry-shaped body so `rgb { 1 2 3 }` stays a
  bare scalar plus scalar container:
  `named_colors/01_trait_colors.txt` line 65, `trait_bg_active_glow {`, and
  `interface/dlc_icons.gfx` line 504, `spriteType {`):
  read as `foo = { ... }`.

Hard errors (`PdxSyntaxError`, always `file:line`): unterminated quote,
unterminated `@[`, unterminated `[[` region, a `]` with no opener, an
operator with no key or no value, and nesting beyond the depth guard
(fuzz-proofing — 100k open braces must error, not blow the stack).

The depth guard is `MAX_NESTING_DEPTH`, and it bounds the language rather
than only the parser: `serialize` refuses a tree nested deeper than a parse
of its output could read, so a constructed tree cannot leave the set the
parser accepts. It also caps the output, since one tab per level per line
grows with the square of the depth.

An unreadable region body is *not* an error — it is what `param-text` is
for. The distinction is that the region's own delimiters are the file's
structure, while what sits between them is the call site's.

That fallback is for syntax alone. `param-text` is a claim that the body is
not a balanced item sequence, so only a syntax failure may produce one: a
body that reaches the depth guard is refused like any other, and a defect in
this package is raised rather than dressed up as a region with no tree.

## Serialization

One canonical rendering; the round-trip claim is semantic (identical tree),
not textual (identical bytes).

- Containers: all-scalar items render inline (`{ a b c }`); anything else
  renders one item per line, tab-indented; empty renders `{}`. Headers render
  before the brace. Empty anonymous containers are preserved (jomini drops
  them — a documented differential divergence).
- Scalars: bools as `yes`/`no`; numbers as their lexeme (`2.0` renders `2`,
  `+0.10` renders `0.1` — the canonicalization happened at parse, and every
  other digit is the file's); `var` as the bare `@name`; `math` verbatim.
- **Bare-vs-quoted is symmetric with the lexer**: a `str` renders bare only
  if re-lexing it yields that same single `str` token — one negative
  character class (no whitespace or `" # ; < = > { } ! [ ]`), and never text
  that would reclassify as bool/num/var/math (`"yes"`, `"123"`, `"@x"` render
  quoted). Explicitly quoted strings stay quoted.
- Top-level entries are blank-line separated, with a trailing newline.
- Keys render bare where the character class allows and quoted otherwise
  (`"two words" = 1`). A key is never *classified*, so `yes` and `123` are
  keys like any other.
- A `param` region renders like a container, one item per line with the
  closing `]` at parent indent. A `param-text` region renders its body
  byte-for-byte between the opener and `]`, with nothing added — comments
  and all, since re-indenting text this package did not read would be an
  edit, and anything inserted would change what the next parse captures.

  That body is the one field written from raw text rather than from a checked
  value, so it is the one that could emit something reading back as a
  different node. A body that balances comes back as a `param`; a body holding
  a `]` the region scan does not skip closes the region early and spills the
  rest into the document. `paramText()` refuses both, with the reason, and the
  serializer refuses them again for trees built as object literals. A body the
  parser produced always passes: the region scan is deterministic, so what it
  captured it captures again.

Consequences, all fixpoint-stable: comments, blank lines, and semicolons
drop; `2.0` renders `2`; `.5` renders `0.5`; multi-line scalar lists render
inline; repaired input re-emits in repaired form.

## Representability, and being closed under it

One rule per question — is this a legal numeral, quoted content, parameter
name, `@name`, inline-math token, bare token? — shared by the lexer, the
parser, the public constructors and the serializer, so that the three sets
below are the same set:

- what the parser can produce,
- what the constructors can build,
- what the serializer can write.

The consequence worth stating: a value that cannot be written is refused
where it is *built*, not where it is emitted. `scalar('say "hi"')`,
`quoted("ends in a backslash \\")`, `inlineMath("hello")` and
`varRef("no_at_sign")` all throw at construction, because each would
otherwise emit text that reads back as a different node — or as a syntax
error in the middle of the next file. A serializer throw is the backstop for
a tree assembled as a bare object literal, not the primary check.

The property suite tests exactly this closure, over generated trees and over
generated constructor arguments.

## Deferred, by name

- **`?=` and `==`**: not Stellaris operators (CK3/Vic3/Imperator territory).
  Both are hard errors today; adding them is a lexer and AST change when a
  sibling game is in scope.
- **Save-file constructs** (`EU4txt` magic, container-as-key templates):
  out of scope — this package parses game-definition files. (Conditional
  regions were originally scoped out as EU4-only; the vanilla sweep found
  them all over Stellaris `common/scripted_effects`, so they are in.)
- **Substitution**: `$NAME$` tokens and the regions guarded by them are
  represented, never applied. This package cannot tell you what a scripted
  effect looks like at a given call site, which is why an unbalanced region
  body stays text rather than being reassembled into a tree.
- **Comment preservation**: out of scope for 0.x; the AST leaves room
  (trivia would attach to entries) but no API pretends it exists.
- **Encodings**: the API takes a decoded string; callers own file reading.
  Stellaris is UTF-8(-BOM). Older games' Windows-1252 is a caller concern
  (the lexer itself is 8-bit clean).
- **Semantics of `var` and `math`**: representation only. Resolution order,
  scripted-variable scoping, and math evaluation are game knowledge and live
  in consumers (the SDK resolves variables; this package never does).
