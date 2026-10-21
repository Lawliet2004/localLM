# Desktop interface direction

The home and settings redesign uses [Impeccable](https://github.com/pbakaus/impeccable). The current visual system is documented in [DESIGN.md](../DESIGN.md), and product constraints in [PRODUCT.md](../PRODUCT.md).

The implementation uses warm paper surfaces, forest text, self hosted IBM Plex typography, a centered home composer, searchable settings navigation, and selectable light/dark previews. All existing model, provider, tool, execution, project, and conversation behavior is preserved through the existing state and API paths.

`e2e/redesign.spec.ts` covers responsive home controls, theme previews, persistence, and draft retention. Existing settings and workspace tests cover the settings destinations, search, shortcuts, and preview restrictions.
