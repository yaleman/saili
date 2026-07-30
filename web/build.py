import shutil
from pathlib import Path

PROJECT = Path(__file__).resolve().parent
DIST = PROJECT / "dist"
CLIENT = DIST / "client"
SERVER = DIST / "server"
STATIC_FILES = ("index.html", "styles.css", "app.mjs", "parser.mjs")


def build() -> None:
    if DIST.exists():
        shutil.rmtree(DIST)
    CLIENT.mkdir(parents=True)
    SERVER.mkdir(parents=True)

    for filename in STATIC_FILES:
        shutil.copy2(PROJECT / filename, CLIENT / filename)

    (SERVER / "index.js").write_text(
        "export default {\n"
        "  fetch(request, env) {\n"
        "    return env.ASSETS.fetch(request);\n"
        "  },\n"
        "};\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    build()
