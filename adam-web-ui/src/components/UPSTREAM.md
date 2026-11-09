# Official Dioxus controls

The `button`, `input`, `checkbox`, `switch`, `slider`, `label`, `separator`,
and `card` directories and `../../assets/dx-components-theme.css` originate from
[DioxusLabs/dioxus-components](https://github.com/DioxusLabs/dioxus-components)
at revision `9a758255ea26e2b20c8cecf4c1c946feb9e71da7`.
The registry redirects to `DioxusLabs/components`; the `dioxus-primitives`
dependency pins the same commit and disables its default router feature.

The source is available under MIT or Apache-2.0. This distribution uses the
MIT option and retains the upstream permission notice in `LICENSE-MIT`.
The upstream MIT file contains no separate copyright attribution.

The checkbox imports `Check` from `dioxus-icons` 0.1.0, whose crate declares
`MIT AND ISC`. `LICENSE-DIOXUS-ICONS` and `LICENSE-LUCIDE` retain that package's
MIT, Lucide ISC, and Feather MIT notices, including the attribution for `check`.

Import from `adam-web-ui` with:

```bash
dx components add button input checkbox switch slider label separator card \
  --git https://github.com/DioxusLabs/dioxus-components \
  --rev 9a758255ea26e2b20c8cecf4c1c946feb9e71da7
```

Local adaptations add adjacent contracts and module tutorials. Theme tokens
are scoped to the theme module; `Theme` installs the library-owned embedded
stylesheet. Lightning CSS transforms the original modules and supplies their
class exports using the public `adam_<component>_[local]` naming pattern.
The installer deduplicates one immutable style element per document, independent
of Dioxus root lifetimes and publication paths. Every standalone styled control
also installs it; consumers do not need a Dioxus asset bundler.
Numeric editor and slider composition are local code, not upstream components.

`build.rs` additionally rewrites every embedded `<n>rem` length into
`calc(<n> * var(--adam-rem, 1rem))` before parsing each module (`scale_rem`),
so a host scales every control together by setting the `--adam-rem` custom
property on `.adam-theme` or an ancestor; the `1rem` fallback applies when no
host sets it. The theme stylesheet documents this near `.adam-theme` and does
not declare `--adam-rem` itself, since `scale_rem` would otherwise rewrite
that declaration's own `rem` value into a self-referential `calc()`.
