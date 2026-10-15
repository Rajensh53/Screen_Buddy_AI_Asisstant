# Contributing to Screen Buddy

Thanks for your interest! Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

Open an issue with:

- what you did, what you expected, and what happened instead
- your Windows version, the engine and model you use (for example Ollama with `qwen3.5:4b`)
- the relevant lines from the log file: `%LOCALAPPDATA%\com.screenbuddy.desktop\logs\screen-buddy.log`

Please don't paste private text you selected; the log never contains it.

## Making a change

1. Read [`CLAUDE.md`](CLAUDE.md): it describes the architecture and the **hard rules** every change must keep
   (no screenshots, mouse hook only, nothing sent anywhere except the chosen model, the popup never takes
   focus, plain-text output, no telemetry).
2. Keep changes small and focused, one topic per pull request.
3. Run the checks before you open the pull request (see the README's
   [Developer guide](README.md#checks)): `cargo clippy --all-targets -- -D warnings` (debug and `--release`),
   `cargo test` and `npm run build`, all without warnings.
4. Add unit tests for pure logic (trigger rules, privacy, prompt, parsing).
5. Ask in an issue before adding a new dependency.
6. Write commit messages in the [Conventional Commits](https://www.conventionalcommits.org) style.

Don't copy code from other projects (GPL ones included): the author must be able to relicense the whole
project (see the contributor terms below). Write it yourself, from official documentation.

## Contributor terms

Screen Buddy is licensed under the [GNU GPL v3.0 or later](LICENSE). By submitting a contribution (a pull
request, a patch, or code in an issue), you agree that:

1. You wrote it yourself, or you otherwise have the right to submit it under these terms.
2. Your contribution is licensed to everyone under GPL-3.0-or-later, like the rest of the project.
3. You also grant **Rajnesh Kumar Charan**, the project's author, a perpetual, worldwide, non-exclusive,
   royalty-free and irrevocable license to use, copy, modify, distribute, sublicense and **relicense** your
   contribution, including under licenses other than the GPL (for example, in paid or store builds of
   Screen Buddy).

You keep the copyright to your contribution. If you don't agree with these terms, please open an issue to
discuss the change instead of submitting code.

The name "Screen Buddy" and the robot character's design are not covered by the license (see the README).
