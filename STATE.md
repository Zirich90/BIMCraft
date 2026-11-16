# Project state

Updated 2026-11-16. The current-status section of `docs/low-voltage-roadmap.md`
is the authority for stage acceptance.

## Current work

Merge upstream BIMCraft v0.12.0 into `low-voltage-circuits`, validate the combined
workspace, commit with the user's one-shot Git identity, then publish to
`Zirich90/BIMCraft` and start the cross-platform release workflow with a new `v*` tag.
The merged source passes the final Cargo package. Fork publication and the new release
tag remain pending the committed-change handoff gate.

## Base and merge decisions

- Pre-merge HEAD: `afeec3407d26b29940cf4920f56152ecf98ecb9b`. It contains the
  low-voltage isolation refactor and six routing fixes, already published to the fork.
- Upstream base: official `v0.12.0`, peeled commit
  `355a90d8de43f2619a691ae8552d561e4b962b9d`. All 60 conflicts in 19 files are resolved.
- The local upstream tag must not be pushed as the fork release: it identifies pure
  upstream, not the merged project. The fork release tag must identify the merged HEAD.
- Model `FORMAT` is **28**. Upstream v0.12.0 uses format 20; the fork already used
  versions through 27. There are no upstream migrations numbered 20 through 27.
  Fork fields/defaults and upstream migrations are retained. Project-information
  migration is unconditional and idempotent so fork files of formats 20–27 receive it.
  `low_voltage::migrate` is currently a no-op. Upstream cannot read format 28 files.
- `Component` retains fork `provision`, `mount`, `detector_space`, `connection_sides`
  and upstream `adaptive`, `host`, `panel` fields.
- Fork placement delegates to upstream `place_component_params(params, z)`, retaining
  bottom-height/Shift placement while adding rotation and work-plane placement.
- UI commands, including catalogue synchronization and fallback carrier checking, use
  `exec`/`exec_batch` for Co-Work dispatch. `run` retains carrier and reload feedback.
- The ribbon contains 14 tabs, including Low Voltage and Collaborate; there are 13 UI
  language choices. Equipment Catalogue is in Manage > Project Location.
- CI retains fork timeouts and target-directory handling. Release jobs build the sample
  WASM plugin; Windows creates `target/tmp` before the MSVC linker uses it.

## Architecture and completed routing work

Rust workspace: `crates/core` owns the model/commands, `crates/ui-egui` is the
egui/eframe/wgpu front end, and `apps/bimcraft` / `apps/bimcraft-cli` are desktop and CLI/MCP.
Low-voltage logic lives in `crates/core/src/low_voltage/`; its tab is in
`crates/ui-egui/src/low_voltage_ui.rs`. Model changes use `Session::execute`, atomic
validation and GUID references. Follow `AGENT.MD` and each crate's local instructions.

The six fixes in `afeec340` are:
1. Group parallel cables into a shared carrier when its capacity permits; otherwise
   retain separate carriers.
2. Join wall-to-ceiling and wall-to-wall mounted runs at their planes' intersection;
   report unreachable transitions.
3. Place devices by their bottom height, with Shift selecting the family's normative
   height (1500 mm bottom height when none is defined).
4. Keep a cable as one logical route using `Run.route`, a shared mark and aggregate
   length, with route-wide selection/copy/delete and cascade deletion of its head.
   Geometric segments remain separate for IFC and carrier calculations.
5. Split an existing cable at a Wire-tool click to continue or branch without a gap,
   in both plan and 3D.
6. Support 3D run height, device-terminal drops/rises and constrained orthogonal
   surface-snapped routing in both interface profiles.

## Merge regression corrections

- Preserve formula warnings alongside carrier warnings; name steep slab top/bottom
  faces by their normal's sign for hosting.
- Update format, PDF-header, family-fingerprint and sketch-message expectations to
  match the combined implementation. Gross-area fixtures explicitly use centerlines;
  new fork projects still default to finish-face room areas.
- The property-boundary sheet test checks invariant text geometry and alpha, allowing
  only the measured four-level RGB antialiasing variation after viewport reframing.
- UI test clicks scroll offscreen ribbon tabs into view. Family-category selection is
  scoped to the dialog body, then scrolled and checked before confirming.
  Catalogue/link tests use Project Location; sheet tests use the Sheet Composition menu.
- Manual Call Point uses **MC**; **CP** remains Specify Coordinates.
- Work-plane-based components show only one Place on Face control; fixture tools keep
  their face-placement checkbox. A regression covers both tools and family modes.
  Component face hosting bypasses equipment mounting and fixed-height placement, with
  regression coverage for zero and nonzero values in the Offset field.
- Restore Portuguese/Spanish Cable Carrier and low-voltage fixture labels, including
  category labels. Translation catalogs were regenerated after the shortcut change.

## Validation (Linux, Rust stable 1.100.0)

- Pre-merge package `tmpbu79n7u7`: fmt, Clippy and tests passed; 3318 passed, 17 ignored.
- Initial merged package `tmp9uowgp_f`: fmt/Clippy passed; ten core unit failures
  identified and corrected. Later packages exposed property raster and room-area
  fixture differences; the complete core suite subsequently passed.
- The first complete UI run had 1346 passed, 26 failed and 8 ignored. The corrections
  above address the observed causes; that historical result is not final acceptance.
- Package `target/cargo-stop/tmpsro653ee.log`: formatting and strict all-target Clippy
  passed. Core unit tests: 2579 passed, 11 ignored; all core integration tests passed.
  UI: 1366 passed, 7 failed, 8 ignored. Cargo test exited 101; later binaries/doc tests
  were not reached. The seven failures cover sheet menus, missing shortcut translations,
  family-category selection and 3D face hosting.
- Final package `target/cargo-stop/tmp3g8yjbb6.log`, run through
  `scripts/cargo_stop_hook.py` after the full correction batch: fmt, strict all-target
  Clippy and `cargo test --workspace -j 1` passed (Cargo exit 0).
  Core unit tests: 2579 passed, 11 ignored; UI: 1373 passed, 8 ignored. All remaining
  workspace integration/unit tests and doc-test targets passed; two SDK doc tests remain
  ignored. The hook's reporting exit 2 is not a test failure.
- Cargo reports the pre-existing plugin SDK manifest warning about workspace lint
  inheritance. This did not fail strict Clippy.
- `cargo i18n update` and `cargo fmt` applied. No temporary debug tests remain.
- Working-tree diff whitespace check passes. The full merge's whitespace report also
  includes upstream TSV empty trailing fields and generated PO ending blank lines;
  the map TSV's Git blob is identical to upstream and has not been reformatted.

## Release prerequisites and remaining limits

- Commit only with the explicitly authorized one-shot Zirich90 identity; do not change
  persistent Git configuration. Complete the current handoff gate before pushing.
- Release jobs require the fork's `BIMCRAFT_KEY_SECRET` Actions secret. Its presence
  has not yet been verified. Do not weaken the release test if it is missing.
- The release workflow covers Windows x64, Linux x86_64/aarch64/riscv64, Flatpak and
  Arch packages, and universal macOS. Artifact success requires workflow evidence.
- A suffixed fork tag is ignored by the existing automatic updater. The updater still
  targets upstream; changing the update channel is outside this merge/release task.
- Native visual acceptance across all themes/profiles and Windows/macOS installation
  checks have not been performed on this merged snapshot.
- Russian UI currently falls back to English because `translations/ui/ru.po` is absent.
  Other interface translations remain partial; project `ru.po` is 174/231 translated.
- Stage 8 real-project output/IOS5 comparison and stage 9 end-to-end, performance and
  recovery acceptance remain open; this merge does not complete those stages.

## Shared context

The active workspace is `/mnt/nvme/BIMCraft` on Linux. Windows paths and the separate
`target/low-voltage-upstream-012` worktree in shared notes describe other snapshots,
not this format-28 merge. `.memory` points to the shared Obsidian store. Its state and
blackboard need synchronization with this branch's final results. The graph cache is
from 2026-11-11 and predates the merge; current source is authoritative. The new face-toggle
regression and revised placement-mode/category-selection details are absent from the
report. The validated source SHA-256 values are:
- `crates/ui-egui/src/orbit.rs`: `a72064c5a34b9d1f76be66fe8b9cc419d0b1507c1c378a116d3016e8491c95b5`
- `crates/ui-egui/src/tests.rs`: `656a72836292177829d5c4ce9f2d1f4ebb76e619de7bebf09e2b8435bd9709a6`
- `crates/ui-egui/src/tests/work_plane_based.rs`: `9206a2ad3a936c5c5a5c9caebc9468bc2e41de2d8a25d727d2afe46fbe7590d3`

Qwen availability and GPU reserve were not established, so no Qwen task was dispatched
for this batch.
