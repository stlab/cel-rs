// Combines Spectrum's elements, Spectrum 2 theme tokens, and begin's zoom-control
// icons into one esbuild bundle (../assets/swc.js). This must be a single compiled
// bundle, not separate vendored/live files - see
// docs/superpowers/specs/2026-07-11-begin-spectrum2-theme-tokens-design.md for why
// (module-scoped Theme class state can't be shared across independently-bundled
// files, and raw CSS static imports break in a plain <script type="module"> context).
import '@spectrum-web-components/bundle/elements.js';
import '@spectrum-web-components/theme/spectrum-two/theme-light.js';
import '@spectrum-web-components/theme/spectrum-two/scale-medium.js';
import '@spectrum-web-components/icons-workflow/icons/sp-icon-zoom-in.js';
import '@spectrum-web-components/icons-workflow/icons/sp-icon-zoom-out.js';
