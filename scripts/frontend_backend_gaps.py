#!/usr/bin/env python3
"""Compare frontend request sites with Axum registrations and parity notes."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]

def norm(path: str) -> str:
    path = re.sub(r"\$\{[^}]+\}", "{param}", path)
    path = re.sub(r"%7B[^%]+%7D", "{param}", path, flags=re.I)
    return re.sub(r"\{[^{}]+\}", "{param}", path)

def frontend_routes():
    found = set()
    files = [ROOT / "frontend/src/api.ts", *((ROOT / "frontend/src/api").glob("*.ts"))]
    for file in files:
        if not file.exists():
            continue
        src = file.read_text()
        for m in re.finditer(r"request(?:<[^>]*>)?\s*\(\s*([`\"'])(.*?)\1\s*(?:,\s*\{(.*?)\})?", src, re.S):
            path, options = m.group(2), m.group(3) or ""
            path = path.split("?")[0]
            method = re.search(r"method\s*:\s*['\"](\w+)", options)
            verb = method.group(1).upper() if method else ("POST" if "jsonBody(" in options else "GET")
            found.add((verb, norm("/v3" + path)))
    return sorted(found)

def registrations():
    # Router registrations are intentionally extracted from source so this remains
    # useful as routes move between the top-level and controller routers.
    result = set()
    for file in [*(ROOT / "src/api").rglob("*.rs")]:
        src = file.read_text()
        for m in re.finditer(r'\.route\(\s*"([^"]+)"\s*,([^\n]*(?:\n\s*[^\n]*){0,5})', src):
            path, expr = m.group(1), m.group(2)
            for verb in re.findall(r"\b(get|post|put|patch|delete|any)\s*\(", expr):
                result.add((verb.upper(), norm(path)))
    return result

def main():
    front, back = frontend_routes(), registrations()
    parity = (ROOT / "docs/api-parity.md").read_text()
    exceptional = set()
    for method, path, status in re.findall(r"\|\s*`?(GET|POST|PUT|PATCH|DELETE)`?\s*\|\s*`([^`]+)`\s*\|\s*([^|]+)", parity):
        if re.search(r"Missing|501|Not implemented", status, re.I):
            exceptional.add((method, norm(path)))
    gaps = [(m,p) for m,p in front if (m,p) not in back or (m,p) in exceptional]
    print("Frontend-called routes missing registration or marked unimplemented:")
    for method, path in gaps:
        print(f"{method:6} {path}")

if __name__ == "__main__":
    main()
