# Contributing to Mouthy

Mouthy is in early development. Issues and pull requests are welcome at [azizi-a/mouthy](https://github.com/azizi-a/mouthy).

## Getting started

1. Set up the toolchain and build as described in [BUILD.md](BUILD.md).
2. Read [AGENTS.md](AGENTS.md) for the architecture and conventions, and [docs/PLAN.md](docs/PLAN.md) for what is in progress.

## Before opening a pull request

Run the checks and make sure they pass:

```bash
bun run lint
bun run format:check
bun run check:translations
cd src-tauri && cargo test
```

- Keep changes small and focused; one logical change per commit.
- Use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`).
- Add a line to `CHANGELOG.md` under the top, unreleased version for user-visible changes.
- Ask in an issue before adding a new dependency.
- All user-facing strings go through i18next; see [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
