#!/usr/bin/env python3
"""Drive the real GTK window and check rendered workflow states with OCR."""

import os
import subprocess
import sys
import time
from pathlib import Path


def run(*command: str) -> str:
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout


def screenshot_text(window: str) -> str:
    image = Path(os.environ["KULKAN_SMOKE_ROOT"]) / "screen.png"
    subprocess.run(
        ["import", "-window", window, str(image)],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return run("tesseract", str(image), "stdout", "--psm", "6")


def wait_for_text(window: str, expected: str, timeout: float = 12.0) -> str:
    deadline = time.monotonic() + timeout
    last_text = ""
    while time.monotonic() < deadline:
        last_text = screenshot_text(window)
        if expected in last_text:
            return last_text
        time.sleep(0.2)
    raise AssertionError(f"did not see {expected!r} in the Library window; saw:\n{last_text}")


def click(window: str, x: int, y: int) -> None:
    run("xdotool", "mousemove", "--window", window, str(x), str(y), "click", "1")


def main() -> None:
    deadline = time.monotonic() + 20.0
    window = None
    while time.monotonic() < deadline:
        search = subprocess.run(
            ["xdotool", "search", "--onlyvisible", "--name", "Kulkán"],
            capture_output=True,
            text=True,
            check=False,
        )
        matches = search.stdout.splitlines()
        if matches:
            window = matches[0]
            break
        time.sleep(0.1)
    if window is None:
        raise AssertionError("Kulkán window did not appear")

    wait_for_text(window, "Songs")
    wait_for_text(window, "Queue")

    click(window, 120, 78)  # Open Search
    wait_for_text(window, "Search title")
    click(window, 400, 104)  # Basic Search entry
    run("xdotool", "key", "ctrl+a")
    run("xdotool", "type", "gtk-smoke-query")
    click(window, 744, 104)  # Submit Search
    wait_for_text(window, "Search results")
    wait_for_text(window, "gtk-smoke-query")

    click(window, 120, 118)  # Artists
    wait_for_text(window, "Artists")
    wait_for_text(window, "No Artists are in the scanned Library")

    click(window, 120, 78)  # Restore the cached Search results
    wait_for_text(window, "Search results")
    wait_for_text(window, "gtk-smoke-query")

    click(window, 120, 78)  # Return from results to criteria
    wait_for_text(window, "Search title")
    if "gtk-smoke-query" not in screenshot_text(window):
        raise AssertionError("Search criteria were not retained for editing")

    click(window, 804, 104)  # Clear Search
    wait_for_text(window, "Search title")
    if "gtk-smoke-query" in screenshot_text(window):
        raise AssertionError("Clear did not blank the Search criteria")

    print("GTK Library workspace smoke test passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"GTK Library workspace smoke test failed: {error}", file=sys.stderr)
        raise
