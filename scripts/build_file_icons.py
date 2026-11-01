"""Builds core/data/file_icons.txt from an unpacked material-icon-theme package.

    python scripts/build_file_icons.py path/to/package

The result is one file: which icon each file name and extension gets, in dark and
light themes, and the SVG of each icon used. Folder icons are left out: nothing draws
them. Run it again to take a newer version of the theme.
"""
import json
import pathlib
import sys

# Languages the theme knows by id, for an editor that would tell it the language.
BY_LANGUAGE = {
    "erl": "erlang", "hrl": "erlang", "ex": "elixir", "exs": "elixir", "jl": "julia", "scala": "scala", "sc": "scala",
    "hs": "haskell", "lhs": "haskell", "pl": "perl", "pm": "perl", "nim": "nim", "elm": "elm", "tf": "terraform",
    "groovy": "groovy", "gradle": "gradle", "bb": "clojure", "for": "fortran", "fi": "fortran", "fpp": "fortran",
    "ftn": "fortran", "f": "fortran", "f90": "fortran", "f95": "fortran", "f03": "fortran", "f08": "fortran",
    "cl": "lisp", "lisp": "lisp", "lsp": "lisp", "asd": "lisp", "el": "lisp", "scm": "scheme", "ss": "scheme",
    "rkt": "racket", "pyx": "python", "pxd": "python", "rlib": "rust", "nims": "nim", "vh": "verilog", "org": "document",
    "sed": "console", "make": "makefile", "ninja": "settings", "thrift": "settings", "avsc": "json", "di": "d",
    "vy": "python", "wit": "webassembly", "war": "zip", "dylib": "exe", "map": "javascript-map", "sum": "lock",
    "coq": "coq", "thy": "document", "mat": "settings",
    # Euro Truck Simulator's own: settings as text, packed archives, models and textures.
    "sii": "settings", "sui": "settings", "scs": "zip", "tobj": "image", "dds": "image", "pmd": "3d", "pmg": "3d",
    "ppd": "3d",
}
NAMES = {
    "cargo.toml": "rust", "cargo.lock": "lock", "build.gradle": "gradle", "build.gradle.kts": "gradle",
    "settings.gradle": "gradle", "settings.gradle.kts": "gradle",
}
# Languages the theme has nothing for. These two are this project's own, and plain.
OWN = {
    "rc-agda": '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><path fill="#7e57c2" d="M16 3 29 27H23L16 14 9 27H3z"/></svg>',
    "rc-apl": '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><circle cx="16" cy="16" r="9" fill="none" stroke="#66bb6a" stroke-width="4"/><path fill="#66bb6a" d="M14 2h4v28h-4z"/></svg>',
}
OWN_EXTENSIONS = {"agda": "rc-agda", "apl": "rc-apl", "aplf": "rc-apl", "apln": "rc-apl", "dyalog": "rc-apl"}


def main():
    package = pathlib.Path(sys.argv[1])
    theme = json.loads((package / "dist" / "material-icons.json").read_text("utf-8"))
    known = theme["iconDefinitions"]
    names = {k.lower(): v for k, v in theme["fileNames"].items()}
    extensions = {k.lower(): v for k, v in theme["fileExtensions"].items()}
    for table, extra in ((names, NAMES), (extensions, BY_LANGUAGE)):
        for key, icon in extra.items():
            if icon not in known:
                print(f"The theme has no icon {icon} for {key}: skipped")
                continue
            table.setdefault(key, icon)
    extensions.update(OWN_EXTENSIONS)
    light_names = {k.lower(): v for k, v in theme["light"]["fileNames"].items()}
    light_extensions = {k.lower(): v for k, v in theme["light"]["fileExtensions"].items()}
    used = {theme["file"], *names.values(), *extensions.values(), *light_names.values(), *light_extensions.values()}
    lines = []
    for tag, table in (("N", names), ("E", extensions), ("n", light_names), ("e", light_extensions)):
        lines += [f"{tag}\t{key}\t{icon}" for key, icon in sorted(table.items()) if "\t" not in key]
    for icon in sorted(used):
        if icon in OWN:
            svg = OWN[icon]
        else:
            svg = (package / "dist" / known[icon]["iconPath"]).resolve().read_text("utf-8")
        lines.append("I\t{}\t{}".format(icon, " ".join(svg.split())))
    out = pathlib.Path(__file__).resolve().parent.parent / "core" / "data" / "file_icons.txt"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(names)} names, {len(extensions)} extensions, {len(used)} icons, {out.stat().st_size / 1e6:.2f} MB")


main()
