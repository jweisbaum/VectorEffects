// Validate before spending time on native builds or creating a GitHub release.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const json = (path) => JSON.parse(readFileSync(path, 'utf8'));
const version = json('package.json').version;
const cargo = readFileSync('Cargo.toml', 'utf8');
const workspace = cargo.split('[workspace.package]')[1]?.split(/\n\[/)[0];
assert.equal(workspace?.match(/^version = "([^"]+)"/m)?.[1], version, 'Cargo version');
assert.equal(json('ui/package.json').version, version, 'UI version');
assert.equal(json('crates/ve-app/tauri.conf.json').version, version, 'Tauri version');
const lock = json('package-lock.json');
assert.equal(lock.version, version, 'npm lockfile version');
assert.equal(lock.packages[''].version, version, 'npm root lockfile version');
assert.equal(lock.packages.ui.version, version, 'npm UI lockfile version');
const notes = readFileSync(`docs/releases/${version}.md`, 'utf8');
assert(notes.trim(), 'Release notes must not be empty');
if (process.env.GITHUB_REF?.startsWith('refs/tags/')) {
  assert.equal(process.env.GITHUB_REF, `refs/tags/v${version}`, 'Release tag must match the application version');
}
console.log(`Release version ${version} and notes are consistent.`);
