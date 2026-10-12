# Contributing Translations to Mouthy

Translations live in `src/i18n/locales/<language-code>/translation.json`. English (`en`) is the source; every other locale must have exactly the same keys.

## Improving an existing translation

Edit the relevant `translation.json` and open a pull request with a short description of the change.

## Adding a language

1. Create `src/i18n/locales/<code>/` using the [ISO 639-1 code](https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes) and copy `en/translation.json` into it.
2. Translate the **values** only. Keep every key, the JSON structure, and any `{{variables}}` exactly as they are.
3. Add the language to `LANGUAGE_METADATA` in `src/i18n/languages.ts`.
4. Run `bun run check:translations`, then check the app under Settings → About → App Language.

## Guidelines

- Use natural, concise language; UI space is limited.
- Do not translate brand or product names (Mouthy, OpenAI).
- Keep technical terms where that is the norm (e.g. "API", "URL").
