# GitHub and platform release plan

Destination: [jweisbaum/VectorEffects](https://github.com/jweisbaum/VectorEffects).

The source repository is linked as `origin`. GitHub rejected the machine's saved
HTTPS credentials on September 13, 2026, so the remote's contents and default
branch have not been verified and the source has not been pushed. Refresh GitHub
authentication in the local Git credential manager or GitHub Desktop; do not put
a token in a remote URL or a tracked file.

## 1. Publish the source

After authentication, fetch and inspect `origin` before choosing the destination
branch. Preserve any remote history; do not force-push over it. If the repository
is empty, push local `main`. If it already contains work, publish this commit on
a feature branch and merge through a pull request after resolving any divergence.

```sh
git fetch origin
git log --oneline --all --graph -20
git push -u origin main
```

The last command is for an empty remote or a verified fast-forward only. Configure
branch protection for `main` once CI has reported its check names. Source pushes
and pull requests run CI; no source push creates a public release.

## 2. Build all four targets

The checked-in [release workflow](../.github/workflows/release.yml) can be run from
Actions → Release builds → Run workflow to produce downloadable build artifacts
without creating a release. It runs the tests, binding check, frontend build, and
offline check before bundling. The optional WebDriver feature is never enabled.

| Build | Runner | Rust target | Installers |
| --- | --- | --- | --- |
| Intel Mac | `macos-15-intel` | `x86_64-apple-darwin` | `.dmg`, `.app` |
| Apple Silicon Mac | `macos-15` | `aarch64-apple-darwin` | `.dmg`, `.app` |
| Windows 64-bit | `windows-2022` | `x86_64-pc-windows-msvc` | `.msi`, setup `.exe` |
| Linux 64-bit | `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `.AppImage`, `.deb`, `.rpm` |

Separate Mac runners test each architecture natively. The Linux build uses the
older Ubuntu baseline to avoid unnecessarily raising its runtime requirements.
Runner architectures were checked against
[GitHub's runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
Bundling and draft-release uploads follow the
[Tauri GitHub pipeline](https://v2.tauri.app/distribute/pipelines/github/), using
[tauri-action's workflow artifact support](https://github.com/tauri-apps/tauri-action).

## 3. Sign and test the installers

For distributed Mac builds, configure repository secrets `APPLE_CERTIFICATE`
(base64 Developer ID certificate), `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), and
`APPLE_TEAM_ID`. The workflow passes these to Tauri. Without them it uses ad-hoc
signing for test builds; notarization is a separate release requirement. See
[Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/).

Choose a Windows signing provider and configure its Tauri signing command before
public distribution. The workflow currently builds unsigned Windows installers.
See [Tauri Windows signing](https://v2.tauri.app/distribute/sign/windows/).

On one real installation of each target, verify installation and launch, Help and
its bundled screenshots, project save/reopen, pixel brushes in all projections,
object dragging, shape animation and erasing, constant motion and Undo, historical
download retry, GRIB/Zarr playback, and GRIB export. Decode an exported file with
ecCodes and distinguish missing cells from painted calm. Check the expiry screen
in an isolated test VM using dates immediately before and on January 1, 2027.

Review the declared `MIT OR Apache-2.0` licensing and include the applicable
license texts and bundled dataset/asset notices before making the source or
installers public. No signing credentials belong in this repository.

## 4. Publish a reviewed beta

Keep `Cargo.toml`, `package.json`, `ui/package.json`, lockfiles, and the Tauri
configuration on the same version. Use a beta version such as `0.1.0-beta.1` when
preparing the first distributable build, then tag its tested commit with the
matching `v0.1.0-beta.1` tag. Pushing the tag runs all four builds and attaches the
installers to a **draft prerelease**. Check architecture labels and installation
results, add release notes and checksums, and publish the draft when ready.

This version intentionally stops working at local midnight on January 1, 2027.
State that expiry date in the beta release notes. Ship a replacement version
before that date and explicitly review its expiry policy. No updater or automatic
network check is added by this workflow.

## Current verification limits

The local Intel Mac production executable has been built successfully, with the
current frontend and Help images embedded and WebDriver disabled. Native/UI
regressions, Metal rendering comparisons, ecCodes decoding, and offline checks
are recorded in [the verification checklist](beta-improvements.md).
GitHub-hosted builds, Windows/Linux installer smoke tests, and signing/notarization
remain release steps until repository authentication and signing configuration
are available. This plan does not claim that those builds have run.
