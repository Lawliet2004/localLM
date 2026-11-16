# LocalLM GLib 0.18 compatibility backport

This is the crates.io `glib` 0.18.5 distribution, retaining its MIT license.
GTK3's Rust bindings require this API series. Upgrading only GLib to 0.20
would introduce incompatible types throughout the GTK3 dependency graph.

The only upstream source change is in `src/variant_iter.rs`: the output
pointer passed to `g_variant_get_child` is declared mutable and supplied as
`&mut p`. This backports the fix in gtk-rs/gtk-rs-core PR 1343 for
RUSTSEC-2024-0429. It prevents the optimizer from treating a C out-argument
as immutable and subsequently using a null pointer.

- Advisory: https://rustsec.org/advisories/RUSTSEC-2024-0429.html
- Upstream fix: https://github.com/gtk-rs/gtk-rs-core/pull/1343
- Original package checksum and source hashes: `LOCALLM-BACKPORT.json`
- Optimized regression: `cargo test --release --manifest-path vendor/glib/Cargo.toml --target-dir target/glib-backport --test variant_iter_backport`

The version remains 0.18.5. Unfiltered advisory reports must retain this
version-based warning alongside the backport evidence; do not claim that a
package-version scan alone identifies the patch. Remove this override when
the GTK dependency supports a maintained GLib release containing the fix.
