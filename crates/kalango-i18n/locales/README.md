# UI catalogs (`kalango-i18n`)

Each file is `locales/<locale>.json`: a flat JSON object mapping **English message ids** (the strings in
`tr("…")` / `trf("…")` / `n_("…")`) to the translation.

```json
{
  "Search apps and actions…": "Buscar apps e ações…",
  "{n} requests": "{n} pedidos"
}
```

## Rules

- **English is the source.** There is no `en.json`: when no catalog matches, the msgid is shown as-is.
- **Named placeholders** (`{n}`, `{version}`, …) must appear unchanged in the translation.
- **Locale name** is `language_REGION` (`pt_BR`, `es_ES`, `ja_JP`). Register the file in
  `crates/kalango-i18n/src/lib.rs` (`CATALOGS`) so it is embedded at build time.
- **Completeness:** `cargo test -p kalango-i18n` fails if `pt_BR` misses a used string or keeps an unused one.
  New catalogs should start from a copy of the English keys (export / Weblate) and grow toward the same coverage.
- **Do not invent personal data** in sample strings (see `AGENTS.md`).

## Adding a language

1. Create `locales/<locale>.json` (Weblate or by hand).
2. Add `("<locale>", include_str!("../locales/<locale>.json"))` to `CATALOGS` in `src/lib.rs`.
3. Run `cargo test -p kalango-i18n --locked`.
4. Open a pull request. UI chrome for picking languages already lists system locales (`locale -a`); a catalog
   only changes Kalango’s own labels.

## Weblate

Hosted project wiring lives in the repo root `weblate.yml` and is described in `docs/I18N.md`. Until the host is
live, PRs with new JSON catalogs are welcome.
