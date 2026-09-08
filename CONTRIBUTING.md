# Contributing to Coconut Pilot

Thanks for helping make the Copilot key useful on more keyboards and desktops.

## Before opening an issue

1. Update Coconut and run `coconut doctor`.
2. Search existing issues.
3. For a Linux input issue, include your distribution, desktop/session type and
   the output of `coconut doctor`. Do not include unrelated personal paths.
4. For a Windows issue, include the Windows version and whether the physical
   key emits the standard `Win + Shift + F23` chord.

Do not publish security vulnerabilities in a public issue. Follow
[SECURITY.md](SECURITY.md) instead.

## Development setup

Linux requires stable Rust, a C linker, `pkg-config` and GLib/GIO development
headers. See the platform-specific package names in [README.md](README.md).

```sh
# Fork the repository on GitHub, clone it with the Code button, then:
cd coconut-pilot
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --target x86_64-pc-windows-gnu --all-targets
cargo check --locked --target x86_64-pc-windows-msvc --all-targets
```

Do not run the privileged Linux input service unless you are testing a real
keyboard change. Most changes can be verified with the test suite and harmless
CLI actions.

## Pull requests

- Start from the default branch and keep each pull request focused.
- Format with `cargo fmt`; Clippy warnings are treated as errors.
- Add or update meaningful tests when behavior changes.
- Explain the observable behavior before and after the change.
- State which platforms you tested: Linux, Windows, or both.
- Update the README when a user-facing command, install flow or limitation changes.

Maintainers may ask for physical keyboard testing when a change affects input
capture, startup integration or an operating-system adapter.

## Commit messages

Use a short imperative summary, for example:

```text
Add Windows Start Menu browser discovery
Fix stale Linux agent reconnect handling
Document release checksum verification
```
