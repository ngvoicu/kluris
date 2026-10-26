"""Serve a local release folder the way GitHub serves releases, for the
installer and life-cycle smoke tests:

    python3 -I scripts/release/serve.py <folder> <port file> <request log>

Listens on 127.0.0.1 on a free port, writes the port to <port file> once it
listens, logs each request's path to <request log> (one per line), and runs
until killed.

Downloads, as github.com serves them:
- `.../releases/download/<tag>/<name>`: `<folder>/<tag>/<name>` when the
  folder holds release folders (`v3.0.0/`, `v3.0.1/`), else `<folder>/<name>`;
- `.../releases/latest/download/<name>`: the same from the newest release
  folder (prereleases left out), else `<folder>/<name>`;
- any other path: `<folder>/<name>` for its last part (`/install.sh`).

The release API, as api.github.com answers `self_update` (when the folder
holds release folders):
- `/repos/<owner>/<repo>/releases`: every release, newest first, one page;
- `/repos/<owner>/<repo>/releases/tags/<tag>`: that release;
- `/repos/<owner>/<repo>/releases/assets/<tag>/<name>`: the asset's bytes,
  each asset's `url` pointing there, with GitHub's `digest` (`sha256:<hex>`).
"""

import hashlib
import http.server
import json
import os
import re
import sys
import urllib.parse

RELEASE_DIR = re.compile(r"^v(\d+)\.(\d+)\.(\d+)(-[0-9A-Za-z.-]+)?$")


def version_key(tag: str):
    match = RELEASE_DIR.match(tag)
    if match is None:
        return None
    major, minor, patch, pre = match.groups()
    return (int(major), int(minor), int(patch), pre is None)


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    folder, port_file, log_file = (os.path.abspath(arg) for arg in argv[1:])
    tags = sorted(
        (name for name in os.listdir(folder)
         if os.path.isdir(os.path.join(folder, name)) and version_key(name) is not None),
        key=version_key,
        reverse=True,
    )
    stable = [tag for tag in tags if version_key(tag)[3]]

    def file_of(tag, name):
        base = os.path.join(folder, tag) if tag else folder
        path = os.path.join(base, name)
        if "/" in name or "\\" in name or name in ("", ".", "..") or not os.path.isfile(path):
            return None
        return path

    class Handler(http.server.BaseHTTPRequestHandler):
        def base(self) -> str:
            host, port = self.server.server_address[:2]
            return f"http://{host}:{port}"

        def release_json(self, owner: str, repo: str, tag: str) -> dict:
            assets = []
            for name in sorted(os.listdir(os.path.join(folder, tag))):
                path = os.path.join(folder, tag, name)
                if not os.path.isfile(path):
                    continue
                with open(path, "rb") as handle:
                    digest = hashlib.sha256(handle.read()).hexdigest()
                assets.append({
                    "name": name,
                    "url": f"{self.base()}/repos/{owner}/{repo}/releases/assets/{tag}/{name}",
                    "browser_download_url": f"{self.base()}/{owner}/{repo}/releases/download/{tag}/{name}",
                    "digest": f"sha256:{digest}",
                })
            return {
                "tag_name": tag,
                "name": tag,
                "created_at": "2026-10-26T00:00:00Z",
                "body": "",
                "draft": False,
                "prerelease": not version_key(tag)[3],
                "html_url": f"{self.base()}/{owner}/{repo}/releases/tag/{tag}",
                "assets": assets,
            }

        def send_json(self, value) -> None:
            body = json.dumps(value).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def send_file(self, path) -> None:
            if path is None:
                self.send_error(404)
                return
            with open(path, "rb") as handle:
                body = handle.read()
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self) -> None:
            path = urllib.parse.urlsplit(self.path).path
            with open(log_file, "a", encoding="utf-8") as log:
                log.write(path + "\n")
            parts = [p for p in path.split("/") if p]
            if len(parts) >= 4 and parts[0] == "repos" and parts[3] == "releases" and tags:
                owner, repo, rest = parts[1], parts[2], parts[4:]
                if not rest:
                    self.send_json([self.release_json(owner, repo, tag) for tag in tags])
                elif len(rest) == 2 and rest[0] == "tags" and rest[1] in tags:
                    self.send_json(self.release_json(owner, repo, rest[1]))
                elif len(rest) == 3 and rest[0] == "assets" and rest[1] in tags:
                    self.send_file(file_of(rest[1], rest[2]))
                else:
                    self.send_error(404)
                return
            name = parts[-1] if parts else ""
            if len(parts) >= 3 and parts[-3] == "download" and parts[-2] in tags:
                self.send_file(file_of(parts[-2], name))
            elif len(parts) >= 3 and parts[-3:-1] == ["latest", "download"] and stable:
                self.send_file(file_of(stable[0], name))
            else:
                self.send_file(file_of("", name))

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    with open(port_file + ".tmp", "w", encoding="utf-8") as handle:
        handle.write(f"{server.server_address[1]}\n")
    os.replace(port_file + ".tmp", port_file)
    server.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
