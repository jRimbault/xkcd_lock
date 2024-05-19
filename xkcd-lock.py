#!/usr/bin/env python3

"""Download random xkcd comic"""

import json
import random
import subprocess
import tempfile
import textwrap
from pathlib import Path
from urllib.request import urlopen, urlretrieve


def main():
    url, title, alt, num = random_comic_url()
    path = download_png(url, title, num)
    alt = "\n".join(textwrap.wrap(alt))
    path = insert_text(path, title, alt, num)
    swaylock(path)


def swaylock(image):
    return subprocess.run(
        [
            "swaylock",
            "--ignore-empty-password",
            "--show-failed-attempts",
            "--daemonize",
            "-i",
            f"DP-1:{image}",
            "-i",
            "eDP-1:/home/jrimbault/Pictures/shape_surface_line-black.jpg",
            "-s",
            "center",
        ],
        check=True,
    )


def insert_text(image, title, alt, num):
    xkcd_dir = Path.home().joinpath("Pictures", "xkcd", "with_text")
    xkcd_dir.mkdir(exist_ok=True)
    tmp = xkcd_dir.joinpath(f"{num} - {safe_path(title)}.png")
    if tmp.exists():
        return tmp
    command = [
        "convert",
        "-size",
        "1920x1080",
        "xc:white",
        image,
        "-gravity",
        "center",
        "-gravity",
        "center",
        "-composite",
        "-gravity",
        "north",
        "-pointsize",
        "36",
        "-annotate",
        "+0+100",
        title,
        "-gravity",
        "south",
        "-pointsize",
        "20",
        "-annotate",
        "+0+100",
        alt,
        tmp,
    ]
    subprocess.run(command, check=True)
    return tmp


def random_comic_url():
    data = get_json("https://xkcd.com/info.0.json")
    comic = random.randint(1, data["num"])
    data = get_json(f"https://xkcd.com/{comic}/info.0.json")
    return data["img"], data["title"], data["alt"], data["num"]


def get_json(url):
    with urlopen(url) as r:
        return json.load(r)


def safe_path(value):
    return "".join(c for c in value if c.isalpha() or c.isdigit() or c == " ").rstrip()


def download_png(url, title, num):
    xkcd_dir = Path.home().joinpath("Pictures", "xkcd")
    xkcd_dir.mkdir(exist_ok=True)
    xkcd = xkcd_dir.joinpath(f"{num} - {safe_path(title)}.png")
    if xkcd.exists():
        return xkcd
    urlretrieve(url, xkcd)
    return xkcd


if __name__ == "__main__":
    main()
