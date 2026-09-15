# VectorEffects

VectorEffects is a desktop editor for painting and animating global wind and
ocean-current fields, combining them with GRIB or Zarr data, and exporting GRIB2.
Generate and edit routing forecasts to test routing algorithms.

[Download releases](https://github.com/jweisbaum/VectorEffects/releases) ·
[Build status](https://github.com/jweisbaum/VectorEffects/actions) ·
[Release notes](docs/releases/0.1.5.md)

![VectorEffects workspace](ui/public/help/workspace.png)

## Install

Open a release and expand **Assets**. Choose `_x64.dmg` for an Intel Mac,
`_aarch64.dmg` for Apple Silicon, `-setup.exe` or `.msi` for Windows, or
`.AppImage`, `.deb`, or `.rpm` for Linux. A `SHA256SUMS` file accompanies each
complete release.

**The current beta expires on January 1, 2027.** Initial beta installers are
ad-hoc signed on Mac and unsigned on Windows. Use the app's Help menu or F1 for
the illustrated guide.

## Develop

Install Rust 1.97 or newer, Node.js 22, and the
[Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/).
Run commands from the repository root:

```sh
npm ci
npm run dev
```

Build installers with `npm run build`. The optional WebDriver feature is only
for local automation and is never included in release builds.

Rust owns the project model, animation, rendering, and GRIB encoding. The UI uses
React and TypeScript inside Tauri. See [spec.md](spec.md) for the application
contract, [CLAUDE.md](CLAUDE.md) for development conventions, and the
[release guide](docs/github-release-plan.md) for the four-platform pipeline.
