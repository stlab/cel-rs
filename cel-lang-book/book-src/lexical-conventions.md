# Lexical conventions

The lexer reads a CEL source file as a stream of tokens before the parser reads
it as expressions. This chapter describes the token forms that participate in
CEL source. For background on the underlying token style, see the
[Rust Reference](https://doc.rust-lang.org/reference/). For the full CEL
expression grammar, see the [Reference Manual](reference.md).

## Whitespace and joint punctuation

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

Compound punctuation uses joint spelling: write the characters of a compound
token with no whitespace between them. This rule applies to `&&`, `||`,
`==`, `!=`, `<=`, `>=`, `<<`, `>>`, `..`, and `..=`.

```text
a&&b
x<=y
1..=5
```

Write each compound token as one uninterrupted piece of punctuation.

## Comments and trivia

Comments are source trivia. They may appear between tokens in the same places
as whitespace and do not become expressions of their own.

```text
x /* midpoint */ + 1
if ready { 1 } // done
else { 0 }
```

Line comments use `// ...`. Block comments use `/* ... */`.

## Identifiers and reserved words

Identifiers name values, callees, and closure parameters.

```text
x
user_name
value2
```

The following words have fixed roles in CEL source:

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

## Lexing and grammar

Lexing answers "what token is this text?" Grammar answers "how do these tokens
combine into a CEL expression?" The same delimiter token can therefore
participate in different expression forms:

```text
()
(1 + 2)
(1, 2)
round(3.5)
1..5
```

Use this chapter for token shapes and the [Reference Manual](reference.md) for
the full expression grammar.
