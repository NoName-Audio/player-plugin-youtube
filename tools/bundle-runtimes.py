#!/usr/bin/env python3
"""Download pinned official standalone runtimes, never invoked by the plugin."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile
import urllib.parse
import urllib.request
import zipfile

LIMIT = 256 * 1024 * 1024
ROOT = Path(__file__).resolve().parent


class HTTPSRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlparse(newurl).scheme != "https":
            raise ValueError("Refusing non-HTTPS redirect")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def download(asset, destination):
    if urllib.parse.urlparse(asset["url"]).scheme != "https":
        raise ValueError("Runtime URL must use HTTPS")
    opener = urllib.request.build_opener(HTTPSRedirect())
    request = urllib.request.Request(asset["url"], headers={"User-Agent": "Player-YouTube-runtime-bundler"})
    digest = hashlib.sha256()
    total = 0
    with opener.open(request, timeout=60) as response, destination.open("wb") as output:
        if int(response.headers.get("Content-Length", "0")) > LIMIT:
            raise ValueError("Runtime download exceeds 256 MiB")
        while True:
            chunk = response.read(1024 * 1024)
            if not chunk:
                break
            total += len(chunk)
            if total > LIMIT:
                raise ValueError("Runtime download exceeds 256 MiB")
            digest.update(chunk)
            output.write(chunk)
    if digest.hexdigest() != asset["sha256"]:
        raise ValueError("Published SHA-256 mismatch: " + asset["url"])


def extract(archive, member, destination):
    # Never extract paths supplied by an archive. Accept exactly one flat regular file.
    with zipfile.ZipFile(archive) as bundle:
        entries = bundle.infolist()
        if len(entries) != 1 or entries[0].filename != member:
            raise ValueError("Unexpected Deno archive members")
        info = entries[0]
        mode = info.external_attr >> 16
        if info.is_dir() or stat.S_ISLNK(mode) or (stat.S_IFMT(mode) not in (0, stat.S_IFREG)):
            raise ValueError("Deno archive member is not a regular file")
        if not 0 < info.file_size <= LIMIT:
            raise ValueError("Deno executable exceeds size limit")
        with bundle.open(info) as source, destination.open("wb") as output:
            total = 0
            while True:
                chunk = source.read(1024 * 1024)
                if not chunk:
                    break
                total += len(chunk)
                if total > LIMIT:
                    raise ValueError("Deno executable exceeds size limit")
                output.write(chunk)


def main():
    manifest = json.loads((ROOT / "runtimes.json").read_text())
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=sorted(manifest["targets"]))
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="youtube-runtimes-", dir=args.output) as temporary:
        staging = Path(temporary)
        for index, asset in enumerate(manifest["targets"][args.target]):
            archive = staging / (str(index) + ".download")
            download(asset, archive)
            executable = staging / asset["filename"]
            if asset["archive_member"]:
                extract(archive, asset["archive_member"], executable)
            else:
                archive.replace(executable)
            executable.chmod(0o755)
        if sum((staging / a["filename"]).stat().st_size for a in manifest["targets"][args.target]) > 384 * 1024 * 1024:
            raise ValueError("Combined runtime files exceed package target limit")
        for asset in manifest["targets"][args.target]:
            os.replace(staging / asset["filename"], args.output / asset["filename"])
    for license_file in sorted((ROOT.parent / "licenses").iterdir()):
        if license_file.is_file():
            shutil.copyfile(license_file, args.output / license_file.name)
    print("Bundled yt-dlp " + manifest["yt_dlp_version"] + " and Deno " + manifest["deno_version"] + " for " + args.target)


if __name__ == "__main__":
    main()
