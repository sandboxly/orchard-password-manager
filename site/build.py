"""Build the website into _site/: a copy of site/ without the build files.
The page's @VERSION@ ("1.0.1") and @MINOR_VERSION@ ("1.0") come from the version in Cargo.toml.

    python3 site/build.py    then serve _site/ (python3 -m http.server -d _site)
"""

from __future__ import annotations

import shutil
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
OUT = ROOT / "_site"


def version() -> str:
    with open(ROOT / "Cargo.toml", "rb") as f:
        return tomllib.load(f)["package"]["version"]


def build() -> None:
    shutil.rmtree(OUT, ignore_errors=True)
    shutil.copytree(SITE, OUT, ignore=shutil.ignore_patterns("build.py", "__pycache__", ".DS_Store"))
    full = version()
    index = OUT / "index.html"
    text = index.read_text(encoding="utf-8").replace("@VERSION@", full)
    index.write_text(text.replace("@MINOR_VERSION@", ".".join(full.split(".")[:2])), encoding="utf-8")
    print(f"Built {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    build()
