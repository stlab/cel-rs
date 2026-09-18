# Lexical conventions

The lexer tokenizes a stream of characters into tokens before the parser reads
them as expressions. It is built on Rust
[token trees](https://doc.rust-lang.org/proc_macro/enum.TokenTree.html) and
inherits many behaviors from the Rust
[lexical structure](https://doc.rust-lang.org/reference/lexical-structure.html).
This chapter describes the token forms that participate in CEL source. For the
full CEL expression grammar, see the [Reference Manual](reference.md).

## Syntax

```text
compound_token = "&&" | "||" | "==" | "!=" | "<=" | ">="
               | "<<" | ">>" | ".." | "..=" .
identifier = identifier_start { identifier_continue } .
```

## Whitespace and Compound tokens

Spaces, tabs, and line breaks separate tokens. They do not otherwise change the
meaning of an expression.

```text
x + 1
x+1
if flag {
    1
} else {
    2
}
```

Certain punctuation characters combine to form single tokens so long as there is no intervening white space. This includes, `&&`, `||`,
`==`, `!=`, `<=`, `>=`, `<<`, `>>`, `..`, and `..=`.

```text
a&&b
x<=y
1..=5
```

## Comments and trivia

Comments are source trivia. They may appear between tokens in the same places
as whitespace.

```text
x /* midpoint */ + 1
if ready { 1 } // done
else { 0 }
```

Line comments use `// ...`. Block comments use `/* ... */`.

## Identifiers and reserved words

An identifier is a non-empty name made from Unicode identifier characters.
The first character may be a Unicode letter or `_`; later characters may also
include Unicode digits. CEL follows the Rust
[identifier rules](https://doc.rust-lang.org/reference/identifiers.html) for
the spellings that its lexer accepts.

```text
x
user_name
value2
π
_temporary
```

These words are reserved for fixed roles in CEL source and cannot name
user-defined values:

- `if`
- `else`
- `as`
- `true`
- `false`

`true` and `false` are boolean literals. `if` and `else` introduce conditional
expressions. `as` introduces an explicit cast.

## Literal tokens

The lexer recognizes these literal token categories:

- integer literals
- floating-point literals
- string literals
- boolean literals
- character literals
- byte literals
- byte-string literals
- C-string literals

```text
42
3.5
"hello"
true
'a'
b'A'
b"bytes"
c"header"
```

The grammar describes the unit value `()` rather than a standalone literal
token.

## Delimiters and punctuation

CEL uses three delimiter pairs:

- `(` `)` for grouping, unit, tuples, calls, and closure parameter types
- `[` `]` for arrays
- `{` `}` for `if` branches

It uses these structural punctuation marks inside expressions:

- `,` between tuple elements, array elements, and call arguments
- `:` in typed closure parameters
- `.` before an unsuffixed tuple index

```text
(1, 2)
[0, 1, 2]
|x: i32| x + 1
point.0
```

## Operators

Operator tokens carry arithmetic, comparison, logical, bitwise, cast, and
range syntax.

```text
+ - * / %
== != < <= > >=
! && ||
& | ^ << >>
as
.. ..=
```

The grammar determines where each operator may appear and how tightly it binds.
See [Reference Manual](reference.md) for
the full expression grammar.
