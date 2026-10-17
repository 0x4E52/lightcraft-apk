# Third-party licences

What the shipped apps show in **About ▸ Licences**, next to LightCraft's own `NOTICE`, `LICENSE-MIT` and
`LICENSE-APACHE`.

- `third-party.json` is generated: every crate the shipped apps link (normal dependencies of `lightcraft`,
  `lightcraft-cli` and `lightcraft-web`, all features, every target), the licence it declares, the licence(s)
  LightCraft uses it under, its authors and repository, and the licence and notice files it publishes. Don't edit
  it by hand: after a dependency change run `cargo xtask licences --write` and commit the result. `cargo xtask
  licences` (part of `cargo xtask ci`) fails when it is out of date.
- `spdx/<id>.txt` are the standard licence texts, from the SPDX License List
  (https://github.com/spdx/license-list-data, `text/`), used for crates that publish no licence file of their
  own. The generator names the file to add when a new licence needs one.

The licence policy lives in `xtask/src/licences.rs` (`ALLOWED`): permissive licences and MPL-2.0. A dependency
available only under the GPL family (GPL, LGPL, AGPL) or any other licence not listed fails the check; a crate that
offers such a licence as one alternative among permissive ones is used under a permissive one.
