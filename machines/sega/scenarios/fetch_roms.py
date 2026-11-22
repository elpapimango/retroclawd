#!/usr/bin/env python3
"""Download Mega Drive games from the owner's RomM server into software/roms/<id>/ (git-ignored), unzipped.

    fetch_roms.py ID [ID ...]        (ROMM_URL and ROMM_TOKEN in the environment; ids from `retroclawd romm games 54`)

Prints one line per game: id, name, size, SHA-1 of the ROM file. Each download goes in its own directory; nothing in it is
executed. The ROMs are never committed.
"""
import hashlib
import io
import json
import os
import sys
import urllib.parse
import urllib.request
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def get(path):
    request = urllib.request.Request(os.environ["ROMM_URL"].rstrip("/") + path,
                                     headers={"Authorization": "Bearer " + os.environ["ROMM_TOKEN"]})
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def main():
    for rom_id in sys.argv[1:]:
        try:
            fetch(rom_id)
        except Exception as e:  # a damaged archive on the server must not stop the others
            print(rom_id, "FAILED:", e)


def fetch(rom_id):
    if True:
        out = os.path.join(ROOT, "software", "roms", rom_id)
        if os.path.isdir(out) and os.listdir(out):
            print(rom_id, "already there")
            return
        meta = json.loads(get(f"/api/roms/{rom_id}"))
        name = meta["files"][0]["file_name"]
        data = get(f"/api/roms/{rom_id}/content/{urllib.parse.quote(name)}")
        os.makedirs(out, exist_ok=True)
        if name.lower().endswith(".zip"):
            with zipfile.ZipFile(io.BytesIO(data)) as z:
                member = max(z.infolist(), key=lambda i: i.file_size)
                rom = z.read(member)
                rom_name = os.path.basename(member.filename)
        else:
            rom, rom_name = data, name
        ext = os.path.splitext(rom_name)[1].lower() or ".bin"
        path = os.path.join(out, "game" + ext)
        with open(path, "wb") as f:
            f.write(rom)
        with open(os.path.join(out, "name.txt"), "w", encoding="utf-8") as f:
            f.write(meta.get("name", name) + "\n")
        print(rom_id, meta.get("name"), len(rom), hashlib.sha1(rom).hexdigest())


main()
