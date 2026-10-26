"""Serve a local release folder the way GitHub Releases does, for the installer
smoke tests:

    python3 -I scripts/release/serve.py <folder> <port file> <request log>

Listens on 127.0.0.1 on a free port, writes the port to <port file> once it
listens, and answers GET <any path>/<name> with the file <folder>/<name>
(404 when there is none), so `/ngvoicu/kluris/releases/latest/download/x`
and `/ngvoicu/kluris/releases/download/v3.0.0/x` both serve `x`. Each
request's path goes to <request log>, one per line. Runs until killed.
"""

import http.server
import os
import sys
import urllib.parse


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    folder, port_file, log_file = (os.path.abspath(arg) for arg in argv[1:])
    files = {name for name in os.listdir(folder) if os.path.isfile(os.path.join(folder, name))}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            path = urllib.parse.urlsplit(self.path).path
            with open(log_file, "a", encoding="utf-8") as log:
                log.write(path + "\n")
            name = path.rsplit("/", 1)[-1]
            if name not in files:
                self.send_error(404)
                return
            with open(os.path.join(folder, name), "rb") as handle:
                body = handle.read()
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

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
