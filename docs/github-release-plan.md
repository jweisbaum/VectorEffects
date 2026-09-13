# GitHub and platform release plan

Destination: [jweisbaum/VectorEffects](https://github.com/jweisbaum/VectorEffects).

The source repository is linked as `origin` over SSH. On macOS, load the GitHub
key using `ssh-add --apple-use-keychain ~/.ssh/github`. Git pushes use this key;
the release workflow uses its own short-lived `GITHUB_TOKEN`. No personal token
or signing credentials belong in the repository.

## 1. Publish the source

After authentication, fetch and inspect `origin` before choosing the destination
branch. Preserve any remote history; do not force-push over it. If the repository
already contains work, merge and resolve any divergence before pushing.

```sh
git fetch origin
git log --oneline --all --graph -20
git push -u origin main
```

The last command is for an empty remote or a verified fast-forward only. Source
pushes and pull requests run CI. A version tag starts the release pipeline.
Repository visibility also applies to release downloads; private repositories
require collaborators to sign in.

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
Bundling and release uploads follow the
[Tauri GitHub pipeline](https://v2.tauri.app/distribute/pipelines/github/), using
[tauri-action's workflow artifact support](https://github.com/tauri-apps/tauri-action).

## 3. Sign and test the installers

For Developer ID signed Mac builds, configure repository secrets `APPLE_CERTIFICATE`
(base64 Developer ID certificate), `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), and
`APPLE_TEAM_ID`. The workflow passes these to Tauri. Without them it uses ad-hoc
signing for beta builds, which are not notarized. See
[Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/).

To sign Windows installers, choose a signing provider and configure its Tauri
signing command. The workflow currently builds unsigned Windows installers.
See [Tauri Windows signing](https://v2.tauri.app/distribute/sign/windows/).

On one real installation of each target, verify installation and launch, Help and
its bundled screenshots, project save/reopen, pixel brushes in all projections,
object dragging, shape animation and erasing, constant motion and Undo, historical
download retry, GRIB/Zarr playback, and GRIB export. Decode an exported file with
ecCodes and distinguish missing cells from painted calm. Check the expiry screen
in an isolated test VM using dates immediately before and on January 1, 2027.

## 4. Publish a beta

Keep `Cargo.toml`, `package.json`, `ui/package.json`, lockfiles, and the Tauri
configuration on the same version. Add release notes at `docs/releases/VERSION.md`
and tag the tested commit with the matching `vVERSION` tag. The initial release
uses `v0.1.0` and is marked as a beta prerelease on GitHub.

```sh
node tools/check-release.mjs
git tag -a v0.1.0 -m "VectorEffects 0.1.0 Beta"
git push origin v0.1.0
```

The pipeline validates versions and notes, creates one draft prerelease, and
runs all four builds. Only after every build succeeds does it verify that each
platform's installers exist, upload `SHA256SUMS`, and publish the beta. A failed
build leaves a draft that can be resumed using Actions → Re-run failed jobs.
Published releases are not overwritten by a new run; issue a new version instead.

Downloads are hosted at
[GitHub Releases](https://github.com/jweisbaum/VectorEffects/releases), linked from
the repository README. No separate hosting service or personal access token is
needed. GitHub Actions must be enabled for the repository; the workflow requests
`contents: write` only in the release jobs.

This version intentionally stops working at local midnight on January 1, 2027.
State that expiry date in the beta release notes. Ship a replacement version
before that date and explicitly review its expiry policy. No updater or automatic
network check is added by this workflow.

## Current verification limits

The local Intel Mac production executable has been built successfully, with the
current frontend and Help images embedded and WebDriver disabled. Native/UI
regressions, Metal rendering comparisons, ecCodes decoding, and offline checks
are recorded in [the verification checklist](beta-improvements.md).
Use the [Actions page](https://github.com/jweisbaum/VectorEffects/actions) to verify
the hosted build results for a specific commit and tag. Passing CI establishes
automated build/test coverage; real installation smoke tests and optional signing
and notarization are separate checks.
