"""Takes the color themes out of VS Code extension packages (.vsix) into Coiman's bundled themes.

    python scripts/bundle_themes.py path/to/folder-with-vsix-files

Each theme's JSON goes to crates/coiman-core/data/themes/, each extension's licence into
LICENSES.md there, and the list Coiman reads into bundled.rs. Only extensions whose package
says MIT are taken. Run it again with newer packages to update them.
"""
import json
import pathlib
import re
import sys
import unicodedata
import zipfile

OUT = pathlib.Path(__file__).resolve().parent.parent / "crates" / "coiman-core" / "data" / "themes"


def slug(text):
    plain = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    return re.sub(r"[^a-z0-9]+", "-", plain.lower()).strip("-")


def main():
    source = pathlib.Path(sys.argv[1])
    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.json"):
        old.unlink()
    entries = []
    licences = ["# Licences of the bundled color themes\n",
                "Each theme below comes unchanged from its extension on open-vsx.org, under its licence.\n"]
    for vsix in sorted(source.rglob("*.vsix")):
        z = zipfile.ZipFile(vsix)
        pkg = json.loads(z.read("extension/package.json"))
        themes = pkg.get("contributes", {}).get("themes", [])
        if not themes:
            continue
        if pkg.get("license") != "MIT":
            print(f"{vsix.name}: licence {pkg.get('license')!r}, skipped")
            continue
        names = z.namelist()
        licence = next((n for n in names if re.fullmatch(r"extension/licen[cs]e(\.md|\.txt)?", n, re.I)), None)
        if licence is None:
            print(f"{vsix.name}: no licence file, skipped")
            continue
        ext_id = f"{pkg['publisher']}.{pkg['name']}"
        labels = []
        for t in themes:
            label = t.get("label") or t.get("id")
            file = slug(label) + ".json"
            text = z.read("extension/" + t["path"].lstrip("./")).decode("utf-8")
            (OUT / file).write_text(text, encoding="utf-8", newline="\n")
            entries.append((label, t.get("uiTheme", "vs-dark"), file))
            labels.append(label)
        text = z.read(licence).decode("utf-8").strip()
        licences.append(f"\n## {pkg.get('displayName', ext_id)} ({ext_id} {pkg['version']})\n\n"
                        f"Themes: {', '.join(labels)}.\n\n```\n{text}\n```\n")
    # Coiman's own themes, kept in own/ (they're the project's, under its licence).
    for own in sorted((OUT / "own").glob("*.json")):
        theme = json.loads(own.read_text("utf-8"))
        ui = {"hc": "hc-black", "hc-light": "hc-light", "light": "vs"}.get(theme.get("type"), "vs-dark")
        entries.append((theme["name"], ui, "own/" + own.name))
    (OUT / "LICENSES.md").write_text("".join(licences), encoding="utf-8", newline="\n")
    rows = "".join(
        f'    ({json.dumps(label, ensure_ascii=False)}, {json.dumps(ui)}, {json.dumps(file)}, '
        f'include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/themes/{file}"))),\n'
        for label, ui, file in sorted(entries, key=lambda e: e[0].lower())
    )
    (OUT / "bundled.rs").write_text(
        "// Made by scripts/bundle_themes.py: (label, uiTheme, file, text).\n"
        f"&[\n{rows}]\n", encoding="utf-8", newline="\n")
    print(f"{len(entries)} themes")


main()
