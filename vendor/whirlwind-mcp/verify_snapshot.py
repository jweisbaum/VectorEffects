#!/usr/bin/env python3
"""Verify this vendor snapshot against its adjacent provenance manifest."""
import hashlib,json,pathlib,tomllib
root=pathlib.Path(__file__).resolve().parent
manifest=root.with_suffix('.json')
if manifest.exists():
    value=json.loads(manifest.read_text()); digest=hashlib.sha256()
    # Path ordering is case-insensitive on Windows. Hash the same POSIX
    # relative-name order on every platform, matching sync-mcp.mjs.
    for file in sorted((p for p in root.rglob('*') if p.is_file() and 'target' not in p.relative_to(root).parts and '__pycache__' not in p.parts and p.name!='Cargo.lock'), key=lambda p: p.relative_to(root).as_posix()):
        digest.update(file.relative_to(root).as_posix().encode());digest.update(b'\0');digest.update(file.read_bytes())
    assert digest.hexdigest()==value['sha256'],'Shared MCP snapshot changed; sync it from the canonical crate.'
    assert tomllib.loads((root/'Cargo.toml').read_text())['package']['version']==value['version']
    print('Shared MCP source version and SHA-256 verified')
else:
    print('Canonical MCP source; vendor snapshots carry the provenance manifest')
