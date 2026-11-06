# Official Dioxus controls

The `button`, `input`, `checkbox`, `switch`, `slider`, `label`, and `separator`
directories and `../../assets/dx-components-theme.css` originate from
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
dx components add button input checkbox switch slider label separator \
  --git https://github.com/DioxusLabs/dioxus-components \
  --rev 9a758255ea26e2b20c8cecf4c1c946feb9e71da7
```

Local adaptations add adjacent contracts and module tutorials. Theme tokens
are scoped to `.adam-theme`; `Theme` installs the library-owned asset.
Numeric editor and slider composition are local code, not upstream components.
