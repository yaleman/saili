import shutil
from pathlib import Path

PROJECT = Path(__file__).resolve().parent
DIST = PROJECT / "dist"
CLIENT = DIST / "client"
SERVER = DIST / "server"
STATIC_FILES = ("index.html", "styles.css", "app.mjs", "parser.mjs", "index.js")


def build() -> None:
    if DIST.exists():
        shutil.rmtree(DIST)
    CLIENT.mkdir(parents=True)
    SERVER.mkdir(parents=True)

    for filename in STATIC_FILES:
        shutil.copy2(PROJECT / filename, CLIENT / filename)


if __name__ == "__main__":
    build()
