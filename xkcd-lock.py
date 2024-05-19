#!/usr/bin/env python3

"""Download random xkcd comic"""

import json
import random
import subprocess
import tempfile
import textwrap
from urllib.request import urlopen, urlretrieve


def main():
    url, title, alt = random_comic_url()
    path = download_png(url)
    alt = "\n".join(textwrap.wrap(alt))
    path = insert_text(path, title, alt)
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


def insert_text(image, title, alt):
    tmp = tempfile.mktemp(suffix=".png")
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
    return data["img"], data["title"], data["alt"]


def get_json(url):
    with urlopen(url) as r:
        return json.load(r)


def download_png(url):
    tmp = tempfile.mktemp(suffix=".png")
    urlretrieve(url, tmp)
    return tmp


if __name__ == "__main__":
    main()
