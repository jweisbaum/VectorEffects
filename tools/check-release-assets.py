"""Require every platform's installers before checksumming and publishing."""

import hashlib
from pathlib import Path
import sys


def main(directory):
    assets = sorted(path for path in directory.iterdir() if path.is_file() and path.name != "SHA256SUMS")
    required = {
        "Intel Mac": lambda name: name.endswith("_x64.dmg"),
        "Apple Silicon Mac": lambda name: name.endswith("_aarch64.dmg"),
        "Windows MSI": lambda name: name.endswith(".msi"),
        "Windows setup": lambda name: name.endswith("-setup.exe"),
        "Linux AppImage": lambda name: name.endswith(".AppImage"),
        "Linux Debian": lambda name: name.endswith(".deb"),
        "Linux RPM": lambda name: name.endswith(".rpm"),
    }
    missing = [platform for platform, matches in required.items() if not any(matches(path.name) for path in assets)]
    if missing:
        raise SystemExit("Missing release installers: " + ", ".join(missing))
    lines = []
    for path in assets:
        if path.stat().st_size == 0:
            raise SystemExit(f"Empty release asset: {path.name}")
        digest = hashlib.sha256()
        with path.open("rb") as asset:
            for block in iter(lambda: asset.read(1024 * 1024), b""):
                digest.update(block)
        lines.append(f"{digest.hexdigest()}  {path.name}\n")
    (directory / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")
    print(f"Verified all four platforms; checksummed {len(assets)} release assets.")


if __name__ == "__main__":
    main(Path(sys.argv[1]))
